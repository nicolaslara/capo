//! GO10: the ONE historical-goal-report renderer.
//!
//! This is the single source of truth for the GO10 historical execution report.
//! Both the operator-facing server surface
//! (`capo_server::CapoServer::handle_goal_report_rendering`) and the
//! deterministic goal-autonomy e2e render through THIS code, so the report the
//! e2e snapshots is byte-for-byte the report the operator ships -- the two cannot
//! drift into divergent definitions of "the historical report".
//!
//! It lives in `capo-state` because that is the crate both `capo-server` and
//! `capo-controller` already depend on, and because every input is a persisted
//! read-model projection plus the event timeline. The report is therefore
//! rebuildable from events + projections (GO10): re-render after a projection
//! rebuild and the bytes are identical.
//!
//! Safety/privacy (GO10): only projected read-model facts are rendered. A raw
//! report/provider body is named by its artifact id, NEVER inlined; a redacted
//! timeline event is shown as a redacted reference and flips the `degraded` flag;
//! an empty timeline also degrades. A `complete` verdict is only ever the
//! auditor's (it arrives here as a [`GoalAuditDecisionProjection`]), never a
//! lifecycle write.

use crate::event::EventRecord;
use crate::projections::{
    DelegatedProviderGoalProjection, EvidenceProjection, GoalAuditDecisionProjection,
    GoalContinuationProjection, GoalProjection, GoalReportProjection, RequirementLedgerProjection,
};

/// Whether a `source` tag denotes OBSERVED evidence (runtime output or an adapter
/// event) rather than an agent-submitted claim. Adapter-event sources may be
/// sub-tagged (`adapter_event:<adapter>`), so this matches the prefix.
///
/// This is the capo-state-side mirror of `capo_tools::source_is_observed_evidence`;
/// the doc comment on [`GoalReportProjection::is_observed_evidence`] notes the two
/// must classify a source identically, and the projection helpers below delegate
/// here so there is one classifier inside this crate.
pub fn source_is_observed_evidence(source: &str) -> bool {
    source == "runtime_output" || source == "adapter_event" || source.starts_with("adapter_event:")
}

/// Everything the GO10 report renders, gathered from the persisted projections
/// plus the goal's event timeline. Held by reference: the caller owns the rows it
/// read out of the store, and the renderer only borrows them.
pub struct GoalReportInputs<'a> {
    pub goal: &'a GoalProjection,
    pub requirements: &'a [RequirementLedgerProjection],
    /// The goal's report ("story") rows, oldest first.
    pub reports: &'a [GoalReportProjection],
    pub continuations: &'a [GoalContinuationProjection],
    pub delegated_provider_goals: &'a [DelegatedProviderGoalProjection],
    /// The goal's observed evidence (by its stable task key, cross-attempt).
    pub evidence: &'a [EvidenceProjection],
    /// The auditor's latest verdict, when the goal has been audited. Completion is
    /// only ever the auditor's, so the verdict is surfaced here as observed-fact.
    pub audit: Option<&'a GoalAuditDecisionProjection>,
    /// The goal's event timeline, in sequence order.
    pub timeline: &'a [EventRecord],
}

impl GoalReportInputs<'_> {
    fn requirement_counts(&self) -> RequirementCounts {
        let mut counts = RequirementCounts::default();
        for requirement in self.requirements {
            match requirement.status.as_str() {
                RequirementLedgerProjection::SUPPORTED
                | RequirementLedgerProjection::VALIDATED
                | RequirementLedgerProjection::REVIEWED => counts.supported += 1,
                RequirementLedgerProjection::BLOCKED => counts.blocked += 1,
                RequirementLedgerProjection::CONTRADICTED => counts.contradicted += 1,
                _ => {}
            }
        }
        counts
    }
}

#[derive(Default)]
struct RequirementCounts {
    supported: usize,
    blocked: usize,
    contradicted: usize,
}

/// The rendered GO10 report: the body text plus the `degraded` flag (set when a
/// referenced artifact was missing/redacted, or the timeline was empty, so a clear
/// placeholder rendered rather than raw content).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedGoalReport {
    pub body: String,
    pub degraded: bool,
}

/// GO10: render the historical execution report as markdown. Rebuildable from the
/// projections + timeline; degrades clearly when a referenced artifact is absent
/// (named as a missing reference, never rendered as raw content -- raw provider
/// transcripts never appear here).
pub fn render_goal_report_markdown(inputs: &GoalReportInputs<'_>) -> RenderedGoalReport {
    let goal = inputs.goal;
    let counts = inputs.requirement_counts();
    let mut degraded = false;
    let mut body = String::new();

    body.push_str(&format!("# Goal report: {}\n\n", goal.goal_id.as_str()));
    body.push_str(&format!("- Objective: {}\n", goal.objective));
    body.push_str(&format!("- Status: {}\n", goal.status));
    if let Some(parent) = goal.parent_goal_id.as_ref() {
        body.push_str(&format!("- Parent goal: {}\n", parent.as_str()));
    }
    body.push_str(&format!(
        "- Requirements: {} ({} supported, {} blocked, {} contradicted)\n",
        inputs.requirements.len(),
        counts.supported,
        counts.blocked,
        counts.contradicted,
    ));
    if !goal.blocker_reason.is_empty() {
        body.push_str(&format!("- Current blocker: {}\n", goal.blocker_reason));
    }

    // The auditor's verdict (GO9). Completion is only ever the auditor's, so the
    // report surfaces the verdict as observed-fact, never a lifecycle claim.
    match inputs.audit {
        Some(audit) => body.push_str(&format!(
            "- Verdict: {} ({}) — {}/{} requirements complete\n",
            audit.verdict, audit.reason, audit.requirements_complete, audit.requirements_total,
        )),
        None => body.push_str("- Verdict: not yet audited\n"),
    }

    body.push_str("\n## Requirements\n\n");
    if inputs.requirements.is_empty() {
        body.push_str("_No requirements recorded._\n");
    }
    for requirement in inputs.requirements {
        body.push_str(&format!(
            "- [{}] {} ({}) — source: {} ({})\n",
            requirement.status,
            requirement.summary,
            requirement.requirement_id.as_str(),
            requirement.last_status_source,
            if source_is_observed_evidence(&requirement.last_status_source) {
                "observed"
            } else {
                "reported"
            },
        ));
    }

    body.push_str("\n## Story\n\n");
    if inputs.reports.is_empty() {
        body.push_str("_No reports recorded._\n");
    }
    for report in inputs.reports {
        let provenance = if report.is_observed_evidence() {
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
        if let Some(artifact) = report.body_artifact_id.as_ref() {
            // GO10 degradation: the raw body lives in an artifact, referenced by
            // id, never inlined. We name it; we never render its raw content.
            body.push_str(&format!("    - body artifact: {artifact}\n"));
        }
    }

    body.push_str("\n## Observed evidence\n\n");
    if inputs.evidence.is_empty() {
        body.push_str("_No observed evidence recorded._\n");
    }
    for row in inputs.evidence {
        body.push_str(&format!(
            "- {} (kind: {}, confidence: {})\n",
            row.evidence_id.as_str(),
            row.kind,
            row.confidence,
        ));
    }

    if !inputs.continuations.is_empty() {
        body.push_str("\n## Continuation decisions\n\n");
        for continuation in inputs.continuations {
            body.push_str(&format!(
                "- {} — {}\n",
                continuation.decision, continuation.reason
            ));
        }
    }

    if !inputs.delegated_provider_goals.is_empty() {
        body.push_str("\n## Delegated provider goals (observed, not authoritative)\n\n");
        for delegated in inputs.delegated_provider_goals {
            body.push_str(&format!(
                "- {}: {} [{}]\n",
                delegated.provider_kind, delegated.provider_state, delegated.source
            ));
        }
    }

    body.push_str("\n## Timeline\n\n");
    if inputs.timeline.is_empty() {
        body.push_str("_No events recorded._\n");
        degraded = true;
    }
    for record in inputs.timeline {
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

    RenderedGoalReport { body, degraded }
}

/// GO10: render the historical report as JSON. Same derived data as the markdown
/// render; raw artifact bodies are referenced by id, never inlined, and redacted
/// events surface their redaction state.
pub fn render_goal_report_json(inputs: &GoalReportInputs<'_>) -> RenderedGoalReport {
    let goal = inputs.goal;
    let counts = inputs.requirement_counts();
    let mut degraded = inputs.timeline.is_empty();
    let timeline_json: Vec<serde_json::Value> = inputs
        .timeline
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
        "goal_id": goal.goal_id.as_str(),
        "objective": goal.objective,
        "status": goal.status,
        "parent_goal_id": goal.parent_goal_id.as_ref().map(|id| id.as_str()),
        "attempt_run_id": goal.attempt_run_id.as_ref().map(|id| id.as_str()),
        "blocker_reason": goal.blocker_reason,
        "success_criteria_json": goal.success_criteria_json,
        "constraints_json": goal.constraints_json,
        "verification_surface_json": goal.verification_surface_json,
        "budget_json": goal.budget_json,
        "stop_conditions_json": goal.stop_conditions_json,
        "verdict": inputs.audit.map(|audit| serde_json::json!({
            "verdict": audit.verdict,
            "reason": audit.reason,
            "requirements_total": audit.requirements_total,
            "requirements_complete": audit.requirements_complete,
        })),
        "requirement_counts": {
            "total": inputs.requirements.len(),
            "supported": counts.supported,
            "blocked": counts.blocked,
            "contradicted": counts.contradicted,
        },
        "requirements": inputs.requirements.iter().map(|requirement| serde_json::json!({
            "requirement_id": requirement.requirement_id.as_str(),
            "summary": requirement.summary,
            "status": requirement.status,
            "last_status_source": requirement.last_status_source,
            "observed": source_is_observed_evidence(&requirement.last_status_source),
        })).collect::<Vec<_>>(),
        "reports": inputs.reports.iter().map(|report| serde_json::json!({
            "goal_report_id": report.goal_report_id,
            "requirement_id": report.requirement_id.as_ref().map(|id| id.as_str()),
            "report_kind": report.report_kind,
            "source": report.source,
            "observed": report.is_observed_evidence(),
            "confidence": report.confidence,
            "summary": report.summary,
            "body_artifact_id": report.body_artifact_id,
            "evidence_id": report.evidence_id.as_ref().map(|id| id.as_str()),
        })).collect::<Vec<_>>(),
        "observed_evidence": inputs.evidence.iter().map(|row| serde_json::json!({
            "evidence_id": row.evidence_id.as_str(),
            "kind": row.kind,
            "confidence": row.confidence,
        })).collect::<Vec<_>>(),
        "continuations": inputs.continuations.iter().map(|continuation| serde_json::json!({
            "continuation_id": continuation.continuation_id,
            "decision": continuation.decision,
            "reason": continuation.reason,
        })).collect::<Vec<_>>(),
        "delegated_provider_goals": inputs.delegated_provider_goals.iter().map(|delegated| serde_json::json!({
            "delegated_goal_id": delegated.delegated_goal_id,
            "provider_kind": delegated.provider_kind,
            "provider_state": delegated.provider_state,
            "source": delegated.source,
        })).collect::<Vec<_>>(),
        "timeline": timeline_json,
        "degraded": degraded,
    });
    RenderedGoalReport {
        body: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        degraded,
    }
}
