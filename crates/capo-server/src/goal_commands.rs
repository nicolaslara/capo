//! GA2 (goal-orchestration GO4/GO5/GO6/GO10): the server-owned goal lifecycle,
//! read, and historical-report surfaces.
//!
//! Every goal MUTATION flows through this server/controller boundary: the CLI
//! and any other client are read-only over goals and never own goal lifecycle or
//! scheduler state. The mutations append an append-only goal event AND project
//! the GA1 goal read models in the SAME transaction, so the read surfaces below
//! are derived read models that rebuild identically from the log.
//!
//! The load-bearing safety property (GO9): a Capo goal-complete transition is
//! reachable ONLY through the GA5 evidence-gated auditor. None of the lifecycle
//! statuses here is `complete`; a direct
//! [`crate::ServerCommand::MarkGoalComplete`] is rejected by construction; and a
//! requirement is never recorded `validated`/`reviewed` on an `agent_reported`
//! source alone -- the read model must never show completion reachable by an
//! agent assertion.

use capo_core::{
    AgentId, CommandIntent, CommandTarget, EvidenceId, GoalId, RequirementId, RunId, SessionId,
    TaskId, TurnId,
};
use capo_state::{
    DelegatedProviderGoalProjection, EventKind, EventRecord, GoalContinuationProjection,
    GoalProjection, GoalReportProjection, NewEvent, ProjectionRecord, RedactionState,
    RequirementLedgerProjection,
};

use crate::util::{command_identity_hash, slug, stable_hash};
use crate::{
    CapoServer, DelegatedProviderGoalView, GoalContinuationView, GoalReportFormat,
    GoalReportListing, GoalReportRecord, GoalReportRendering, GoalReportView, GoalRequirementView,
    GoalSpec, GoalStatusSummary, GoalTimelineEntry, GoalTimelineView, GoalView,
    RequirementStatusRecord, ServerClientOrigin, ServerError, ServerResponse,
    ServerResponsePayload, ServerResult,
};

/// The lifecycle statuses the goal surface owns. `complete` is intentionally
/// absent: completion is the GA5 auditor's verdict, not a lifecycle write.
const LIFECYCLE_STATUSES: &[&str] = &[
    GoalProjection::ACTIVE,
    GoalProjection::PAUSED,
    GoalProjection::BLOCKED,
    GoalProjection::CLEARED,
];

/// Classify a report/evidence/requirement `source` tag. Delegates the
/// observed-vs-claim decision to the canonical
/// [`capo_tools::source_is_observed_evidence`] (the single source of truth the
/// state and tools crates also share) and only adds the new error-on-unknown
/// behavior on top: returns `Ok(true)` for observed evidence, `Ok(false)` for an
/// agent claim, and an error for an unclassifiable source so a malformed tag
/// never silently lands in the read model.
fn source_is_observed(source: &str) -> ServerResult<bool> {
    if capo_tools::source_is_observed_evidence(source) {
        Ok(true)
    } else if source == capo_tools::EVIDENCE_SOURCE_AGENT_REPORTED {
        Ok(false)
    } else {
        Err(ServerError::UnclassifiableReportSource {
            source: source.to_string(),
        })
    }
}

impl CapoServer {
    pub(crate) fn handle_goal_command_set(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        spec: GoalSpec,
    ) -> ServerResult<ServerResponse> {
        self.reject_complete_status(&spec.goal_id, GoalProjection::ACTIVE)?;
        let goal = GoalProjection {
            goal_id: GoalId::new(spec.goal_id.clone()),
            project_id: self.project_id.clone(),
            task_id: spec.task_id.clone().map(TaskId::new),
            agent_id: spec.agent_id.clone().map(AgentId::new),
            session_id: spec.session_id.clone().map(SessionId::new),
            parent_goal_id: spec.parent_goal_id.clone().map(GoalId::new),
            attempt_run_id: spec.attempt_run_id.clone().map(RunId::new),
            objective: spec.objective.clone(),
            // A `SetGoal` re-issue preserves a paused/blocked/cleared status only
            // by an explicit lifecycle command; the create/set path lands the goal
            // `active` so it is eligible for continuation.
            status: existing_status_or_active(self, &spec.goal_id)?,
            success_criteria_json: spec.success_criteria_json.clone(),
            constraints_json: spec.constraints_json.clone(),
            verification_surface_json: spec.verification_surface_json.clone(),
            budget_json: spec.budget_json.clone(),
            stop_conditions_json: spec.stop_conditions_json.clone(),
            blocker_reason: String::new(),
            updated_sequence: 0,
        };
        let mut records = vec![ProjectionRecord::Goal(goal)];
        for requirement in &spec.requirements {
            // Seed each requirement at `unverified` unless it already exists (the
            // ledger is last-write-wins; a re-issued spec never regresses an
            // auditor-advanced status because we only insert when absent).
            if self
                .state()
                .requirement_ledgers_for_goal(&GoalId::new(spec.goal_id.clone()))
                .map_err(ServerError::State)?
                .iter()
                .all(|ledger| ledger.requirement_id.as_str() != requirement.requirement_id)
            {
                records.push(ProjectionRecord::RequirementLedger(
                    RequirementLedgerProjection {
                        requirement_id: RequirementId::new(requirement.requirement_id.clone()),
                        goal_id: GoalId::new(spec.goal_id.clone()),
                        project_id: self.project_id.clone(),
                        summary: requirement.summary.clone(),
                        status: RequirementLedgerProjection::UNVERIFIED.to_string(),
                        last_status_source: "controller".to_string(),
                        updated_sequence: 0,
                    },
                ));
            }
        }
        // A `SetGoal` re-issue is a genuine mutation when its content differs (a
        // changed objective, structured field, or new requirement), so the
        // discriminator hashes the full spec content: an unchanged re-issue stays
        // idempotent, a changed one appends and re-projects in place.
        let discriminator = stable_hash(spec_content_fingerprint(&spec).as_bytes());
        self.append_goal_event(
            &origin,
            EventKind::GoalCreated,
            &spec.goal_id,
            spec.session_id.as_deref(),
            spec.agent_id.as_deref(),
            "goal.created",
            &discriminator,
            &records,
        )?;
        self.goal_view_response(request_id, origin, &spec.goal_id)
    }

    /// AI5 (architecture-improvements): close the autonomy loop. Evaluate the GA4
    /// continuation scheduler for the goal, durably record the decision, and -- ONLY
    /// on a `Continue` decision (which the scheduler reaches only when continuation
    /// is explicitly enabled and every safe-boundary precondition holds) -- drive
    /// exactly ONE follow-on turn through the SINGLE production orchestration path
    /// ([`crate::CapoServer::run_dispatch_turn`], the same path AI1's
    /// `RunDispatchTurn` uses). Every non-continuing decision records only and
    /// drives no turn; a `BudgetLimit` additionally aborts the goal's attempt run
    /// durably (the recording path's RTL7 `run.aborted`).
    ///
    /// The off-by-default invariant is preserved at this boundary: with
    /// `conditions.enabled = false` the scheduler short-circuits to `pause` and no
    /// turn is ever dispatched.
    pub(crate) fn handle_continue_goal(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
        continuation_id: String,
        conditions: crate::ContinueGoalConditions,
        turn: crate::ContinueGoalTurn,
    ) -> ServerResult<ServerResponse> {
        use capo_controller::{
            ContinuationConditions, ContinuationDecision, GoalBudget, RunResourceCeiling,
            RunResourceUsage,
        };

        // The goal must exist; the scheduler reads its lifecycle status (active/
        // blocked) from the persisted projection.
        let goal = self.require_goal(&goal_id)?;

        let budget = GoalBudget {
            ceiling: RunResourceCeiling::for_live_provider(
                conditions.budget_max_turns,
                std::time::Duration::from_secs(conditions.budget_timeout_seconds),
                conditions.budget_max_token_cost,
            ),
            usage: RunResourceUsage {
                turns_taken: conditions.budget_turns_taken,
                wall_clock_elapsed: std::time::Duration::ZERO,
                token_cost: conditions.budget_token_cost,
            },
        };
        let scheduler_conditions = ContinuationConditions {
            enabled: conditions.enabled,
            runtime_idle: conditions.runtime_idle,
            session_idle: conditions.session_idle,
            user_input_queued: conditions.user_input_queued,
            permission_pending: conditions.permission_pending,
            capability_profile_valid: conditions.capability_profile_valid,
            next_step_writes_source: conditions.next_step_writes_source,
            checkpoint_boundary_available: conditions.checkpoint_boundary_available,
            verification_runner_available: conditions.verification_runner_available,
            last_continuation_made_no_progress: conditions.last_continuation_made_no_progress,
            strategy_changed_since_suppression: conditions.strategy_changed_since_suppression,
            budget,
        };

        // The abort refs pair a terminal `budget-limit` decision with the RTL7
        // `run.aborted` event. Built from the goal's bound attempt run + session;
        // `abort_run_for_ceiling` resolves the rest from persisted state, so the
        // runtime/external refs are not load-bearing for the abort itself.
        let abort_refs = match (
            goal.attempt_run_id.clone(),
            goal.session_id.clone(),
            goal.task_id.clone(),
            goal.agent_id.clone(),
        ) {
            (Some(run_id), Some(session_id), Some(task_id), Some(agent_id)) => Some((
                capo_controller::FakeRunRefs {
                    task_id,
                    agent_id,
                    session_id,
                    run_id,
                    runtime_process_ref: String::new(),
                    external_session_ref: String::new(),
                },
                TurnId::new(turn.turn_id.clone()),
            )),
            _ => None,
        };

        // Evaluate AND durably record the decision (event + GoalContinuationProjection),
        // aborting the attempt run on a budget breach -- the single GA4 recording path.
        let outcome = self
            .controller
            .evaluate_and_record_continuation(
                &GoalId::new(goal_id.clone()),
                &continuation_id,
                &scheduler_conditions,
                None,
                abort_refs.as_ref().map(|(refs, turn_id)| (refs, turn_id)),
            )
            .map_err(ServerError::State)?;

        // ONLY a `Continue` decision drives the next turn, and it re-enters the
        // SINGLE production path -- never a parallel driver. Every other decision
        // records only. Because the scheduler returns `Continue` only when
        // `enabled` is set, the off-by-default invariant holds here by construction.
        let dispatched = if outcome.decision == ContinuationDecision::Continue {
            Some(self.drive_continued_turn(turn)?)
        } else {
            None
        };

        self.response(
            request_id,
            origin,
            ServerResponsePayload::ContinuationEvaluated(crate::ContinuationEvaluatedSummary {
                goal_id,
                continuation_id,
                decision: outcome.decision.as_str().to_string(),
                reason: outcome.reason.to_string(),
                dispatched,
            }),
        )
    }

    /// AI5: drive ONE continued turn through the SINGLE production orchestration
    /// path. This is a thin builder over [`crate::CapoServer::run_dispatch_turn`]
    /// (AI1) -- the exact same loop an operator turn enters -- so a continued turn
    /// is indistinguishable from an operator turn in its event sequence +
    /// `TurnFinished`. It is reached ONLY from a recorded `Continue` decision.
    fn drive_continued_turn(
        &self,
        turn: crate::ContinueGoalTurn,
    ) -> ServerResult<crate::DispatchTurnSummary> {
        use crate::{DispatchTurnMode, DispatchTurnRequest, LiveProviderTurn, TurnFinishedSummary};

        // A live turn must run inside a wall-clock-bounded ceiling (the loop rejects
        // an unbounded one); a zero timeout cannot satisfy that.
        if turn.timeout_seconds == 0 {
            return Err(ServerError::AdapterFixture(
                "ContinueGoal requires a non-zero wall-clock timeout for the continued turn (the \
                 live turn runs inside a wall-clock-bounded resource ceiling)"
                    .to_string(),
            ));
        }
        let ceiling = capo_controller::RunResourceCeiling::for_live_provider(
            turn.max_turns,
            std::time::Duration::from_secs(turn.timeout_seconds),
            turn.max_token_cost,
        );
        let usage_before = capo_controller::RunResourceUsage {
            turns_taken: turn.turns_taken_before,
            wall_clock_elapsed: std::time::Duration::ZERO,
            token_cost: turn.token_cost_before,
        };
        let outcome = self.run_dispatch_turn(DispatchTurnRequest {
            agent_name: turn.agent_name,
            adapter: turn.adapter,
            goal: turn.goal,
            workspace: turn.workspace,
            artifacts: turn.artifacts,
            session_id: turn.session_id,
            run_id: turn.run_id,
            turn_id: turn.turn_id,
            mode: DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
                capability_profile: turn.capability_profile,
                runtime_scope: turn.runtime_scope,
                credential_scan_policy: turn.credential_scan_policy,
                raw_prompt_policy: turn.raw_prompt_policy,
                raw_output_policy: turn.raw_output_policy,
                tool_wrapper_policy: turn.tool_wrapper_policy,
                live_provider_opt_in: turn.live_provider_opt_in,
                live_execution_opt_in: turn.live_execution_opt_in,
                mock_runtime_opt_in: turn.mock_runtime_opt_in,
                mock_provider_output_name: turn.mock_provider_output_name,
                mock_provider_output_jsonl: turn.mock_provider_output_jsonl,
                ceiling,
                usage_before,
                turn_token_cost: turn.turn_token_cost,
                codex_program_override: None,
                unattended: turn.unattended,
            })),
        })?;
        Ok(crate::DispatchTurnSummary {
            run: outcome.run,
            finished: TurnFinishedSummary::from_finished(&outcome.finished),
            ceiling_breach_code: outcome
                .ceiling_breach
                .map(|breach| breach.code().to_string()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_goal_lifecycle(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
        status: &'static str,
        blocker_reason: &str,
        event_kind: EventKind,
        event_label: &str,
    ) -> ServerResult<ServerResponse> {
        let mut goal = self.require_goal(&goal_id)?;
        debug_assert!(LIFECYCLE_STATUSES.contains(&status));
        goal.status = status.to_string();
        goal.blocker_reason = blocker_reason.to_string();
        goal.updated_sequence = 0;
        // Capture the event-envelope refs before moving `goal` into the projection
        // record (the same call both borrows these and consumes `goal`).
        let session_ref = goal.session_id.as_ref().map(|id| id.as_str().to_string());
        let agent_ref = goal.agent_id.as_ref().map(|id| id.as_str().to_string());
        // A re-block / re-pause with a NEW reason after an intervening transition
        // is a genuine mutation, so the discriminator carries the target status and
        // reason; an identical repeat stays idempotent.
        let discriminator = format!("{status}:{blocker_reason}");
        self.append_goal_event(
            &origin,
            event_kind,
            &goal_id,
            session_ref.as_deref(),
            agent_ref.as_deref(),
            event_label,
            &discriminator,
            &[ProjectionRecord::Goal(goal)],
        )?;
        self.goal_view_response(request_id, origin, &goal_id)
    }

    pub(crate) fn handle_set_requirement_status(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        record: RequirementStatusRecord,
    ) -> ServerResult<ServerResponse> {
        self.require_goal(&record.goal_id)?;
        let observed = source_is_observed(&record.source)?;
        // The read model must never show a requirement validated/reviewed by an
        // agent claim alone -- that strength is the auditor's, on observed
        // evidence (GO9). Reject the regression here, at the boundary.
        let claim_only = !observed;
        if claim_only
            && matches!(
                record.status.as_str(),
                RequirementLedgerProjection::VALIDATED | RequirementLedgerProjection::REVIEWED
            )
        {
            return Err(ServerError::IllegalGoalStatusTransition {
                goal_id: record.goal_id.clone(),
                requested_status: format!("{}:agent_reported", record.status),
            });
        }
        if !is_known_requirement_status(&record.status) {
            return Err(ServerError::IllegalGoalStatusTransition {
                goal_id: record.goal_id.clone(),
                requested_status: record.status.clone(),
            });
        }
        let ledger = RequirementLedgerProjection {
            requirement_id: RequirementId::new(record.requirement_id.clone()),
            goal_id: GoalId::new(record.goal_id.clone()),
            project_id: self.project_id.clone(),
            summary: record.summary.clone(),
            status: record.status.clone(),
            last_status_source: record.source.clone(),
            updated_sequence: 0,
        };
        // Each requirement advance (unverified->supported->validated->reviewed) is
        // a distinct ledger mutation, so the discriminator carries the requested
        // status and source: the central GO3/GO9 advance is no longer collapsed
        // into a single first-write, while a verbatim repeat stays idempotent.
        let discriminator = format!("{}:{}", record.status, record.source);
        self.append_goal_event(
            &origin,
            EventKind::RequirementStatusChanged,
            &record.requirement_id,
            None,
            None,
            "goal.requirement_status_changed",
            &discriminator,
            &[ProjectionRecord::RequirementLedger(ledger)],
        )?;
        self.goal_view_response(request_id, origin, &record.goal_id)
    }

    pub(crate) fn handle_record_goal_report(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        report: GoalReportRecord,
    ) -> ServerResult<ServerResponse> {
        self.require_goal(&report.goal_id)?;
        // Classify the source (and reject an unclassifiable tag). Observed
        // evidence carries no agent confidence; an agent claim keeps its declared
        // confidence so the auditor can weigh it.
        let observed = source_is_observed(&report.source)?;
        let confidence = if observed { None } else { report.confidence };
        let projection = GoalReportProjection {
            goal_report_id: report.goal_report_id.clone(),
            goal_id: GoalId::new(report.goal_id.clone()),
            project_id: self.project_id.clone(),
            session_id: report.session_id.clone().map(SessionId::new),
            requirement_id: report.requirement_id.clone().map(RequirementId::new),
            report_kind: report.report_kind.clone(),
            source: report.source.clone(),
            confidence,
            summary: report.summary.clone(),
            body_artifact_id: report.body_artifact_id.clone(),
            evidence_id: report.evidence_id.clone().map(EvidenceId::new),
            updated_sequence: 0,
        };
        // Re-recording the same report id with changed content (a corrected
        // summary, a now-cited evidence id) is a genuine upsert, so the
        // discriminator hashes the report content; an identical re-record stays
        // idempotent.
        let discriminator = stable_hash(report_content_fingerprint(&report).as_bytes());
        self.append_goal_event(
            &origin,
            EventKind::GoalReportRecorded,
            &report.goal_report_id,
            report.session_id.as_deref(),
            None,
            "goal.report_recorded",
            &discriminator,
            &[ProjectionRecord::GoalReport(projection)],
        )?;
        self.goal_view_response(request_id, origin, &report.goal_id)
    }

    /// GA2 (goal-orchestration GO9): the direct "mark complete" request is
    /// rejected by construction. Completion is the auditor's verdict.
    pub(crate) fn reject_mark_goal_complete(&self, goal_id: &str) -> ServerResult<ServerResponse> {
        Err(ServerError::GoalCompleteNotALifecycleCommand {
            goal_id: goal_id.to_string(),
        })
    }

    pub(crate) fn handle_list_goals(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
    ) -> ServerResult<ServerResponse> {
        let goals = self
            .state()
            .goals_for_project(&self.project_id)
            .map_err(ServerError::State)?;
        let mut summaries = Vec::with_capacity(goals.len());
        for goal in &goals {
            summaries.push(self.goal_status_summary(goal)?);
        }
        self.response(request_id, origin, ServerResponsePayload::Goals(summaries))
    }

    pub(crate) fn handle_view_goal(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
    ) -> ServerResult<ServerResponse> {
        self.goal_view_response(request_id, origin, &goal_id)
    }

    pub(crate) fn handle_goal_report_listing(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
        surface: GoalReportSurface,
    ) -> ServerResult<ServerResponse> {
        let goal = self.require_goal(&goal_id)?;
        let reports = self
            .state()
            .goal_reports_for_goal(&GoalId::new(goal_id.clone()))
            .map_err(ServerError::State)?;
        let filtered: Vec<GoalReportView> = reports
            .iter()
            .filter(|report| surface.matches(report))
            .map(goal_report_view)
            .collect();
        let listing = GoalReportListing {
            goal_id,
            surface: surface.label().to_string(),
            blocker_reason: goal.blocker_reason.clone(),
            reports: filtered,
        };
        self.response(
            request_id,
            origin,
            ServerResponsePayload::GoalReports(listing),
        )
    }

    pub(crate) fn handle_goal_timeline(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
    ) -> ServerResult<ServerResponse> {
        let goal = self.require_goal(&goal_id)?;
        let entries = self
            .goal_timeline_entries(&goal)?
            .into_iter()
            .map(|record| GoalTimelineEntry {
                sequence: record.sequence,
                event_id: record.event_id,
                kind: record.kind,
                actor: record.actor,
                redaction_state: record.redaction_state,
            })
            .collect();
        self.response(
            request_id,
            origin,
            ServerResponsePayload::GoalTimeline(GoalTimelineView { goal_id, entries }),
        )
    }

    pub(crate) fn handle_goal_report_rendering(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: String,
        format: GoalReportFormat,
    ) -> ServerResult<ServerResponse> {
        let goal = self.require_goal(&goal_id)?;
        let id = GoalId::new(goal_id.clone());
        // Gather the persisted projections + timeline and render through the SHARED
        // GO10 renderer in `capo-state` -- the SAME code the deterministic goal
        // e2e snapshots, so the operator surface and the e2e cannot drift into two
        // contradictory definitions of "the historical report".
        let requirements = self
            .state()
            .requirement_ledgers_for_goal(&id)
            .map_err(ServerError::State)?;
        let reports = self
            .state()
            .goal_reports_for_goal(&id)
            .map_err(ServerError::State)?;
        let continuations = self
            .state()
            .goal_continuations_for_goal(&id)
            .map_err(ServerError::State)?;
        let delegated_provider_goals = self
            .state()
            .delegated_provider_goals_for_goal(&id)
            .map_err(ServerError::State)?;
        let evidence = match goal.task_id.as_ref() {
            Some(task_id) => self
                .state()
                .evidence_for_task(task_id)
                .map_err(ServerError::State)?,
            None => Vec::new(),
        };
        let audit = self
            .state()
            .latest_goal_audit_decision(&id)
            .map_err(ServerError::State)?;
        let timeline = self.goal_timeline_entries(&goal)?;
        let inputs = capo_state::GoalReportInputs {
            goal: &goal,
            requirements: &requirements,
            reports: &reports,
            continuations: &continuations,
            delegated_provider_goals: &delegated_provider_goals,
            evidence: &evidence,
            audit: audit.as_ref(),
            timeline: &timeline,
        };
        let rendered = match format {
            GoalReportFormat::Markdown => capo_state::render_goal_report_markdown(&inputs),
            GoalReportFormat::Json => capo_state::render_goal_report_json(&inputs),
        };
        let rendering = GoalReportRendering {
            goal_id,
            format: format.as_str().to_string(),
            body: rendered.body,
            degraded: rendered.degraded,
        };
        self.response(
            request_id,
            origin,
            ServerResponsePayload::GoalReport(rendering),
        )
    }

    // --- helpers -----------------------------------------------------------

    fn goal_view_response(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        goal_id: &str,
    ) -> ServerResult<ServerResponse> {
        let view = self.assemble_goal_view(goal_id)?;
        self.response(
            request_id,
            origin,
            ServerResponsePayload::GoalView(Box::new(view)),
        )
    }

    fn assemble_goal_view(&self, goal_id: &str) -> ServerResult<GoalView> {
        let goal = self.require_goal(goal_id)?;
        let summary = self.goal_status_summary(&goal)?;
        let id = GoalId::new(goal_id.to_string());
        let requirements = self
            .state()
            .requirement_ledgers_for_goal(&id)
            .map_err(ServerError::State)?
            .iter()
            .map(|ledger| GoalRequirementView {
                requirement_id: ledger.requirement_id.to_string(),
                summary: ledger.summary.clone(),
                status: ledger.status.clone(),
                last_status_source: ledger.last_status_source.clone(),
                observed: source_is_observed(&ledger.last_status_source).unwrap_or(false),
            })
            .collect();
        let reports = self
            .state()
            .goal_reports_for_goal(&id)
            .map_err(ServerError::State)?
            .iter()
            .map(goal_report_view)
            .collect();
        let continuations = self
            .state()
            .goal_continuations_for_goal(&id)
            .map_err(ServerError::State)?
            .iter()
            .map(goal_continuation_view)
            .collect();
        let delegated_provider_goals = self
            .state()
            .delegated_provider_goals_for_goal(&id)
            .map_err(ServerError::State)?
            .iter()
            .map(delegated_provider_goal_view)
            .collect();
        Ok(GoalView {
            success_criteria_json: goal.success_criteria_json.clone(),
            constraints_json: goal.constraints_json.clone(),
            verification_surface_json: goal.verification_surface_json.clone(),
            budget_json: goal.budget_json.clone(),
            stop_conditions_json: goal.stop_conditions_json.clone(),
            task_id: goal.task_id.as_ref().map(ToString::to_string),
            agent_id: goal.agent_id.as_ref().map(ToString::to_string),
            session_id: goal.session_id.as_ref().map(ToString::to_string),
            summary,
            requirements,
            reports,
            continuations,
            delegated_provider_goals,
        })
    }

    fn goal_status_summary(&self, goal: &GoalProjection) -> ServerResult<GoalStatusSummary> {
        let requirements = self
            .state()
            .requirement_ledgers_for_goal(&goal.goal_id)
            .map_err(ServerError::State)?;
        let reports = self
            .state()
            .goal_reports_for_goal(&goal.goal_id)
            .map_err(ServerError::State)?;
        let requirements_supported = requirements
            .iter()
            .filter(|ledger| {
                matches!(
                    ledger.status.as_str(),
                    RequirementLedgerProjection::SUPPORTED
                        | RequirementLedgerProjection::VALIDATED
                        | RequirementLedgerProjection::REVIEWED
                )
            })
            .count();
        let blocked_requirement_count = requirements
            .iter()
            .filter(|ledger| ledger.status == RequirementLedgerProjection::BLOCKED)
            .count();
        let contradicted_requirement_count = requirements
            .iter()
            .filter(|ledger| ledger.status == RequirementLedgerProjection::CONTRADICTED)
            .count();
        Ok(GoalStatusSummary {
            goal_id: goal.goal_id.to_string(),
            objective: goal.objective.clone(),
            status: goal.status.clone(),
            parent_goal_id: goal.parent_goal_id.as_ref().map(ToString::to_string),
            attempt_run_id: goal.attempt_run_id.as_ref().map(ToString::to_string),
            requirement_count: requirements.len(),
            requirements_supported,
            blocked_requirement_count,
            contradicted_requirement_count,
            report_count: reports.len(),
            blocker_reason: goal.blocker_reason.clone(),
            updated_sequence: goal.updated_sequence,
        })
    }

    /// The goal's event timeline (GO5/GO10): the goal's own events (keyed by the
    /// goal id as `item_id`) plus its attempt run's events, in sequence order.
    ///
    /// This is an item-SCOPED read, not a bounded prefix scan of the whole project
    /// log: the goal/requirement/report/continuation events are fetched directly by
    /// their `item_id` via [`capo_state::SqliteStateStore::events_for_items`], so a
    /// goal event with any sequence is returned regardless of how large the global
    /// log has grown. The result is deduped by sequence (the run-scoped evidence
    /// events may overlap an item-keyed event) and ordered by sequence, so the
    /// timeline rebuilds identically from the log.
    fn goal_timeline_entries(&self, goal: &GoalProjection) -> ServerResult<Vec<EventRecord>> {
        let mut records: Vec<EventRecord> = Vec::new();
        if let Some(run_id) = goal.attempt_run_id.as_ref() {
            records = self
                .state()
                .evidence_events_for_run(run_id)
                .map_err(ServerError::State)?;
        }
        let mut item_ids: Vec<String> = vec![goal.goal_id.to_string()];
        for ledger in self
            .state()
            .requirement_ledgers_for_goal(&goal.goal_id)
            .map_err(ServerError::State)?
        {
            item_ids.push(ledger.requirement_id.to_string());
        }
        for report in self
            .state()
            .goal_reports_for_goal(&goal.goal_id)
            .map_err(ServerError::State)?
        {
            item_ids.push(report.goal_report_id);
        }
        for continuation in self
            .state()
            .goal_continuations_for_goal(&goal.goal_id)
            .map_err(ServerError::State)?
        {
            item_ids.push(continuation.continuation_id);
        }
        let scoped = self
            .state()
            .events_for_items(&item_ids)
            .map_err(ServerError::State)?;
        for record in scoped {
            if !records.iter().any(|seen| seen.sequence == record.sequence) {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.sequence);
        Ok(records)
    }

    fn require_goal(&self, goal_id: &str) -> ServerResult<GoalProjection> {
        self.state()
            .goal(&GoalId::new(goal_id.to_string()))
            .map_err(ServerError::State)?
            .ok_or_else(|| ServerError::UnknownGoal {
                goal_id: goal_id.to_string(),
            })
    }

    /// Reject a lifecycle write that would land a goal on a `complete` (or other
    /// non-lifecycle) status. Completion is the auditor's verdict (GO9).
    fn reject_complete_status(&self, goal_id: &str, status: &str) -> ServerResult<()> {
        if status == "complete" {
            return Err(ServerError::GoalCompleteNotALifecycleCommand {
                goal_id: goal_id.to_string(),
            });
        }
        if !LIFECYCLE_STATUSES.contains(&status) {
            return Err(ServerError::IllegalGoalStatusTransition {
                goal_id: goal_id.to_string(),
                requested_status: status.to_string(),
            });
        }
        Ok(())
    }

    /// Append a goal event AND project its read models in one transaction,
    /// recording the server-request envelope alongside. Mirrors the existing
    /// dispatch append pattern so goal mutations sit on the same single-writer
    /// serialization point as every other write.
    ///
    /// `discriminator` makes the idempotency key unique per LOGICAL operation, not
    /// per `(kind, entity)`. The store's `append_event` short-circuits on a
    /// repeated `(project_id, idempotency_key)` WITHOUT re-applying projections,
    /// so a key of only `{event_label}:{item_id}` would collapse every later
    /// same-kind transition on one entity (a second requirement-status advance, a
    /// `SetGoal` re-issue, a re-block with a new reason) into a silent no-op. We
    /// fold the intended new state into the key (status+source for a requirement,
    /// the lifecycle status+reason for a goal, the spec/report content hash for a
    /// create/record) so a genuine mutation appends a new event and re-applies its
    /// projection, while a verbatim retry stays idempotent. Mirrors the
    /// `dispatch.rs` pattern where the key embeds the occurrence-unique plan id.
    #[allow(clippy::too_many_arguments)]
    fn append_goal_event(
        &self,
        origin: &ServerClientOrigin,
        kind: EventKind,
        item_id: &str,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        event_label: &str,
        discriminator: &str,
        records: &[ProjectionRecord],
    ) -> ServerResult<()> {
        let occurrence = stable_hash(format!("{item_id}:{discriminator}").as_bytes());
        let event_id = format!("event-{}-{}", slug(event_label), occurrence);
        let mut event = NewEvent::new(event_id, kind, &origin.actor_id);
        event.project_id = Some(self.project_id.clone());
        event.agent_id = agent_id.map(|id| AgentId::new(id.to_string()));
        event.session_id = session_id.map(|id| SessionId::new(id.to_string()));
        event.item_id = Some(item_id.to_string());
        event.payload_json = serde_json::json!({
            "kind": event_label,
            "item_id": item_id,
        })
        .to_string();
        event.idempotency_key = Some(format!("{event_label}:{item_id}:{discriminator}"));
        event.redaction_state = RedactionState::Safe;
        self.state()
            .append_event(event, records)
            .map_err(ServerError::State)?;
        // Record the server-request envelope so the goal mutation is auditable as
        // a server-boundary action like every other command.
        let command_hash =
            command_identity_hash(format!("{event_label}:{item_id}:{discriminator}"));
        let command = self.command_envelope(
            &format!("goal-{}-{}", slug(event_label), occurrence),
            origin,
            &command_hash,
            CommandTarget::Project(self.project_id.clone()),
            CommandIntent::SendTask,
            None,
        );
        self.record_server_request_handled(&command, origin, event_label, None, None)
            .map_err(ServerError::State)?;
        Ok(())
    }

    /// Test-only and crate-internal access to the underlying store for the goal
    /// read surfaces. The goal mutations and reads all go through this so they sit
    /// on the same store the rest of the server uses.
    fn state(&self) -> &capo_state::SqliteStateStore {
        self.controller.state()
    }
}

/// A stable, order-fixed fingerprint of a [`GoalSpec`]'s content. Two specs with
/// the same content produce the same fingerprint (so an unchanged `SetGoal`
/// re-issue is idempotent); any content change (objective, a structured field, a
/// requirement) changes it (so a genuine re-issue appends and re-projects).
fn spec_content_fingerprint(spec: &GoalSpec) -> String {
    let mut requirements: Vec<String> = spec
        .requirements
        .iter()
        .map(|requirement| format!("{}={}", requirement.requirement_id, requirement.summary))
        .collect();
    requirements.sort();
    format!(
        "obj={}|task={:?}|agent={:?}|session={:?}|parent={:?}|run={:?}|success={}|constraints={}|verification={}|budget={}|stop={}|reqs={}",
        spec.objective,
        spec.task_id,
        spec.agent_id,
        spec.session_id,
        spec.parent_goal_id,
        spec.attempt_run_id,
        spec.success_criteria_json,
        spec.constraints_json,
        spec.verification_surface_json,
        spec.budget_json,
        spec.stop_conditions_json,
        requirements.join(","),
    )
}

/// A stable fingerprint of a [`GoalReportRecord`]'s content, for the same
/// idempotency reasoning as [`spec_content_fingerprint`].
fn report_content_fingerprint(report: &GoalReportRecord) -> String {
    format!(
        "goal={}|session={:?}|requirement={:?}|kind={}|source={}|confidence={:?}|summary={}|artifact={:?}|evidence={:?}",
        report.goal_id,
        report.session_id,
        report.requirement_id,
        report.report_kind,
        report.source,
        report.confidence,
        report.summary,
        report.body_artifact_id,
        report.evidence_id,
    )
}

/// Whether `status` is a recognized requirement-ledger status (GO9 states).
fn is_known_requirement_status(status: &str) -> bool {
    matches!(
        status,
        RequirementLedgerProjection::UNVERIFIED
            | RequirementLedgerProjection::SUPPORTED
            | RequirementLedgerProjection::VALIDATED
            | RequirementLedgerProjection::REVIEWED
            | RequirementLedgerProjection::BLOCKED
            | RequirementLedgerProjection::CONTRADICTED
    )
}

/// On a `SetGoal` re-issue, keep the existing lifecycle status (so a paused goal
/// stays paused across a metadata update); a brand-new goal lands `active`.
fn existing_status_or_active(server: &CapoServer, goal_id: &str) -> ServerResult<String> {
    Ok(server
        .controller
        .state()
        .goal(&GoalId::new(goal_id.to_string()))
        .map_err(ServerError::State)?
        .map(|goal| goal.status)
        .unwrap_or_else(|| GoalProjection::ACTIVE.to_string()))
}

fn goal_report_view(report: &GoalReportProjection) -> GoalReportView {
    GoalReportView {
        goal_report_id: report.goal_report_id.clone(),
        requirement_id: report.requirement_id.as_ref().map(ToString::to_string),
        report_kind: report.report_kind.clone(),
        source: report.source.clone(),
        observed: report.is_observed_evidence(),
        confidence: report.confidence,
        summary: report.summary.clone(),
        body_artifact_id: report.body_artifact_id.clone(),
        evidence_id: report.evidence_id.as_ref().map(ToString::to_string),
    }
}

fn goal_continuation_view(continuation: &GoalContinuationProjection) -> GoalContinuationView {
    GoalContinuationView {
        continuation_id: continuation.continuation_id.clone(),
        decision: continuation.decision.clone(),
        reason: continuation.reason.clone(),
        attempt_run_id: continuation
            .attempt_run_id
            .as_ref()
            .map(ToString::to_string),
    }
}

fn delegated_provider_goal_view(
    delegated: &DelegatedProviderGoalProjection,
) -> DelegatedProviderGoalView {
    DelegatedProviderGoalView {
        delegated_goal_id: delegated.delegated_goal_id.clone(),
        provider_kind: delegated.provider_kind.clone(),
        provider_goal_ref: delegated.provider_goal_ref.clone(),
        provider_state: delegated.provider_state.clone(),
        source: delegated.source.clone(),
    }
}

/// GA2 (goal-orchestration GO5): which report-row read surface a listing serves.
/// Each surface is a deterministic filter over the goal-report ledger so the read
/// model is derived, not hand-curated.
#[derive(Clone, Copy, Debug)]
pub(crate) enum GoalReportSurface {
    /// The full story: every report, observed and reported, oldest first.
    Story,
    /// Observed-evidence rows only.
    Evidence,
    /// Validation-kind rows (a `record_validation` report or a `validation`
    /// observed kind).
    Validations,
    /// Review-kind rows.
    Reviews,
    /// The risk surface: raised blockers and contradiction reports.
    Risks,
}

impl GoalReportSurface {
    fn label(&self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Evidence => "evidence",
            Self::Validations => "validations",
            Self::Reviews => "reviews",
            Self::Risks => "risks",
        }
    }

    fn matches(&self, report: &GoalReportProjection) -> bool {
        let kind = report.report_kind.to_ascii_lowercase();
        match self {
            Self::Story => true,
            Self::Evidence => report.is_observed_evidence(),
            Self::Validations => kind.contains("validation") || kind.contains("validate"),
            Self::Reviews => kind.contains("review"),
            Self::Risks => {
                kind.contains("blocker") || kind.contains("risk") || kind.contains("contradict")
            }
        }
    }
}
