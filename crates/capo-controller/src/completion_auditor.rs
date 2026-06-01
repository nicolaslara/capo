//! GA5 (goal-orchestration GO9): the evidence-gated completion auditor.
//!
//! What this is and what it is NOT. The auditor is a PURE function over a
//! snapshot of OBSERVED goal state: [`CompletionAuditor::audit`] takes
//! [`AuditInputs`] (the goal's requirements, the observed-vs-reported provenance
//! of each requirement's last status, and whether each requirement is backed by
//! concrete observed evidence) and returns an [`AuditDecision`] -- a goal-level
//! `complete | incomplete` verdict plus per-requirement detail. It performs NO
//! I/O, appends NO event, and holds NO state of its own, so every scenario
//! (complete, partial, weak-evidence, contradicted, blocked, overclaimed) is
//! exhaustively testable without a database or a live provider. The controller
//! wiring ([`FakeBoundaryController::audit_goal_completion`] /
//! [`FakeBoundaryController::audit_and_record_goal_completion`]) is the only thing
//! that touches persisted state: it reads the GA1 goal/requirement/report/evidence
//! projections, folds them into [`AuditInputs`], calls the pure decision, and (for
//! the recording variant) durably records the verdict through a
//! `goal.audit_decision_recorded` event + [`GoalAuditDecisionProjection`].
//!
//! The auditor is the ONLY path to a Capo goal-complete transition (GO9 +
//! knowledge.md). Agents PROPOSE completion; they never ASSERT it. A
//! `capo.complete_requirement` / `capo.complete_subtask` report and any
//! provider-native completion are recorded as `source=agent_reported` / observed
//! evidence (GA1/GA2) and never directly flip goal state -- the only way a goal is
//! ever judged complete is an auditor verdict of `complete`, recorded HERE.
//!
//! Evidence over prose (GO9 + Non-Goals). A requirement is judged complete ONLY
//! when it reached a satisfying ledger state (`validated` / `reviewed`) AND that
//! state is backed by CONCRETE OBSERVED EVIDENCE (a runtime/adapter observation or
//! a verification-runner verdict, classified exactly like the `tools-aci` evidence
//! sources). A requirement whose only support is an agent claim -- even a
//! high-confidence `capo.complete_requirement` report -- is NOT complete: its
//! audited state is `claim_only`. The auditor never consults a global/aggregate
//! confidence to substitute for requirement-level evidence (Non-Goals), so an
//! overclaimed goal with confident prose but no observed evidence stays
//! `incomplete`.
//!
//! Requirement states (GO9). The auditor distinguishes the six requirement states
//! the ledger carries -- `unverified`, `supported`, `validated`, `reviewed`,
//! `blocked`, `contradicted` -- and additionally records `claim_only` (a satisfying
//! ledger status NOT backed by observed evidence) and `weak` (a satisfying status
//! whose only validation is explicitly weak/skipped) so the verdict explains
//! exactly why a requirement did or did not count. Skipped or weak validation is
//! recorded explicitly rather than silently treated as a pass.
//!
//! Derived, not hand-written (GO9). The verdict and every per-requirement decision
//! are recorded so "why is this goal (not) complete?" is a derived read model
//! ([`GoalAuditDecisionProjection`] + its `requirement_detail_json`), never
//! hand-written prose.

use capo_core::RequirementId;
use capo_state::{
    EvidenceProjection, GoalAuditDecisionProjection, GoalProjection, GoalReportProjection,
    RequirementLedgerProjection,
};

use super::*;

/// GA5 (GO9): the audited state of one requirement, as judged by the auditor.
///
/// This is RICHER than the GA1 ledger status: it folds the ledger status together
/// with the observed-evidence check and the validation strength so the verdict is
/// fully explainable. `Validated`/`Reviewed` are the only states that COUNT toward
/// goal-complete, and only when [`RequirementAudit::observed_evidence`] holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequirementAuditState {
    /// The requirement reached `validated` AND is backed by observed evidence:
    /// it counts toward completion.
    Validated,
    /// The requirement reached `reviewed` AND is backed by observed evidence:
    /// it counts toward completion.
    Reviewed,
    /// The requirement has only an agent claim (`supported` by a claim, or a
    /// satisfying status NOT backed by observed evidence). A PROPOSAL only; it
    /// does NOT count toward completion. This is the overclaim guard.
    ClaimOnly,
    /// The requirement has support but no validation/review yet (`supported`
    /// backed by observed evidence). Not yet complete.
    Supported,
    /// A satisfying status whose validation is explicitly weak or skipped. Not
    /// complete; recorded explicitly rather than treated as a pass.
    Weak,
    /// No verification at all (`unverified`). Not complete.
    Unverified,
    /// A raised blocker on the requirement (`blocked`). Blocks the goal.
    Blocked,
    /// Observed evidence contradicts the requirement (`contradicted`). Blocks the
    /// goal.
    Contradicted,
}

impl RequirementAuditState {
    /// The stable machine string recorded in the per-requirement detail.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validated => "validated",
            Self::Reviewed => "reviewed",
            Self::ClaimOnly => "claim_only",
            Self::Supported => "supported",
            Self::Weak => "weak",
            Self::Unverified => "unverified",
            Self::Blocked => "blocked",
            Self::Contradicted => "contradicted",
        }
    }

    /// Whether this audited state COUNTS toward goal-complete. Only an observed
    /// `validated`/`reviewed` requirement does.
    pub const fn counts_complete(self) -> bool {
        matches!(self, Self::Validated | Self::Reviewed)
    }

    /// Whether this state is a HARD blocker (a blocked or contradicted
    /// requirement can never make the goal complete until it is resolved).
    pub const fn is_blocking(self) -> bool {
        matches!(self, Self::Blocked | Self::Contradicted)
    }
}

/// GA5 (GO9): the per-requirement input the pure auditor decides over.
///
/// Every field is an OBSERVED fact read from persisted goal state; the auditor
/// never reaches outside this struct. `ledger_status` is the GA1 requirement
/// ledger status; `observed_evidence` is whether the requirement is backed by
/// concrete observed evidence (a runtime/adapter observation or verification
/// verdict, NOT an agent claim); `weak_validation` records an explicitly weak or
/// skipped validation on an otherwise-satisfying requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementInput {
    pub requirement_id: RequirementId,
    /// The GA1 ledger status: `unverified` / `supported` / `validated` /
    /// `reviewed` / `blocked` / `contradicted`.
    pub ledger_status: String,
    /// Whether this requirement is backed by CONCRETE OBSERVED EVIDENCE. An
    /// `agent_reported` claim alone is NOT observed evidence, so this is `false`
    /// for a requirement whose only support is a claim.
    pub observed_evidence: bool,
    /// Whether the requirement's validation is explicitly weak/skipped. A weak
    /// validation never counts as a pass.
    pub weak_validation: bool,
}

/// GA5 (GO9): the audited result for one requirement -- its input plus the
/// auditor's judged state and a machine reason. Recorded in the verdict's
/// `requirement_detail_json` so the read model explains every requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementAudit {
    pub requirement_id: RequirementId,
    pub state: RequirementAuditState,
    /// Whether the requirement is backed by concrete observed evidence.
    pub observed_evidence: bool,
    /// A stable machine reason code for the audited state.
    pub reason: &'static str,
}

/// GA5 (GO9): the full snapshot the pure auditor decides over.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditInputs {
    pub requirements: Vec<RequirementInput>,
}

/// GA5 (GO9): the auditor's goal-level verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditVerdict {
    /// Every requirement reached a satisfying state backed by observed evidence.
    /// This is the ONLY value that marks a Capo goal complete.
    Complete,
    /// At least one requirement is not complete (blocked, contradicted, claim-only,
    /// weak, supported-only, unverified) or the goal has no requirements.
    Incomplete,
}

impl AuditVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => GoalAuditDecisionProjection::COMPLETE,
            Self::Incomplete => GoalAuditDecisionProjection::INCOMPLETE,
        }
    }

    /// Whether this verdict marks the goal complete.
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// GA5 (GO9): the decision the pure auditor produces -- the goal verdict, a stable
/// machine reason code, and the per-requirement audited detail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditDecision {
    pub verdict: AuditVerdict,
    /// A stable machine reason code (e.g. `all_requirements_met`, `no_requirements`,
    /// `requirement_blocked`, `requirement_contradicted`, `requirement_claim_only`,
    /// `requirement_weak_validation`, `requirement_supported_only`,
    /// `requirement_unverified`).
    pub reason: &'static str,
    /// The audited detail for every requirement, in input order.
    pub requirements: Vec<RequirementAudit>,
}

impl AuditDecision {
    /// How many requirements the auditor judged complete.
    pub fn requirements_complete(&self) -> usize {
        self.requirements
            .iter()
            .filter(|requirement| requirement.state.counts_complete())
            .count()
    }
}

/// GA5 (GO9): the pure evidence-gated completion auditor.
///
/// Stateless: a namespace for the one pure decision function. The auditor holds
/// nothing -- all state is in [`AuditInputs`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompletionAuditor;

impl CompletionAuditor {
    /// Audit one requirement, PURELY. Folds the ledger status with the
    /// observed-evidence check and the weak-validation flag into the richer
    /// [`RequirementAuditState`].
    ///
    /// The load-bearing gate (GO9): a `validated`/`reviewed` requirement counts
    /// toward completion ONLY when it is backed by observed evidence. A satisfying
    /// status with no observed evidence is `ClaimOnly` (the overclaim guard); a
    /// satisfying status whose validation is explicitly weak/skipped is `Weak`.
    /// Blocked/contradicted are hard blockers; supported/unverified are not yet
    /// complete.
    fn audit_requirement(input: &RequirementInput) -> RequirementAudit {
        let (state, reason) = match input.ledger_status.as_str() {
            RequirementLedgerProjection::BLOCKED => {
                (RequirementAuditState::Blocked, "requirement_blocked")
            }
            RequirementLedgerProjection::CONTRADICTED => (
                RequirementAuditState::Contradicted,
                "requirement_contradicted",
            ),
            RequirementLedgerProjection::VALIDATED | RequirementLedgerProjection::REVIEWED => {
                // A satisfying status only counts when backed by observed evidence
                // and not explicitly weak. Otherwise it is downgraded so the
                // verdict explains exactly why it did not count.
                if !input.observed_evidence {
                    (RequirementAuditState::ClaimOnly, "requirement_claim_only")
                } else if input.weak_validation {
                    (RequirementAuditState::Weak, "requirement_weak_validation")
                } else if input.ledger_status == RequirementLedgerProjection::REVIEWED {
                    (RequirementAuditState::Reviewed, "requirement_reviewed")
                } else {
                    (RequirementAuditState::Validated, "requirement_validated")
                }
            }
            RequirementLedgerProjection::SUPPORTED => {
                // `supported` is short of validation/review regardless of evidence,
                // but distinguish a claim-only `supported` from an observed one so
                // the read model is precise.
                if input.observed_evidence {
                    (
                        RequirementAuditState::Supported,
                        "requirement_supported_only",
                    )
                } else {
                    (RequirementAuditState::ClaimOnly, "requirement_claim_only")
                }
            }
            // `unverified` or any unrecognized status: never complete.
            _ => (RequirementAuditState::Unverified, "requirement_unverified"),
        };
        RequirementAudit {
            requirement_id: input.requirement_id.clone(),
            state,
            observed_evidence: input.observed_evidence,
            reason,
        }
    }

    /// Decide whether the goal is complete, as a PURE function of `inputs`.
    ///
    /// A goal is `complete` iff it has at least one requirement and EVERY
    /// requirement is complete (an observed `validated`/`reviewed`). Otherwise it
    /// is `incomplete`, and the reason names the FIRST requirement that is not
    /// complete (hard blockers first, then the other not-complete states), so the
    /// "why not complete?" answer points at a concrete requirement. A goal with no
    /// requirements is `incomplete` (`no_requirements`): completion is never
    /// reachable without at least one observed-evidence-backed requirement.
    pub fn audit(inputs: &AuditInputs) -> AuditDecision {
        let requirements: Vec<RequirementAudit> = inputs
            .requirements
            .iter()
            .map(Self::audit_requirement)
            .collect();

        // A goal with no requirements is never complete: completion must rest on
        // requirement-level observed evidence, never on a bare goal-level claim.
        if requirements.is_empty() {
            return AuditDecision {
                verdict: AuditVerdict::Incomplete,
                reason: "no_requirements",
                requirements,
            };
        }

        // The reason names the FIRST not-complete requirement. Hard blockers
        // (blocked/contradicted) take precedence so an unsafe requirement surfaces
        // first; then the softer not-complete states in their first-seen order.
        if let Some(blocker) = requirements.iter().find(|r| r.state.is_blocking()) {
            return AuditDecision {
                verdict: AuditVerdict::Incomplete,
                reason: blocker.reason,
                requirements,
            };
        }
        if let Some(incomplete) = requirements.iter().find(|r| !r.state.counts_complete()) {
            return AuditDecision {
                verdict: AuditVerdict::Incomplete,
                reason: incomplete.reason,
                requirements,
            };
        }

        // Every requirement is an observed validated/reviewed: complete.
        AuditDecision {
            verdict: AuditVerdict::Complete,
            reason: "all_requirements_met",
            requirements,
        }
    }
}

impl FakeBoundaryController {
    /// GA5 (GO9): evaluate the completion verdict for a goal, PURELY (read-only).
    ///
    /// Reads the OBSERVED state the auditor needs from persisted goal state -- the
    /// requirement ledger, the goal-report story (to find observed-evidence rows
    /// vs agent claims), and the task/session observed `EvidenceProjection` rows --
    /// folds it into [`AuditInputs`], and returns the pure [`CompletionAuditor::
    /// audit`] decision. This appends NOTHING; it is the read-only evaluation used
    /// to answer "is this goal complete?" before recording.
    ///
    /// The observed-evidence check per requirement is the GO9 gate: a requirement
    /// is "backed by observed evidence" iff there is an observed-source goal-report
    /// row OR an `EvidenceProjection` row for the goal's task. An `agent_reported`
    /// claim -- including a `capo.complete_requirement` report -- never counts as
    /// observed evidence, so it cannot satisfy the gate on its own.
    pub fn audit_goal_completion(&self, goal_id: &GoalId) -> StateResult<AuditDecision> {
        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;
        let inputs = self.build_audit_inputs(&goal)?;
        Ok(CompletionAuditor::audit(&inputs))
    }

    /// Build the pure auditor inputs from persisted goal state.
    fn build_audit_inputs(&self, goal: &GoalProjection) -> StateResult<AuditInputs> {
        let requirements = self.state.requirement_ledgers_for_goal(&goal.goal_id)?;
        let reports = self.state.goal_reports_for_goal(&goal.goal_id)?;

        // Observed evidence spans EVERY attempt session bound to the goal's task
        // (a continuation rebinds the goal to a fresh attempt session, so a
        // session-scoped read would drop prior-attempt observed evidence). The task
        // id is the stable cross-attempt key; fall back to the bound session only
        // for a task-less goal.
        let evidence: Vec<EvidenceProjection> = match goal.task_id.as_ref() {
            Some(task_id) => self.state.evidence_for_task(task_id)?,
            None => match goal.session_id.as_ref() {
                Some(session_id) => self.state.evidence_for_session(session_id)?,
                None => Vec::new(),
            },
        };

        // Does the goal have ANY observed evidence at all (task-scoped)? A
        // requirement-tagged observed report binds evidence to a specific
        // requirement; an untagged observed evidence row backs the goal broadly.
        let goal_has_observed_evidence = !evidence.is_empty()
            || reports
                .iter()
                .any(GoalReportProjection::is_observed_evidence);

        let inputs = requirements
            .iter()
            .map(|requirement| {
                let observed_evidence = requirement_has_observed_evidence(
                    requirement,
                    &reports,
                    goal_has_observed_evidence,
                );
                RequirementInput {
                    requirement_id: requirement.requirement_id.clone(),
                    ledger_status: requirement.status.clone(),
                    observed_evidence,
                    weak_validation: requirement_has_weak_validation(requirement, &reports),
                }
            })
            .collect();

        Ok(AuditInputs {
            requirements: inputs,
        })
    }

    /// GA5 (GO9): evaluate AND durably record the completion verdict.
    ///
    /// Calls [`Self::audit_goal_completion`], then records the verdict through a
    /// `goal.audit_decision_recorded` event + [`GoalAuditDecisionProjection`] so
    /// the "why is (or isn't) this goal complete?" answer is a derived read model
    /// rather than hand-written prose. This is the ONLY path that records a Capo
    /// goal-complete verdict; an agent claim or a provider-native completion never
    /// reaches it.
    ///
    /// Recording is idempotent on `(goal, audit_id)`: the caller supplies a stable
    /// `audit_id`, so a verbatim re-audit re-records nothing.
    pub fn audit_and_record_goal_completion(
        &self,
        goal_id: &GoalId,
        audit_id: &str,
    ) -> StateResult<AuditDecision> {
        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;
        let inputs = self.build_audit_inputs(&goal)?;
        let decision = CompletionAuditor::audit(&inputs);

        let requirement_detail_json = serde_json::Value::Array(
            decision
                .requirements
                .iter()
                .map(|requirement| {
                    serde_json::json!({
                        "requirement_id": requirement.requirement_id.as_str(),
                        "state": requirement.state.as_str(),
                        "observed_evidence": requirement.observed_evidence,
                        "reason": requirement.reason,
                    })
                })
                .collect(),
        )
        .to_string();

        let requirements_total = decision.requirements.len() as i64;
        let requirements_complete = decision.requirements_complete() as i64;

        let projection = GoalAuditDecisionProjection {
            audit_id: audit_id.to_string(),
            goal_id: goal_id.clone(),
            project_id: self.project_id.clone(),
            attempt_run_id: goal.attempt_run_id.clone(),
            verdict: decision.verdict.as_str().to_string(),
            reason: decision.reason.to_string(),
            requirements_total,
            requirements_complete,
            requirement_detail_json: requirement_detail_json.clone(),
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "audit_id": audit_id,
            "goal_id": goal_id.as_str(),
            "verdict": decision.verdict.as_str(),
            "reason": decision.reason,
            "requirements_total": requirements_total,
            "requirements_complete": requirements_complete,
            "requirement_detail": requirement_detail_json,
            "attempt_run_id": goal.attempt_run_id.as_ref().map(RunId::as_str),
        })
        .to_string();

        let event = NewEvent {
            event_id: format!("event-audit-{audit_id}"),
            kind: EventKind::GoalAuditDecisionRecorded,
            actor: "capo-controller-auditor".to_string(),
            project_id: Some(self.project_id.clone()),
            task_id: goal.task_id.clone(),
            agent_id: goal.agent_id.clone(),
            session_id: goal.session_id.clone(),
            run_id: goal.attempt_run_id.clone(),
            turn_id: None,
            item_id: Some(audit_id.to_string()),
            payload_json: payload,
            idempotency_key: Some(format!("audit:{}:{}", goal_id.as_str(), audit_id)),
            redaction_state: RedactionState::Safe,
        };

        self.state
            .append_event(event, &[ProjectionRecord::GoalAuditDecision(projection)])?;

        Ok(decision)
    }
}

/// Whether a requirement is backed by CONCRETE OBSERVED EVIDENCE (GO9 gate).
///
/// A requirement is observed-backed iff there is an observed-source goal-report row
/// tagged to that requirement, OR an `EvidenceProjection` row recorded for the
/// goal's task while this requirement is being audited. Because evidence rows are
/// task-scoped (not requirement-scoped) in GA1, an untagged observed evidence row
/// backs any requirement of the goal; a requirement-tagged observed report binds it
/// precisely. An `agent_reported` claim NEVER counts here -- that is the whole point
/// of the gate.
fn requirement_has_observed_evidence(
    requirement: &RequirementLedgerProjection,
    reports: &[GoalReportProjection],
    goal_has_observed_evidence: bool,
) -> bool {
    // A requirement-tagged observed report is the most precise binding.
    let observed_report_for_requirement = reports.iter().any(|report| {
        report.is_observed_evidence()
            && report.requirement_id.as_ref() == Some(&requirement.requirement_id)
    });
    if observed_report_for_requirement {
        return true;
    }
    // Otherwise the requirement is backed iff the goal has ANY task-scoped observed
    // evidence (an `EvidenceProjection` row or an untagged observed report). This is
    // deliberately conservative: the auditor still requires the requirement to reach
    // a satisfying ledger status, and GA2 already forbids a `validated`/`reviewed`
    // ledger status from an `agent_reported` source, so the satisfying status itself
    // cannot have come from a claim.
    goal_has_observed_evidence
}

/// Whether a requirement carries an explicitly weak or skipped validation (GO9
/// "record skipped or weak validation explicitly"). Read from an observed
/// validation report tagged to the requirement whose summary marks it weak/skipped.
fn requirement_has_weak_validation(
    requirement: &RequirementLedgerProjection,
    reports: &[GoalReportProjection],
) -> bool {
    reports.iter().any(|report| {
        report.requirement_id.as_ref() == Some(&requirement.requirement_id)
            && report.report_kind == "capo.record_validation"
            && is_weak_validation_summary(&report.summary)
    })
}

/// A validation summary is weak/skipped when it explicitly says so. Kept as a
/// narrow, explicit check so a weak validation is never silently treated as a pass.
fn is_weak_validation_summary(summary: &str) -> bool {
    let lower = summary.to_ascii_lowercase();
    lower.contains("weak") || lower.contains("skipped") || lower.contains("skip")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_state::{
        EvidenceProjection, GoalProjection, NewEvent, ProjectionRecord, RequirementLedgerProjection,
    };

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-ga5-{name}-{nanos}-{n}"))
    }

    const PROJECT: &str = "project-capo";
    const GOAL: &str = "goal-ga5";
    const TASK: &str = "task-ga5";
    const SESSION: &str = "session-ga5";
    const RUN: &str = "run-ga5";

    fn open() -> (FakeBoundaryController, PathBuf) {
        let state_root = temp_root("state");
        let controller =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("controller");
        (controller, state_root)
    }

    // ----- Pure auditor branch coverage (no DB) -----------------------------

    fn requirement(status: &str, observed_evidence: bool) -> RequirementInput {
        RequirementInput {
            requirement_id: RequirementId::new(format!("req-{status}-{observed_evidence}")),
            ledger_status: status.to_string(),
            observed_evidence,
            weak_validation: false,
        }
    }

    #[test]
    fn pure_auditor_completes_when_every_requirement_is_observed_validated_or_reviewed() {
        let inputs = AuditInputs {
            requirements: vec![
                requirement(RequirementLedgerProjection::VALIDATED, true),
                requirement(RequirementLedgerProjection::REVIEWED, true),
            ],
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Complete);
        assert_eq!(decision.reason, "all_requirements_met");
        assert_eq!(decision.requirements_complete(), 2);
        assert_eq!(
            decision.requirements[0].state,
            RequirementAuditState::Validated
        );
        assert_eq!(
            decision.requirements[1].state,
            RequirementAuditState::Reviewed
        );
    }

    #[test]
    fn pure_auditor_overclaim_validated_without_observed_evidence_is_claim_only() {
        // A `validated` ledger status with NO observed evidence is a PROPOSAL only:
        // the overclaim guard downgrades it to `claim_only` and the goal stays
        // incomplete. Global confidence never substitutes for requirement evidence.
        let inputs = AuditInputs {
            requirements: vec![requirement(RequirementLedgerProjection::VALIDATED, false)],
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "requirement_claim_only");
        assert_eq!(
            decision.requirements[0].state,
            RequirementAuditState::ClaimOnly
        );
        assert_eq!(decision.requirements_complete(), 0);
    }

    #[test]
    fn pure_auditor_weak_validation_does_not_complete() {
        let inputs = AuditInputs {
            requirements: vec![RequirementInput {
                weak_validation: true,
                ..requirement(RequirementLedgerProjection::VALIDATED, true)
            }],
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "requirement_weak_validation");
        assert_eq!(decision.requirements[0].state, RequirementAuditState::Weak);
    }

    #[test]
    fn pure_auditor_blocked_and_contradicted_block_the_goal() {
        for (status, expected_state, expected_reason) in [
            (
                RequirementLedgerProjection::BLOCKED,
                RequirementAuditState::Blocked,
                "requirement_blocked",
            ),
            (
                RequirementLedgerProjection::CONTRADICTED,
                RequirementAuditState::Contradicted,
                "requirement_contradicted",
            ),
        ] {
            // A hard blocker outranks an otherwise-complete requirement so the
            // verdict points at the unsafe requirement first.
            let inputs = AuditInputs {
                requirements: vec![
                    requirement(RequirementLedgerProjection::VALIDATED, true),
                    requirement(status, true),
                ],
            };
            let decision = CompletionAuditor::audit(&inputs);
            assert_eq!(decision.verdict, AuditVerdict::Incomplete);
            assert_eq!(decision.reason, expected_reason);
            assert_eq!(decision.requirements[1].state, expected_state);
        }
    }

    #[test]
    fn pure_auditor_partial_supported_and_unverified_are_incomplete() {
        let inputs = AuditInputs {
            requirements: vec![
                requirement(RequirementLedgerProjection::VALIDATED, true),
                requirement(RequirementLedgerProjection::SUPPORTED, true),
                requirement(RequirementLedgerProjection::UNVERIFIED, false),
            ],
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        // The first not-complete requirement is the `supported` one.
        assert_eq!(decision.reason, "requirement_supported_only");
        assert_eq!(
            decision.requirements[1].state,
            RequirementAuditState::Supported
        );
        assert_eq!(
            decision.requirements[2].state,
            RequirementAuditState::Unverified
        );
        assert_eq!(decision.requirements_complete(), 1);
    }

    #[test]
    fn pure_auditor_supported_without_observed_evidence_is_claim_only() {
        let inputs = AuditInputs {
            requirements: vec![requirement(RequirementLedgerProjection::SUPPORTED, false)],
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "requirement_claim_only");
        assert_eq!(
            decision.requirements[0].state,
            RequirementAuditState::ClaimOnly
        );
    }

    #[test]
    fn pure_auditor_no_requirements_is_never_complete() {
        let inputs = AuditInputs {
            requirements: Vec::new(),
        };
        let decision = CompletionAuditor::audit(&inputs);
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "no_requirements");
    }

    // ----- Controller wiring over persisted state ----------------------------

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

    fn goal_projection() -> GoalProjection {
        GoalProjection {
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new(TASK)),
            agent_id: None,
            session_id: Some(SessionId::new(SESSION)),
            parent_goal_id: None,
            attempt_run_id: Some(RunId::new(RUN)),
            objective: "Drive the GA5 auditor".to_string(),
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

    fn requirement_ledger(req_id: &str, status: &str, source: &str) -> RequirementLedgerProjection {
        RequirementLedgerProjection {
            requirement_id: RequirementId::new(req_id),
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            summary: format!("requirement {req_id}"),
            status: status.to_string(),
            last_status_source: source.to_string(),
            updated_sequence: 0,
        }
    }

    fn observed_evidence(evidence_id: &str) -> EvidenceProjection {
        EvidenceProjection {
            evidence_id: EvidenceId::new(evidence_id),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new(TASK)),
            session_id: Some(SessionId::new(SESSION)),
            run_id: Some(RunId::new(RUN)),
            kind: "test".to_string(),
            artifact_id: None,
            confidence: 90,
            updated_sequence: 0,
        }
    }

    fn agent_claim_report(report_id: &str, req_id: &str) -> GoalReportProjection {
        GoalReportProjection {
            goal_report_id: report_id.to_string(),
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            session_id: Some(SessionId::new(SESSION)),
            requirement_id: Some(RequirementId::new(req_id)),
            report_kind: "capo.complete_requirement".to_string(),
            // The load-bearing tag: an agent CLAIM, never observed evidence.
            source: "agent_reported".to_string(),
            confidence: Some(99),
            summary: "I completed the requirement".to_string(),
            body_artifact_id: None,
            evidence_id: None,
            updated_sequence: 0,
        }
    }

    #[test]
    fn agent_reported_completion_alone_does_not_transition_goal_to_complete() {
        // premature-completion-blocked: a `validated` requirement whose ONLY
        // support is a high-confidence `capo.complete_requirement` agent claim --
        // and NO observed evidence -- never completes the goal. (Note: this seeds a
        // `validated` ledger status directly to model the worst case; in production
        // GA2 already forbids a `validated` status from an `agent_reported` source,
        // so the auditor is defense-in-depth on top of that.)
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-claim-only",
            &[
                ProjectionRecord::Goal(goal_projection()),
                ProjectionRecord::RequirementLedger(requirement_ledger(
                    "req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "agent_reported",
                )),
                ProjectionRecord::GoalReport(agent_claim_report("report-claim", "req-1")),
            ],
        );

        let decision = controller
            .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-1")
            .expect("audit");
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "requirement_claim_only");

        // The verdict is recorded as a derived read model, and the latest verdict
        // is NOT complete.
        let latest = controller
            .state()
            .latest_goal_audit_decision(&GoalId::new(GOAL))
            .expect("latest")
            .expect("decision present");
        assert!(!latest.is_complete());
        assert_eq!(latest.verdict, GoalAuditDecisionProjection::INCOMPLETE);
        assert_eq!(latest.requirements_total, 1);
        assert_eq!(latest.requirements_complete, 0);
        assert!(latest.requirement_detail_json.contains("claim_only"));
    }

    #[test]
    fn requirement_with_observed_evidence_and_validation_transitions_to_complete() {
        // complete-with-evidence: a `validated` requirement backed by concrete
        // observed `EvidenceProjection` evidence DOES complete the goal.
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-complete",
            &[
                ProjectionRecord::Goal(goal_projection()),
                ProjectionRecord::RequirementLedger(requirement_ledger(
                    "req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "runtime_output",
                )),
                ProjectionRecord::Evidence(observed_evidence("evidence-check-1")),
            ],
        );

        let decision = controller
            .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-complete")
            .expect("audit");
        assert_eq!(decision.verdict, AuditVerdict::Complete);
        assert_eq!(decision.reason, "all_requirements_met");

        let latest = controller
            .state()
            .latest_goal_audit_decision(&GoalId::new(GOAL))
            .expect("latest")
            .expect("decision present");
        assert!(latest.is_complete());
        assert_eq!(latest.requirements_total, 1);
        assert_eq!(latest.requirements_complete, 1);
        assert_eq!(latest.attempt_run_id, Some(RunId::new(RUN)));
    }

    #[test]
    fn auditor_blocks_a_blocked_requirement_even_with_observed_evidence() {
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-blocked",
            &[
                ProjectionRecord::Goal(goal_projection()),
                ProjectionRecord::RequirementLedger(requirement_ledger(
                    "req-1",
                    RequirementLedgerProjection::BLOCKED,
                    "runtime_output",
                )),
                ProjectionRecord::Evidence(observed_evidence("evidence-check-1")),
            ],
        );

        let decision = controller
            .audit_goal_completion(&GoalId::new(GOAL))
            .expect("audit");
        assert_eq!(decision.verdict, AuditVerdict::Incomplete);
        assert_eq!(decision.reason, "requirement_blocked");
    }

    #[test]
    fn auditor_recording_is_idempotent_on_audit_id() {
        let (controller, _root) = open();
        seed(
            &controller,
            "seed-idempotent",
            &[
                ProjectionRecord::Goal(goal_projection()),
                ProjectionRecord::RequirementLedger(requirement_ledger(
                    "req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "runtime_output",
                )),
                ProjectionRecord::Evidence(observed_evidence("evidence-check-1")),
            ],
        );

        for _ in 0..2 {
            let _ = controller
                .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-dup")
                .expect("audit");
        }
        let recorded = controller
            .state()
            .goal_audit_decisions_for_goal(&GoalId::new(GOAL))
            .expect("audits");
        assert_eq!(recorded.len(), 1, "verbatim re-audit re-records nothing");
    }

    #[test]
    fn auditor_verdict_survives_restart_and_projection_rebuild() {
        let state_root = temp_root("restart");
        {
            let controller = FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root)
                .expect("controller");
            seed(
                &controller,
                "seed-restart",
                &[
                    ProjectionRecord::Goal(goal_projection()),
                    ProjectionRecord::RequirementLedger(requirement_ledger(
                        "req-1",
                        RequirementLedgerProjection::REVIEWED,
                        "runtime_output",
                    )),
                    ProjectionRecord::Evidence(observed_evidence("evidence-check-1")),
                ],
            );
            let decision = controller
                .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-restart")
                .expect("audit");
            assert_eq!(decision.verdict, AuditVerdict::Complete);
        }

        // Re-open over the same state root and rebuild projections from the event
        // log: the auditor verdict rebuilds identically.
        let reopened =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("reopen");
        reopened.state().rebuild_projections().expect("rebuild");
        let latest = reopened
            .state()
            .latest_goal_audit_decision(&GoalId::new(GOAL))
            .expect("latest")
            .expect("decision present");
        assert!(latest.is_complete());
        assert_eq!(latest.verdict, GoalAuditDecisionProjection::COMPLETE);
        assert_eq!(latest.reason, "all_requirements_met");
        assert_eq!(latest.requirements_complete, 1);
    }

    #[test]
    fn observed_report_tagged_to_requirement_backs_completion() {
        // An OBSERVED goal-report row (source=runtime_output) tagged to the
        // requirement is concrete observed evidence -- it backs completion exactly
        // like an `EvidenceProjection` row.
        let (controller, _root) = open();
        let observed_report = GoalReportProjection {
            goal_report_id: "report-observed".to_string(),
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            session_id: Some(SessionId::new(SESSION)),
            requirement_id: Some(RequirementId::new("req-1")),
            report_kind: "runtime_output".to_string(),
            source: "runtime_output".to_string(),
            confidence: None,
            summary: "observed check passed".to_string(),
            body_artifact_id: None,
            evidence_id: None,
            updated_sequence: 0,
        };
        seed(
            &controller,
            "seed-observed-report",
            &[
                ProjectionRecord::Goal(goal_projection()),
                ProjectionRecord::RequirementLedger(requirement_ledger(
                    "req-1",
                    RequirementLedgerProjection::VALIDATED,
                    "runtime_output",
                )),
                ProjectionRecord::GoalReport(observed_report),
            ],
        );

        let decision = controller
            .audit_goal_completion(&GoalId::new(GOAL))
            .expect("audit");
        assert_eq!(decision.verdict, AuditVerdict::Complete);
        assert!(decision.requirements[0].observed_evidence);
    }
}
