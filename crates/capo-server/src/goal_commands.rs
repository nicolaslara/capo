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
    TaskId,
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

/// Classify a report/evidence/requirement `source` tag the same way
/// [`GoalReportProjection::is_observed_evidence`] does, so the server and the
/// projection agree on observed-vs-reported. Returns `Ok(true)` for observed
/// evidence, `Ok(false)` for an agent claim, and an error for an unclassifiable
/// source so a malformed tag never silently lands in the read model.
fn source_is_observed(source: &str) -> ServerResult<bool> {
    if source == "agent_reported" {
        Ok(false)
    } else if source == "runtime_output"
        || source == "adapter_event"
        || source.starts_with("adapter_event:")
    {
        Ok(true)
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
        self.append_goal_event(
            &origin,
            EventKind::GoalCreated,
            &spec.goal_id,
            spec.session_id.as_deref(),
            spec.agent_id.as_deref(),
            "goal.created",
            &records,
        )?;
        self.goal_view_response(request_id, origin, &spec.goal_id)
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
        self.append_goal_event(
            &origin,
            event_kind,
            &goal_id,
            session_ref.as_deref(),
            agent_ref.as_deref(),
            event_label,
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
        self.append_goal_event(
            &origin,
            EventKind::RequirementStatusChanged,
            &record.requirement_id,
            None,
            None,
            "goal.requirement_status_changed",
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
        self.append_goal_event(
            &origin,
            EventKind::GoalReportRecorded,
            &report.goal_report_id,
            report.session_id.as_deref(),
            None,
            "goal.report_recorded",
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
        let view = self.assemble_goal_view(&goal_id)?;
        let timeline = self.goal_timeline_entries(&self.require_goal(&goal_id)?)?;
        let rendering = match format {
            GoalReportFormat::Markdown => render_report_markdown(&view, &timeline),
            GoalReportFormat::Json => render_report_json(&view, &timeline),
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
    /// Reads the event log forward, so it rebuilds identically.
    fn goal_timeline_entries(&self, goal: &GoalProjection) -> ServerResult<Vec<EventRecord>> {
        let mut records: Vec<EventRecord> = Vec::new();
        if let Some(run_id) = goal.attempt_run_id.as_ref() {
            records = self
                .state()
                .evidence_events_for_run(run_id)
                .map_err(ServerError::State)?;
        }
        // The goal lifecycle / report / continuation events are keyed by the
        // domain id as `item_id`, and the run-scoped read above misses goal events
        // with no run. Fold the goal/requirement/report/continuation ids in by a
        // forward scan, deduped by sequence.
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
        let scanned = self
            .state()
            .events_after(0, EVENT_TIMELINE_LIMIT)
            .map_err(ServerError::State)?;
        for record in scanned {
            if record
                .item_id
                .as_deref()
                .is_some_and(|item| item_ids.iter().any(|id| id == item))
                && !records.iter().any(|seen| seen.sequence == record.sequence)
            {
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
    #[allow(clippy::too_many_arguments)]
    fn append_goal_event(
        &self,
        origin: &ServerClientOrigin,
        kind: EventKind,
        item_id: &str,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        event_label: &str,
        records: &[ProjectionRecord],
    ) -> ServerResult<()> {
        let event_id = format!(
            "event-{}-{}",
            slug(event_label),
            stable_hash(item_id.as_bytes())
        );
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
        event.idempotency_key = Some(format!("{event_label}:{item_id}"));
        event.redaction_state = RedactionState::Safe;
        self.state()
            .append_event(event, records)
            .map_err(ServerError::State)?;
        // Record the server-request envelope so the goal mutation is auditable as
        // a server-boundary action like every other command.
        let command_hash = command_identity_hash(format!("{event_label}:{item_id}"));
        let command = self.command_envelope(
            &format!(
                "goal-{}-{}",
                slug(event_label),
                stable_hash(item_id.as_bytes())
            ),
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

const EVENT_TIMELINE_LIMIT: usize = 4096;

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

/// GA2 (goal-orchestration GO10): render the historical execution report as
/// markdown. Rebuildable from the goal view + timeline; degrades clearly when a
/// referenced artifact is absent (it is named as a missing reference, never
/// rendered as raw content -- raw provider transcripts never appear here).
fn render_report_markdown(view: &GoalView, timeline: &[EventRecord]) -> GoalReportRendering {
    let summary = &view.summary;
    let mut degraded = false;
    let mut body = String::new();
    body.push_str(&format!("# Goal report: {}\n\n", summary.goal_id));
    body.push_str(&format!("- Objective: {}\n", summary.objective));
    body.push_str(&format!("- Status: {}\n", summary.status));
    if let Some(parent) = &summary.parent_goal_id {
        body.push_str(&format!("- Parent goal: {parent}\n"));
    }
    body.push_str(&format!(
        "- Requirements: {} ({} supported, {} blocked, {} contradicted)\n",
        summary.requirement_count,
        summary.requirements_supported,
        summary.blocked_requirement_count,
        summary.contradicted_requirement_count,
    ));
    if !summary.blocker_reason.is_empty() {
        body.push_str(&format!("- Current blocker: {}\n", summary.blocker_reason));
    }
    body.push_str("\n## Requirements\n\n");
    if view.requirements.is_empty() {
        body.push_str("_No requirements recorded._\n");
    }
    for requirement in &view.requirements {
        body.push_str(&format!(
            "- [{}] {} ({}) — source: {} ({})\n",
            requirement.status,
            requirement.summary,
            requirement.requirement_id,
            requirement.last_status_source,
            if requirement.observed {
                "observed"
            } else {
                "reported"
            },
        ));
    }
    body.push_str("\n## Story\n\n");
    if view.reports.is_empty() {
        body.push_str("_No reports recorded._\n");
    }
    for report in &view.reports {
        let provenance = if report.observed {
            "observed".to_string()
        } else {
            format!(
                "reported (confidence {})",
                report
                    .confidence
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            )
        };
        body.push_str(&format!(
            "- {} [{}] {} — {}\n",
            report.report_kind, report.source, report.summary, provenance,
        ));
        if let Some(artifact) = &report.body_artifact_id {
            // GO10 degradation: the raw body lives in an artifact, referenced by
            // id, never inlined. We name it; we never render its raw content.
            body.push_str(&format!("    - body artifact: {artifact}\n"));
        }
    }
    if !view.continuations.is_empty() {
        body.push_str("\n## Continuation decisions\n\n");
        for continuation in &view.continuations {
            body.push_str(&format!(
                "- {} — {}\n",
                continuation.decision, continuation.reason
            ));
        }
    }
    if !view.delegated_provider_goals.is_empty() {
        body.push_str("\n## Delegated provider goals (observed, not authoritative)\n\n");
        for delegated in &view.delegated_provider_goals {
            body.push_str(&format!(
                "- {}: {} [{}]\n",
                delegated.provider_kind, delegated.provider_state, delegated.source
            ));
        }
    }
    body.push_str("\n## Timeline\n\n");
    if timeline.is_empty() {
        body.push_str("_No events recorded._\n");
        degraded = true;
    }
    for record in timeline {
        let state = record.redaction_state.as_str();
        if state != "safe" {
            // GO10 degradation: a redacted event is shown as a redacted reference,
            // not its raw payload.
            degraded = true;
            body.push_str(&format!(
                "- [{}] {} (actor {}) — [redacted:{}]\n",
                record.sequence, record.kind, record.actor, state
            ));
        } else {
            body.push_str(&format!(
                "- [{}] {} (actor {})\n",
                record.sequence, record.kind, record.actor
            ));
        }
    }
    GoalReportRendering {
        goal_id: summary.goal_id.clone(),
        format: GoalReportFormat::Markdown.as_str().to_string(),
        body,
        degraded,
    }
}

/// GA2 (goal-orchestration GO10): render the historical report as JSON. Same
/// derived data as the markdown render; raw artifact bodies are referenced by id,
/// never inlined, and redacted events surface their redaction state.
fn render_report_json(view: &GoalView, timeline: &[EventRecord]) -> GoalReportRendering {
    let summary = &view.summary;
    let mut degraded = timeline.is_empty();
    let timeline_json: Vec<serde_json::Value> = timeline
        .iter()
        .map(|record| {
            let state = record.redaction_state.as_str();
            if state != "safe" {
                degraded = true;
            }
            serde_json::json!({
                "sequence": record.sequence,
                "event_id": record.event_id,
                "kind": record.kind,
                "actor": record.actor,
                "redaction_state": state,
            })
        })
        .collect();
    let value = serde_json::json!({
        "goal_id": summary.goal_id,
        "objective": summary.objective,
        "status": summary.status,
        "parent_goal_id": summary.parent_goal_id,
        "attempt_run_id": summary.attempt_run_id,
        "blocker_reason": summary.blocker_reason,
        "success_criteria_json": view.success_criteria_json,
        "constraints_json": view.constraints_json,
        "verification_surface_json": view.verification_surface_json,
        "budget_json": view.budget_json,
        "stop_conditions_json": view.stop_conditions_json,
        "requirements": view.requirements.iter().map(|requirement| serde_json::json!({
            "requirement_id": requirement.requirement_id,
            "summary": requirement.summary,
            "status": requirement.status,
            "last_status_source": requirement.last_status_source,
            "observed": requirement.observed,
        })).collect::<Vec<_>>(),
        "reports": view.reports.iter().map(|report| serde_json::json!({
            "goal_report_id": report.goal_report_id,
            "requirement_id": report.requirement_id,
            "report_kind": report.report_kind,
            "source": report.source,
            "observed": report.observed,
            "confidence": report.confidence,
            "summary": report.summary,
            "body_artifact_id": report.body_artifact_id,
            "evidence_id": report.evidence_id,
        })).collect::<Vec<_>>(),
        "continuations": view.continuations.iter().map(|continuation| serde_json::json!({
            "continuation_id": continuation.continuation_id,
            "decision": continuation.decision,
            "reason": continuation.reason,
        })).collect::<Vec<_>>(),
        "delegated_provider_goals": view.delegated_provider_goals.iter().map(|delegated| serde_json::json!({
            "delegated_goal_id": delegated.delegated_goal_id,
            "provider_kind": delegated.provider_kind,
            "provider_state": delegated.provider_state,
            "source": delegated.source,
        })).collect::<Vec<_>>(),
        "timeline": timeline_json,
        "degraded": degraded,
    });
    GoalReportRendering {
        goal_id: summary.goal_id.clone(),
        format: GoalReportFormat::Json.as_str().to_string(),
        body: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        degraded,
    }
}
