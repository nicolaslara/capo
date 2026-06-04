//! GA7 (goal-orchestration GO11/GO12): parent/child subgoal reporting and
//! provider-native goal delegation as OBSERVED-NOT-AUTHORITATIVE.
//!
//! Two pure decisions and their controller wiring live here, both built on the
//! GA1 goal/requirement/report/delegated-provider projections and the GA5
//! auditor; nothing here introduces a second completion notion.
//!
//! Parent/child subgoals (GO11). A child agent publishes its progress, evidence,
//! blockers, and completion CLAIMS to its own session AND to the parent Capo goal
//! ([`FakeBoundaryController::report_child_to_parent`] records a
//! `goal.report_recorded` against the PARENT goal, tagged with the child goal /
//! session so the parent-visible story
//! ([`FakeBoundaryController::parent_subgoal_story`]) can attribute it). A child
//! completion claim NEVER automatically satisfies a parent requirement: the
//! [`ParentMergeGate`] is the only thing that lets child work count, and it
//! requires BOTH (1) the child goal itself audited `complete` by the GA5 auditor
//! on OBSERVED evidence, AND (2) a recorded parent merge/review point. A bare
//! child claim -- even a high-confidence one -- is a PROPOSAL: the gate returns
//! `Rejected` until the parent reviews the child's observed result against the
//! subgoal result contract.
//!
//! Subgoal result contract (GO11). [`SubgoalResultContract`] is the explicit
//! merge contract: which parent requirement the child satisfies, the child goal
//! that must be audited complete, and the capability profile / workspace
//! checkpoint / evidence the child report must be scoped to. The contract is the
//! parent's statement of what "done" means for the subgoal, independent of the
//! child's self-report.
//!
//! Provider-native delegation (GO12). Capo does NOT assume a provider has a goal
//! mode: [`ProviderGoalSupport::probe`] feature-probes a provider's advertised
//! capability and yields a [`ProviderGoalCapability`] of `Native` (dispatch to the
//! provider-native goal mode, mirroring objective + success criteria) or
//! `Unavailable` (fall back to Capo's own goal loop). Either way, any
//! provider-native goal STATE and COMPLETION is recorded as a
//! [`DelegatedProviderGoalProjection`] tagged `source=agent_reported` /observed --
//! evidence the GA5 auditor weighs, NEVER an authoritative Capo completion. Codex
//! `/goal` is therefore observed-not-authoritative: it never becomes the Capo goal
//! model (Non-Goals), and a provider-native `completed` state cannot flip the Capo
//! goal -- only the auditor can.

use capo_core::RequirementId;
use capo_state::{
    DelegatedProviderGoalProjection, GoalProjection, GoalReportProjection,
    RequirementLedgerProjection,
};

use super::*;

/// GA7 (GO11): the explicit merge contract for a child subgoal.
///
/// The parent's statement of what "done" means for a subgoal, INDEPENDENT of the
/// child's self-report: which parent requirement the child satisfies, the child
/// goal that must be audited `complete`, and the capability profile / workspace
/// checkpoint / evidence the child report must be scoped to (so a child report is
/// never accepted outside the boundary the parent granted it).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubgoalResultContract {
    /// The child goal whose audited completion this contract gates.
    pub child_goal_id: GoalId,
    /// The parent requirement the child's observed result may satisfy.
    pub parent_requirement_id: RequirementId,
    /// The capability profile the child report must be scoped to (GO11 "keep
    /// child reports scoped by capability profile"). A child report claiming work
    /// outside this profile is rejected by the gate.
    pub capability_profile: String,
    /// The workspace/checkpoint the child work must be bounded by (GO11 "keep
    /// child reports scoped by workspace/checkpoint").
    pub workspace_checkpoint: String,
}

/// GA7 (GO11): a child's completion CLAIM against a parent requirement.
///
/// This is a PROPOSAL the parent weighs; it never flips parent state. It carries
/// the boundary the child actually ran under so the gate can check the claim
/// against the contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildCompletionClaim {
    pub child_goal_id: GoalId,
    pub parent_requirement_id: RequirementId,
    /// The capability profile the child actually ran under.
    pub capability_profile: String,
    /// The workspace/checkpoint the child actually ran under.
    pub workspace_checkpoint: String,
}

/// GA7 (GO11): whether a child's work may satisfy a parent requirement -- the
/// decision the pure [`ParentMergeGate`] produces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParentMergeOutcome {
    /// The child is audited `complete` on observed evidence AND a parent merge
    /// point reviewed it AND the claim is in-scope of the contract: the child's
    /// work may satisfy the parent requirement.
    Merged,
    /// The child's work may NOT (yet) satisfy the parent requirement.
    Rejected,
}

impl ParentMergeOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Rejected => "rejected",
        }
    }

    /// Whether the child's work may satisfy the parent requirement.
    pub const fn is_merged(self) -> bool {
        matches!(self, Self::Merged)
    }
}

/// GA7 (GO11): the pure inputs the merge gate decides over.
///
/// Every field is an OBSERVED fact; the gate reaches outside NOTHING. The
/// load-bearing fields are `child_audited_complete` (the child's OWN GA5 auditor
/// verdict on observed evidence, NOT its self-claim) and `parent_merge_reviewed`
/// (a recorded parent review/merge point). A child completion claim with neither
/// is `Rejected`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParentMergeInputs {
    /// Whether the CHILD goal was audited `complete` by the GA5 auditor on its own
    /// observed evidence. This is the child's auditor verdict, never its claim.
    pub child_audited_complete: bool,
    /// Whether the parent recorded a merge/review point accepting the child result
    /// (a parent-side review report against the contract).
    pub parent_merge_reviewed: bool,
    /// Whether the child's claimed boundary matches the contract's
    /// capability-profile scope.
    pub capability_profile_in_scope: bool,
    /// Whether the child's claimed boundary matches the contract's workspace /
    /// checkpoint scope.
    pub workspace_checkpoint_in_scope: bool,
}

/// GA7 (GO11): the pure decision the merge gate produces -- the outcome plus a
/// stable machine reason code, so "why did (or didn't) the child merge?" is
/// explainable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParentMergeDecision {
    pub outcome: ParentMergeOutcome,
    /// A stable machine reason (e.g. `child_merged`, `child_not_audited_complete`,
    /// `parent_merge_not_reviewed`, `out_of_capability_profile`,
    /// `out_of_workspace_checkpoint`).
    pub reason: &'static str,
}

/// GA7 (GO11): the pure parent/child merge gate.
///
/// Stateless: a namespace for the one pure decision. A child completion claim
/// satisfies a parent requirement ONLY when the child is audited complete on
/// observed evidence, the parent reviewed the merge, and the claim is in-scope of
/// the contract. This is the GO11 invariant in one place: child completion claims
/// do NOT auto-satisfy parent requirements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParentMergeGate;

impl ParentMergeGate {
    /// Decide whether a child's work may satisfy a parent requirement, PURELY.
    ///
    /// The order of checks fixes a deterministic reason: scope first (a child that
    /// ran outside the granted boundary is rejected regardless of its audit), then
    /// the child's own auditor verdict, then the parent merge/review point. Only
    /// when all hold is the outcome `Merged`.
    pub fn decide(inputs: &ParentMergeInputs) -> ParentMergeDecision {
        if !inputs.capability_profile_in_scope {
            return ParentMergeDecision {
                outcome: ParentMergeOutcome::Rejected,
                reason: "out_of_capability_profile",
            };
        }
        if !inputs.workspace_checkpoint_in_scope {
            return ParentMergeDecision {
                outcome: ParentMergeOutcome::Rejected,
                reason: "out_of_workspace_checkpoint",
            };
        }
        if !inputs.child_audited_complete {
            return ParentMergeDecision {
                outcome: ParentMergeOutcome::Rejected,
                reason: "child_not_audited_complete",
            };
        }
        if !inputs.parent_merge_reviewed {
            return ParentMergeDecision {
                outcome: ParentMergeOutcome::Rejected,
                reason: "parent_merge_not_reviewed",
            };
        }
        ParentMergeDecision {
            outcome: ParentMergeOutcome::Merged,
            reason: "child_merged",
        }
    }
}

/// GA7 (GO12): a provider's advertised goal-mode capability, the result of a
/// feature probe -- NOT an assumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderGoalCapability {
    /// The provider advertises a native goal mode (e.g. Codex `/goal`): Capo may
    /// mirror the objective + success criteria and dispatch to it, observing its
    /// events.
    Native,
    /// The provider advertises NO goal mode: Capo falls back to its own goal loop.
    Unavailable,
}

impl ProviderGoalCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Unavailable => "unavailable",
        }
    }

    /// Whether the provider-native goal mode is available to delegate to.
    pub const fn is_native(self) -> bool {
        matches!(self, Self::Native)
    }
}

/// GA7 (GO12): a feature-probe of a provider's goal-mode support.
///
/// Capo does NOT assume a provider has a goal mode. The probe inspects the
/// provider's advertised command surface (the capability strings the adapter
/// exposes) and yields a [`ProviderGoalCapability`]. The probe is recorded with
/// the provider kind and command surface so the limitation is documented with
/// evidence, not assumed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderGoalSupport {
    pub provider_kind: String,
    /// The provider's advertised command surface (e.g. `["/goal", "/status"]`).
    pub command_surface: Vec<String>,
    pub capability: ProviderGoalCapability,
}

impl ProviderGoalSupport {
    /// The provider-native goal command Capo probes for (Codex `/goal`).
    pub const NATIVE_GOAL_COMMAND: &'static str = "/goal";

    /// Feature-probe a provider's goal-mode support from its advertised command
    /// surface. `Native` iff the surface advertises the native goal command;
    /// `Unavailable` otherwise. This is the GO12 "feature-probe rather than
    /// assume" gate.
    pub fn probe(provider_kind: &str, command_surface: &[String]) -> Self {
        let capability = if command_surface
            .iter()
            .any(|command| command == Self::NATIVE_GOAL_COMMAND)
        {
            ProviderGoalCapability::Native
        } else {
            ProviderGoalCapability::Unavailable
        };
        Self {
            provider_kind: provider_kind.to_string(),
            command_surface: command_surface.to_vec(),
            capability,
        }
    }
}

/// GA7 (GO11): one entry of the parent-visible subgoal story -- a child goal and
/// the reports it published up to the parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParentSubgoalStoryEntry {
    pub child_goal: GoalProjection,
    /// The reports the child published to the PARENT goal, oldest first. A claim
    /// (`source=agent_reported`) is a proposal; an observed-source row is evidence.
    pub child_reports: Vec<GoalReportProjection>,
    /// The child's own requirement ledger (its subgoal requirements).
    pub child_requirements: Vec<RequirementLedgerProjection>,
}

impl ParentSubgoalStoryEntry {
    /// The child's completion CLAIMS published to the parent (proposals only).
    pub fn child_completion_claims(&self) -> Vec<&GoalReportProjection> {
        self.child_reports
            .iter()
            .filter(|report| {
                report.is_agent_reported() && report.report_kind == "capo.complete_requirement"
            })
            .collect()
    }
}

impl FakeBoundaryController {
    /// GA7 (GO11): record a child agent's report UP to the parent Capo goal.
    ///
    /// The child publishes progress / evidence / blockers / completion claims to
    /// its own session AND to the parent goal. This records a `goal.report_recorded`
    /// against the PARENT goal, tagged with the child goal id (in the summary's
    /// structured prefix) and the child session, preserving the
    /// observed-vs-reported `source` tag so a child CLAIM is never stored
    /// indistinguishably from observed evidence. It does NOT touch the parent's
    /// requirement ledger: a child claim is a proposal, never an auto-satisfy.
    ///
    /// Idempotent on `report_id`: re-recording the same report re-records nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn report_child_to_parent(
        &self,
        parent_goal_id: &GoalId,
        child_goal_id: &GoalId,
        report_id: &str,
        report_kind: &str,
        source: &str,
        confidence: Option<i64>,
        summary: &str,
        requirement_id: Option<&RequirementId>,
    ) -> StateResult<GoalReportProjection> {
        let parent = self
            .state
            .goal(parent_goal_id)?
            .ok_or_else(|| missing_read_model("goal", parent_goal_id))?;
        let child = self
            .state
            .goal(child_goal_id)?
            .ok_or_else(|| missing_read_model("goal", child_goal_id))?;

        let projection = GoalReportProjection {
            goal_report_id: report_id.to_string(),
            goal_id: parent_goal_id.clone(),
            project_id: self.project_id.clone(),
            // The child's OWN session: the report is attributed to the child while
            // being published up to the parent goal.
            session_id: child.session_id.clone(),
            requirement_id: requirement_id.cloned(),
            report_kind: report_kind.to_string(),
            source: source.to_string(),
            confidence,
            summary: summary.to_string(),
            body_artifact_id: None,
            evidence_id: None,
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "goal_report_id": report_id,
            "parent_goal_id": parent_goal_id.as_str(),
            "child_goal_id": child_goal_id.as_str(),
            "report_kind": report_kind,
            "source": source,
            "confidence": confidence,
            "summary": summary,
            "requirement_id": requirement_id.map(RequirementId::as_str),
        })
        .to_string();

        let event = NewEvent {
            event_id: format!("event-child-report-{report_id}"),
            kind: EventKind::GoalReportRecorded,
            actor: "capo-controller-subgoal".to_string(),
            project_id: Some(self.project_id.clone()),
            task_id: parent.task_id.clone(),
            agent_id: child.agent_id.clone(),
            session_id: child.session_id.clone(),
            run_id: child.attempt_run_id.clone(),
            turn_id: None,
            item_id: Some(report_id.to_string()),
            payload_json: payload,
            idempotency_key: Some(format!("child-report:{report_id}")),
            redaction_state: RedactionState::Safe,
        };

        self.state
            .append_event(event, &[ProjectionRecord::GoalReport(projection.clone())])?;

        Ok(projection)
    }

    /// GA7 (GO11): the parent-visible subgoal story -- every child goal of the
    /// parent and the reports each published up.
    ///
    /// A read-only projection over the GA1 goal/report/requirement projections; it
    /// appends nothing. The story attributes each child report by the child goal it
    /// belongs to (matched on the child's session) so the parent sees a clear
    /// per-subgoal picture with claims (proposals) and observed evidence
    /// distinguished by `source`.
    pub fn parent_subgoal_story(
        &self,
        parent_goal_id: &GoalId,
    ) -> StateResult<Vec<ParentSubgoalStoryEntry>> {
        let children = self.state.child_goals_for_parent(parent_goal_id)?;
        let parent_reports = self.state.goal_reports_for_goal(parent_goal_id)?;

        let mut story = Vec::with_capacity(children.len());
        for child in children {
            // Reports the child published to the parent are attributed by the
            // child's session (the session the child report carries).
            let child_reports: Vec<GoalReportProjection> = parent_reports
                .iter()
                .filter(|report| report.session_id == child.session_id)
                .cloned()
                .collect();
            let child_requirements = self.state.requirement_ledgers_for_goal(&child.goal_id)?;
            story.push(ParentSubgoalStoryEntry {
                child_goal: child,
                child_reports,
                child_requirements,
            });
        }
        Ok(story)
    }

    /// GA7 (GO11): decide whether a child's work may satisfy a parent requirement.
    ///
    /// Folds the OBSERVED facts the [`ParentMergeGate`] needs from persisted state
    /// -- the child's OWN latest GA5 auditor verdict (`child_audited_complete`),
    /// whether the parent recorded a merge/review point against the contract, and
    /// whether the claim is in-scope of the contract -- and returns the pure
    /// decision. A child completion claim alone never reaches `Merged`: the child
    /// must be audited complete on observed evidence AND the parent must have
    /// reviewed the merge.
    ///
    /// The parent merge/review point is observed as a parent-goal report of kind
    /// `capo.record_validation` (or `capo.merge_subgoal`) tagged to the parent
    /// requirement, citing the child goal in its summary -- a recorded, observed
    /// parent decision, not the child's self-report.
    pub fn evaluate_parent_merge(
        &self,
        parent_goal_id: &GoalId,
        contract: &SubgoalResultContract,
        claim: &ChildCompletionClaim,
    ) -> StateResult<ParentMergeDecision> {
        // The child's OWN auditor verdict on observed evidence -- never the child's
        // self-claim. A child with no audit verdict is not complete.
        let child_audited_complete = self
            .state
            .latest_goal_audit_decision(&contract.child_goal_id)?
            .is_some_and(|decision| decision.is_complete());

        // The parent's recorded merge/review point: a parent-goal report against
        // the parent requirement that cites the child goal. This is the parent's
        // observed decision, recorded separately from the child's claim.
        let parent_reports = self.state.goal_reports_for_goal(parent_goal_id)?;
        let parent_merge_reviewed = parent_reports.iter().any(|report| {
            report.requirement_id.as_ref() == Some(&contract.parent_requirement_id)
                && (report.report_kind == "capo.merge_subgoal"
                    || report.report_kind == "capo.record_validation")
                && report.summary.contains(contract.child_goal_id.as_str())
        });

        let inputs = ParentMergeInputs {
            child_audited_complete,
            parent_merge_reviewed,
            capability_profile_in_scope: claim.capability_profile == contract.capability_profile,
            workspace_checkpoint_in_scope: claim.workspace_checkpoint
                == contract.workspace_checkpoint,
        };

        Ok(ParentMergeGate::decide(&inputs))
    }

    /// GA7 (GO12): record an OBSERVED provider-native goal observation.
    ///
    /// When Capo delegates to a provider-native goal mode (e.g. Codex `/goal`), the
    /// provider's reported goal state, command surface, and completion are recorded
    /// as a [`DelegatedProviderGoalProjection`] tagged `source` (an
    /// `agent_reported`/observed tag), NEVER an authoritative Capo completion. A
    /// provider-native `completed` state is evidence the GA5 auditor weighs; it does
    /// not flip the Capo goal. Idempotent on `delegated_goal_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn record_delegated_provider_goal(
        &self,
        goal_id: &GoalId,
        delegated_goal_id: &str,
        provider_kind: &str,
        provider_goal_ref: Option<&str>,
        provider_state: &str,
        source: &str,
        body_artifact_id: Option<&str>,
    ) -> StateResult<DelegatedProviderGoalProjection> {
        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;

        let projection = DelegatedProviderGoalProjection {
            delegated_goal_id: delegated_goal_id.to_string(),
            goal_id: goal_id.clone(),
            project_id: self.project_id.clone(),
            session_id: goal.session_id.clone(),
            provider_kind: provider_kind.to_string(),
            provider_goal_ref: provider_goal_ref.map(str::to_string),
            provider_state: provider_state.to_string(),
            source: source.to_string(),
            body_artifact_id: body_artifact_id.map(str::to_string),
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "delegated_goal_id": delegated_goal_id,
            "goal_id": goal_id.as_str(),
            "provider_kind": provider_kind,
            "provider_goal_ref": provider_goal_ref,
            "provider_state": provider_state,
            "source": source,
            "body_artifact_id": body_artifact_id,
        })
        .to_string();

        let event = NewEvent {
            event_id: format!("event-delegated-{delegated_goal_id}"),
            kind: EventKind::DelegatedProviderGoalObserved,
            actor: "capo-controller-delegation".to_string(),
            project_id: Some(self.project_id.clone()),
            task_id: goal.task_id.clone(),
            agent_id: goal.agent_id.clone(),
            session_id: goal.session_id.clone(),
            run_id: goal.attempt_run_id.clone(),
            turn_id: None,
            item_id: Some(delegated_goal_id.to_string()),
            payload_json: payload,
            idempotency_key: Some(format!("delegated:{delegated_goal_id}")),
            redaction_state: RedactionState::Safe,
        };

        self.state.append_event(
            event,
            &[ProjectionRecord::DelegatedProviderGoal(projection.clone())],
        )?;

        Ok(projection)
    }
}

#[cfg(test)]
mod tests {

    use capo_state::{
        EvidenceProjection, GoalProjection, NewEvent, ProjectionRecord, RequirementLedgerProjection,
    };

    use super::*;

    fn temp_root(name: &str) -> capo_tmptest::TempRoot {
        capo_tmptest::TempRoot::new(&format!("capo-ga7-{name}"))
    }

    const PROJECT: &str = "project-capo";
    const PARENT: &str = "goal-parent";
    const CHILD: &str = "goal-child";
    const PARENT_SESSION: &str = "session-parent";
    const CHILD_SESSION: &str = "session-child";

    fn open() -> (FakeBoundaryController, capo_tmptest::TempRoot) {
        let state_root = temp_root("state");
        let controller =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("controller");
        (controller, state_root)
    }

    fn seed(controller: &FakeBoundaryController, event_id: &str, records: &[ProjectionRecord]) {
        let mut event = NewEvent::new(event_id, EventKind::GoalCreated, "test-seed");
        event.project_id = Some(ProjectId::new(PROJECT));
        event.idempotency_key = Some(event_id.to_string());
        event.item_id = Some(event_id.to_string());
        controller
            .state
            .append_event(event, records)
            .expect("seed append");
    }

    fn goal(goal_id: &str, session_id: &str, parent: Option<&str>) -> GoalProjection {
        GoalProjection {
            goal_id: GoalId::new(goal_id),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new(format!("task-{goal_id}"))),
            agent_id: None,
            session_id: Some(SessionId::new(session_id)),
            parent_goal_id: parent.map(GoalId::new),
            attempt_run_id: None,
            objective: format!("objective for {goal_id}"),
            status: GoalProjection::ACTIVE.to_string(),
            success_criteria_json: "{}".to_string(),
            constraints_json: "{}".to_string(),
            verification_surface_json: "{}".to_string(),
            budget_json: "{}".to_string(),
            stop_conditions_json: "{}".to_string(),
            blocker_reason: String::new(),
            updated_sequence: 0,
        }
    }

    fn requirement(goal_id: &str, req_id: &str, status: &str, source: &str) -> ProjectionRecord {
        ProjectionRecord::RequirementLedger(RequirementLedgerProjection {
            requirement_id: RequirementId::new(req_id),
            goal_id: GoalId::new(goal_id),
            project_id: ProjectId::new(PROJECT),
            summary: format!("requirement {req_id}"),
            status: status.to_string(),
            last_status_source: source.to_string(),
            updated_sequence: 0,
        })
    }

    fn child_observed_evidence(evidence_id: &str) -> ProjectionRecord {
        ProjectionRecord::Evidence(EvidenceProjection {
            evidence_id: EvidenceId::new(evidence_id),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new(format!("task-{CHILD}"))),
            session_id: Some(SessionId::new(CHILD_SESSION)),
            run_id: None,
            kind: "test".to_string(),
            artifact_id: None,
            confidence: 90,
            updated_sequence: 0,
        })
    }

    // ----- Pure merge-gate branch coverage (no DB) --------------------------

    fn merge_inputs() -> ParentMergeInputs {
        ParentMergeInputs {
            child_audited_complete: true,
            parent_merge_reviewed: true,
            capability_profile_in_scope: true,
            workspace_checkpoint_in_scope: true,
        }
    }

    #[test]
    fn pure_merge_gate_merges_only_when_audited_reviewed_and_in_scope() {
        let decision = ParentMergeGate::decide(&merge_inputs());
        assert_eq!(decision.outcome, ParentMergeOutcome::Merged);
        assert_eq!(decision.reason, "child_merged");
    }

    #[test]
    fn pure_merge_gate_rejects_child_claim_without_audit() {
        // GO11 invariant: a child completion claim does NOT auto-satisfy a parent
        // requirement. Without the child's own auditor verdict, the gate rejects.
        let decision = ParentMergeGate::decide(&ParentMergeInputs {
            child_audited_complete: false,
            ..merge_inputs()
        });
        assert_eq!(decision.outcome, ParentMergeOutcome::Rejected);
        assert_eq!(decision.reason, "child_not_audited_complete");
    }

    #[test]
    fn pure_merge_gate_rejects_without_parent_merge_review() {
        let decision = ParentMergeGate::decide(&ParentMergeInputs {
            parent_merge_reviewed: false,
            ..merge_inputs()
        });
        assert_eq!(decision.outcome, ParentMergeOutcome::Rejected);
        assert_eq!(decision.reason, "parent_merge_not_reviewed");
    }

    #[test]
    fn pure_merge_gate_rejects_out_of_scope_claim() {
        let profile = ParentMergeGate::decide(&ParentMergeInputs {
            capability_profile_in_scope: false,
            ..merge_inputs()
        });
        assert_eq!(profile.reason, "out_of_capability_profile");
        let workspace = ParentMergeGate::decide(&ParentMergeInputs {
            workspace_checkpoint_in_scope: false,
            ..merge_inputs()
        });
        assert_eq!(workspace.reason, "out_of_workspace_checkpoint");
    }

    // ----- Pure provider feature-probe (no DB) ------------------------------

    #[test]
    fn provider_goal_probe_detects_native_and_unavailable() {
        let native =
            ProviderGoalSupport::probe("codex", &["/goal".to_string(), "/status".to_string()]);
        assert_eq!(native.capability, ProviderGoalCapability::Native);
        assert!(native.capability.is_native());

        let unavailable = ProviderGoalSupport::probe("fake", &["/chat".to_string()]);
        assert_eq!(unavailable.capability, ProviderGoalCapability::Unavailable);
        assert!(!unavailable.capability.is_native());
    }

    // ----- Controller wiring over persisted state ---------------------------

    #[test]
    fn child_reports_publish_up_to_parent_and_form_a_subgoal_story() {
        // GO11: a child publishes progress + a completion CLAIM up to the parent;
        // the parent-visible story attributes them to the child subgoal.
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-parent",
            &[
                ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None)),
                requirement(
                    PARENT,
                    "parent-req-1",
                    RequirementLedgerProjection::UNVERIFIED,
                    "agent_reported",
                ),
            ],
        );
        seed(
            &controller,
            "seed-child",
            &[
                ProjectionRecord::Goal(goal(CHILD, CHILD_SESSION, Some(PARENT))),
                requirement(
                    CHILD,
                    "child-req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "runtime_output",
                ),
            ],
        );

        controller
            .report_child_to_parent(
                &GoalId::new(PARENT),
                &GoalId::new(CHILD),
                "child-progress-1",
                "capo.report_progress",
                "agent_reported",
                Some(70),
                "child made progress",
                None,
            )
            .expect("progress report");
        controller
            .report_child_to_parent(
                &GoalId::new(PARENT),
                &GoalId::new(CHILD),
                "child-claim-1",
                "capo.complete_requirement",
                "agent_reported",
                Some(99),
                "child claims done",
                Some(&RequirementId::new("parent-req-1")),
            )
            .expect("claim report");

        let story = controller
            .parent_subgoal_story(&GoalId::new(PARENT))
            .expect("story");
        assert_eq!(story.len(), 1, "one child subgoal");
        let entry = &story[0];
        assert_eq!(entry.child_goal.goal_id, GoalId::new(CHILD));
        assert_eq!(entry.child_reports.len(), 2, "both child reports up");
        assert_eq!(entry.child_requirements.len(), 1);
        assert_eq!(entry.child_completion_claims().len(), 1);

        // The child claim did NOT auto-satisfy the parent requirement: the parent
        // requirement is still unverified.
        let parent_reqs = controller
            .state()
            .requirement_ledgers_for_goal(&GoalId::new(PARENT))
            .expect("parent reqs");
        assert_eq!(
            parent_reqs[0].status,
            RequirementLedgerProjection::UNVERIFIED,
            "child claim does not flip the parent requirement"
        );
    }

    #[test]
    fn child_claim_alone_does_not_merge_into_parent_requirement() {
        // The end-to-end GO11 invariant over persisted state: a child completion
        // claim, with NO child auditor verdict and NO parent merge review, is
        // rejected by the merge gate.
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-parent",
            &[
                ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None)),
                requirement(
                    PARENT,
                    "parent-req-1",
                    RequirementLedgerProjection::UNVERIFIED,
                    "agent_reported",
                ),
            ],
        );
        seed(
            &controller,
            "seed-child",
            &[ProjectionRecord::Goal(goal(
                CHILD,
                CHILD_SESSION,
                Some(PARENT),
            ))],
        );
        controller
            .report_child_to_parent(
                &GoalId::new(PARENT),
                &GoalId::new(CHILD),
                "child-claim-1",
                "capo.complete_requirement",
                "agent_reported",
                Some(99),
                "child claims done",
                Some(&RequirementId::new("parent-req-1")),
            )
            .expect("claim report");

        let contract = SubgoalResultContract {
            child_goal_id: GoalId::new(CHILD),
            parent_requirement_id: RequirementId::new("parent-req-1"),
            capability_profile: "trusted-local".to_string(),
            workspace_checkpoint: "checkpoint-1".to_string(),
        };
        let claim = ChildCompletionClaim {
            child_goal_id: GoalId::new(CHILD),
            parent_requirement_id: RequirementId::new("parent-req-1"),
            capability_profile: "trusted-local".to_string(),
            workspace_checkpoint: "checkpoint-1".to_string(),
        };

        let decision = controller
            .evaluate_parent_merge(&GoalId::new(PARENT), &contract, &claim)
            .expect("merge");
        assert_eq!(decision.outcome, ParentMergeOutcome::Rejected);
        assert_eq!(decision.reason, "child_not_audited_complete");
    }

    #[test]
    fn child_audited_complete_and_parent_reviewed_merges() {
        // The full GO11 merge path: the child is audited complete on OBSERVED
        // evidence (via the GA5 auditor) AND the parent records a merge/review
        // point citing the child -> the merge gate accepts.
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-parent",
            &[
                ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None)),
                requirement(
                    PARENT,
                    "parent-req-1",
                    RequirementLedgerProjection::SUPPORTED,
                    "agent_reported",
                ),
            ],
        );
        seed(
            &controller,
            "seed-child",
            &[
                ProjectionRecord::Goal(goal(CHILD, CHILD_SESSION, Some(PARENT))),
                requirement(
                    CHILD,
                    "child-req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "runtime_output",
                ),
                child_observed_evidence("child-evidence-1"),
            ],
        );

        // The child is audited complete on its OWN observed evidence (GA5).
        let child_audit = controller
            .audit_and_record_goal_completion(&GoalId::new(CHILD), "child-audit-1")
            .expect("child audit");
        assert_eq!(child_audit.verdict, AuditVerdict::Complete);

        // The parent records a merge/review point against its requirement, citing
        // the child goal -- the parent's observed decision.
        controller
            .report_child_to_parent(
                &GoalId::new(PARENT),
                &GoalId::new(CHILD),
                "parent-merge-review-1",
                "capo.merge_subgoal",
                "runtime_output",
                None,
                &format!("parent reviewed and merged subgoal {CHILD}"),
                Some(&RequirementId::new("parent-req-1")),
            )
            .expect("merge review");

        let contract = SubgoalResultContract {
            child_goal_id: GoalId::new(CHILD),
            parent_requirement_id: RequirementId::new("parent-req-1"),
            capability_profile: "trusted-local".to_string(),
            workspace_checkpoint: "checkpoint-1".to_string(),
        };
        let claim = ChildCompletionClaim {
            child_goal_id: GoalId::new(CHILD),
            parent_requirement_id: RequirementId::new("parent-req-1"),
            capability_profile: "trusted-local".to_string(),
            workspace_checkpoint: "checkpoint-1".to_string(),
        };

        let decision = controller
            .evaluate_parent_merge(&GoalId::new(PARENT), &contract, &claim)
            .expect("merge");
        assert_eq!(decision.outcome, ParentMergeOutcome::Merged);
        assert_eq!(decision.reason, "child_merged");
    }

    #[test]
    fn delegated_provider_completion_is_recorded_as_evidence_and_audited_not_auto_completed() {
        // GO12: a provider-native `completed` state is recorded as OBSERVED
        // delegated-provider evidence (source=agent_reported), NEVER an
        // authoritative Capo completion. With no observed requirement evidence the
        // GA5 auditor judges the goal incomplete despite the provider's "completed".
        let (controller, _root) = open();

        // Feature-probe: Codex advertises a native goal mode, so Capo mirrors the
        // objective and delegates.
        let support = ProviderGoalSupport::probe(
            "codex",
            &[ProviderGoalSupport::NATIVE_GOAL_COMMAND.to_string()],
        );
        assert!(support.capability.is_native());

        seed(
            &controller,
            "seed-goal",
            &[
                ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None)),
                requirement(
                    PARENT,
                    "req-1",
                    RequirementLedgerProjection::VALIDATED,
                    // Provenance is an agent claim only -- no observed evidence.
                    "agent_reported",
                ),
            ],
        );

        controller
            .record_delegated_provider_goal(
                &GoalId::new(PARENT),
                "delegated-1",
                "codex",
                Some("codex-goal-abc"),
                // The provider reports its goal "completed" -- a CLAIM, observed
                // only as evidence.
                "completed",
                "agent_reported",
                None,
            )
            .expect("delegated record");

        // The provider-native completion is recorded as observed-not-authoritative
        // evidence.
        let delegated = controller
            .state()
            .delegated_provider_goals_for_goal(&GoalId::new(PARENT))
            .expect("delegated");
        assert_eq!(delegated.len(), 1);
        assert_eq!(delegated[0].provider_state, "completed");
        assert_eq!(delegated[0].source, "agent_reported");

        // Auditing through GA5: the provider's "completed" did NOT flip the Capo
        // goal -- the requirement has no observed evidence, so the goal is
        // incomplete (the overclaim guard).
        let audit = controller
            .audit_and_record_goal_completion(&GoalId::new(PARENT), "audit-1")
            .expect("audit");
        assert_eq!(audit.verdict, AuditVerdict::Incomplete);
        assert_eq!(audit.reason, "requirement_claim_only");

        let latest = controller
            .state()
            .latest_goal_audit_decision(&GoalId::new(PARENT))
            .expect("latest")
            .expect("decision");
        assert!(!latest.is_complete());
    }

    #[test]
    fn provider_unavailable_falls_back_and_records_observed_state() {
        // GO12 fallback: a provider with NO goal mode falls back to Capo's loop;
        // any provider goal state is still recorded as observed evidence.
        let (controller, _root) = open();
        let support = ProviderGoalSupport::probe("fake", &["/chat".to_string()]);
        assert_eq!(support.capability, ProviderGoalCapability::Unavailable);

        seed(
            &controller,
            "seed-goal",
            &[ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None))],
        );

        controller
            .record_delegated_provider_goal(
                &GoalId::new(PARENT),
                "delegated-fallback",
                "fake",
                None,
                "unsupported",
                "agent_reported",
                None,
            )
            .expect("delegated record");

        let delegated = controller
            .state()
            .delegated_provider_goals_for_goal(&GoalId::new(PARENT))
            .expect("delegated");
        assert_eq!(delegated.len(), 1);
        assert_eq!(delegated[0].provider_state, "unsupported");
    }

    #[test]
    fn parent_child_and_delegation_survive_restart_and_rebuild() {
        // GA6/GA7: parent/child reports and delegated-provider observations rebuild
        // identically after a server restart + projection rebuild.
        let state_root = temp_root("restart");
        {
            let controller = FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root)
                .expect("controller");
            seed(
                &controller,
                "seed-parent",
                &[ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None))],
            );
            seed(
                &controller,
                "seed-child",
                &[ProjectionRecord::Goal(goal(
                    CHILD,
                    CHILD_SESSION,
                    Some(PARENT),
                ))],
            );
            controller
                .report_child_to_parent(
                    &GoalId::new(PARENT),
                    &GoalId::new(CHILD),
                    "child-progress-1",
                    "capo.report_progress",
                    "agent_reported",
                    Some(50),
                    "child progress",
                    None,
                )
                .expect("child report");
            controller
                .record_delegated_provider_goal(
                    &GoalId::new(PARENT),
                    "delegated-1",
                    "codex",
                    Some("codex-goal-abc"),
                    "running",
                    "adapter_event",
                    None,
                )
                .expect("delegated");
        }

        let reopened =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("reopen");
        reopened.state().rebuild_projections().expect("rebuild");

        let story = reopened
            .parent_subgoal_story(&GoalId::new(PARENT))
            .expect("story");
        assert_eq!(story.len(), 1);
        assert_eq!(story[0].child_reports.len(), 1);

        let delegated = reopened
            .state()
            .delegated_provider_goals_for_goal(&GoalId::new(PARENT))
            .expect("delegated");
        assert_eq!(delegated.len(), 1);
        assert_eq!(delegated[0].provider_state, "running");
        assert_eq!(delegated[0].source, "adapter_event");
    }

    #[test]
    fn delegated_provider_recording_is_idempotent() {
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-goal",
            &[ProjectionRecord::Goal(goal(PARENT, PARENT_SESSION, None))],
        );
        for _ in 0..2 {
            controller
                .record_delegated_provider_goal(
                    &GoalId::new(PARENT),
                    "delegated-dup",
                    "codex",
                    None,
                    "running",
                    "adapter_event",
                    None,
                )
                .expect("delegated");
        }
        let delegated = controller
            .state()
            .delegated_provider_goals_for_goal(&GoalId::new(PARENT))
            .expect("delegated");
        assert_eq!(delegated.len(), 1, "verbatim re-record re-records nothing");
    }
}
