//! GA3 (goal-orchestration GO7): the sourced continuation context packet and
//! continuation prompt assembly.
//!
//! What this is and where it lives (the GO7 boundary): the continuation context
//! packet is assembled HERE, on the controller/server side, as INPUT to the real
//! turn loop. It is a derived, read-only view over persisted goal state; it is
//! NOT an authoritative read model and is never persisted as one. Nothing in this
//! module appends an event or projects a record -- it only READS the GA1/GA2 goal
//! projections (objective, requirements, reports, evidence, blockers,
//! validation/review state, continuation decisions) plus the goal's memory packets
//! and review findings, and folds them into one bounded, explainable packet.
//!
//! Cross-attempt survival (GO7): observed evidence, review findings, and memory
//! packets are read by the goal's TASK id, not its currently-bound session id. A
//! continuation rebinds the goal to a fresh attempt session (`goals.session_id` is
//! mutable), so a session-scoped read would silently drop every prior-attempt
//! observation -- the task id is the stable key that spans all attempt sessions. A
//! task-less goal falls back to its bound session.
//!
//! The load-bearing GO7 property -- survives restart / compaction / transcript
//! loss: the active objective and the audit contract are reconstructed STRICTLY
//! from persisted goal state (the `goals` / `requirement_ledgers` projections),
//! never from a model transcript. After a server restart and a full projection
//! rebuild, re-assembling the packet from the rebuilt state yields the SAME
//! objective and audit contract, because both live in the event log and
//! projections rather than in any in-memory conversation. This directly addresses
//! the observed compaction-related goal loss: the objective does not depend on the
//! provider keeping it in context.
//!
//! Provenance and bounding (GO7, consistent with `memory-architecture.md`): every
//! injected fragment carries a `source_ref` (where it came from -- a goal id, a
//! requirement id, a report id, an evidence id, a memory packet id, a review
//! finding id, or a workpad ref) AND a `content_hash` (a stable FNV-1a digest of
//! the fragment's serialized summary), so the packet is explainable and
//! provenance is queryable. Assembly is bounded by explicit selection limits
//! ([`ContinuationContextLimits`]): the newest N reports, the newest N observed
//! evidence rows, the newest N memory packets, etc. The packet carries SUMMARIES
//! and REFERENCES only -- it never dumps whole files or raw transcripts. A
//! referenced report/evidence body is named by its artifact id with the artifact's
//! content hash and redaction state; the body itself is never inlined, and a
//! non-`safe` artifact is recorded as a redacted reference.

use capo_state::{
    DelegatedProviderGoalProjection, EvidenceProjection, GoalContinuationProjection,
    GoalProjection, GoalReportProjection, MemoryPacketProjection, RequirementLedgerProjection,
    ReviewFindingProjection,
};

use super::*;

/// GA3 (GO7): the explicit selection/size limits that keep packet assembly
/// bounded. Each limit caps how many of the newest rows of a given kind are
/// folded into the packet, so the packet never grows with unbounded history and
/// never dumps a whole ledger. The defaults are deliberately small; a caller may
/// tune them, but assembly is ALWAYS bounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContinuationContextLimits {
    /// Max agent-report / story rows (newest first).
    pub max_reports: usize,
    /// Max observed-evidence rows (newest first).
    pub max_evidence: usize,
    /// Max review-finding rows (newest first).
    pub max_reviews: usize,
    /// Max memory-packet references (newest first).
    pub max_memory_packets: usize,
    /// Max continuation-decision rows (newest first).
    pub max_continuations: usize,
    /// Max length of any single fragment summary, in chars. A longer summary is
    /// truncated with an explicit ellipsis so no fragment inlines a whole body.
    pub max_summary_chars: usize,
}

impl Default for ContinuationContextLimits {
    fn default() -> Self {
        Self {
            max_reports: 5,
            max_evidence: 5,
            max_reviews: 5,
            max_memory_packets: 5,
            max_continuations: 3,
            max_summary_chars: 240,
        }
    }
}

/// GA3 (GO7): the class of state a fragment was sourced from. Recorded on every
/// fragment so the packet's provenance is queryable by kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationSourceKind {
    /// The active goal's objective / lifecycle state.
    Goal,
    /// A requirement's audit-contract entry.
    Requirement,
    /// An agent-reported claim (a proposal, never observed evidence).
    AgentReport,
    /// Observed evidence (`runtime_output` / `adapter_event`).
    ObservedEvidence,
    /// A recorded review finding.
    ReviewFinding,
    /// A continuation decision.
    ContinuationDecision,
    /// A delegated-provider goal observation (observed, not authoritative).
    DelegatedProviderGoal,
    /// A memory-packet reference.
    MemoryPacket,
    /// A workpad / source reference.
    WorkpadRef,
}

impl ContinuationSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Requirement => "requirement",
            Self::AgentReport => "agent_report",
            Self::ObservedEvidence => "observed_evidence",
            Self::ReviewFinding => "review_finding",
            Self::ContinuationDecision => "continuation_decision",
            Self::DelegatedProviderGoal => "delegated_provider_goal",
            Self::MemoryPacket => "memory_packet",
            Self::WorkpadRef => "workpad_ref",
        }
    }
}

/// GA3 (GO7): a single sourced fragment of the continuation context packet.
///
/// A fragment carries a short, bounded `summary` plus its full provenance: which
/// `kind` of state it came from, the `source_ref` (the durable id of the row /
/// artifact it was sourced from), and a `content_hash` over the summary so the
/// fragment is explainable and a rebuild reproduces it identically. When the
/// fragment references a stored body it names the `body_artifact_id` (and the
/// artifact's content hash, when known) -- the body itself is NEVER inlined.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationContextFragment {
    pub kind: ContinuationSourceKind,
    /// The durable source id this fragment was derived from (a goal/requirement/
    /// report/evidence/memory-packet/review id, or a workpad ref).
    pub source_ref: String,
    /// A short, bounded human/agent-readable summary of the fragment.
    pub summary: String,
    /// `true` for observed evidence, `false` for an agent-reported claim, and
    /// `None` for fragments that are neither (the objective, a memory packet).
    pub observed: Option<bool>,
    /// The artifact id holding the raw body, when the fragment references one. The
    /// body is named, never inlined.
    pub body_artifact_id: Option<String>,
    /// The referenced artifact's content hash, when the artifact is recorded.
    pub body_content_hash: Option<String>,
    /// `true` when the referenced artifact is NOT `safe` and is therefore included
    /// as a redacted reference only.
    pub redacted: bool,
    /// A stable FNV-1a digest of the fragment's serialized summary + provenance,
    /// so the packet is explainable and rebuilds identically.
    pub content_hash: String,
}

/// GA3 (GO7): the audit contract -- the objective and the per-requirement
/// completion criteria, reconstructed from persisted goal state. This is the part
/// of the packet that MUST survive restart/compaction: it carries no transcript
/// and is rebuilt verbatim from the `goals` / `requirement_ledgers` projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationAuditContract {
    pub goal_id: String,
    pub objective: String,
    /// The goal's lifecycle status (`active` / `paused` / `blocked` / `cleared`).
    /// Never `complete` -- completion is the GA5 auditor's verdict.
    pub status: String,
    pub success_criteria_json: String,
    pub constraints_json: String,
    pub verification_surface_json: String,
    pub stop_conditions_json: String,
    /// The current blocker reason while blocked, else empty.
    pub blocker_reason: String,
    /// The per-requirement audit-contract entries, requirement-id order.
    pub requirements: Vec<ContinuationRequirement>,
    /// A stable digest over the objective + audit contract, so a restart-rebuilt
    /// contract is provably identical (same digest) to the pre-restart one.
    pub content_hash: String,
}

/// GA3 (GO7): one requirement's audit-contract entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationRequirement {
    pub requirement_id: String,
    pub summary: String,
    /// `unverified` / `supported` / `validated` / `reviewed` / `blocked` /
    /// `contradicted`.
    pub status: String,
    /// `true` when the last status was driven by observed evidence, `false` for a
    /// claim. A requirement validated/reviewed by a claim alone is impossible by
    /// construction (the GA2 boundary rejects it); this records provenance.
    pub observed_status: bool,
}

/// GA3 (GO7): the assembled, sourced continuation context packet.
///
/// This is the controller-side INPUT to the real turn loop on a continuation. It
/// is NOT persisted and is never an authoritative read model -- it is a derived
/// view that is cheap to rebuild from the durable goal state at any time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuationContextPacket {
    /// The audit contract (objective + requirements), the transcript-independent
    /// core that survives restart/compaction.
    pub audit_contract: ContinuationAuditContract,
    /// The sourced fragments, in a stable deterministic order.
    pub fragments: Vec<ContinuationContextFragment>,
    /// A stable digest over the whole packet (audit contract + every fragment's
    /// content hash), so the entire packet is explainable and reproducible.
    pub content_hash: String,
}

impl ContinuationContextPacket {
    /// Render the packet into the continuation PROMPT text the loop injects.
    ///
    /// The prompt leads with the reconstructed objective and audit contract (so a
    /// freshly-restarted, compacted, or transcript-lost session is re-grounded in
    /// the goal from persisted state, not from a model transcript), then lists the
    /// bounded sourced fragments. It is deterministic given the packet, so a
    /// rebuild produces the same prompt. It carries summaries and references only;
    /// no raw body or transcript is inlined.
    pub fn render_prompt(&self) -> String {
        let contract = &self.audit_contract;
        let mut out = String::new();
        out.push_str("# Continuation context (reconstructed from persisted goal state)\n\n");
        out.push_str(&format!("Goal: {}\n", contract.goal_id));
        out.push_str(&format!("Objective: {}\n", contract.objective));
        out.push_str(&format!("Status: {}\n", contract.status));
        if !contract.blocker_reason.is_empty() {
            out.push_str(&format!("Current blocker: {}\n", contract.blocker_reason));
        }
        out.push_str("\n## Audit contract (requirements)\n");
        if contract.requirements.is_empty() {
            out.push_str("(no requirements recorded)\n");
        }
        for requirement in &contract.requirements {
            out.push_str(&format!(
                "- [{}] {} ({}) — {}\n",
                requirement.status,
                requirement.summary,
                requirement.requirement_id,
                if requirement.observed_status {
                    "observed"
                } else {
                    "reported"
                },
            ));
        }
        if !self.fragments.is_empty() {
            out.push_str("\n## Sourced context\n");
            for fragment in &self.fragments {
                let provenance = match fragment.observed {
                    Some(true) => " [observed]",
                    Some(false) => " [reported]",
                    None => "",
                };
                let redaction = if fragment.redacted {
                    " [redacted body]"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "- ({}) {}{}{} — src:{}\n",
                    fragment.kind.as_str(),
                    fragment.summary,
                    provenance,
                    redaction,
                    fragment.source_ref,
                ));
            }
        }
        out
    }
}

impl FakeBoundaryController {
    /// GA3 (goal-orchestration GO7): assemble the sourced continuation context
    /// packet for `goal_id` from PERSISTED goal state, with default bounds.
    pub fn continuation_context_packet(
        &self,
        goal_id: &GoalId,
    ) -> StateResult<ContinuationContextPacket> {
        self.continuation_context_packet_with_limits(goal_id, ContinuationContextLimits::default())
    }

    /// GA3 (GO7): assemble the packet with caller-chosen bounds.
    ///
    /// Reconstructs the objective and audit contract from the `goals` /
    /// `requirement_ledgers` projections (no transcript), then folds the bounded
    /// newest reports, observed evidence, review findings, memory packets,
    /// continuation decisions, delegated-provider observations, and the goal's
    /// workpad ref into sourced fragments. Each fragment carries a source ref and a
    /// content hash; referenced bodies are named (artifact id + content hash +
    /// redaction), never inlined.
    pub fn continuation_context_packet_with_limits(
        &self,
        goal_id: &GoalId,
        limits: ContinuationContextLimits,
    ) -> StateResult<ContinuationContextPacket> {
        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;

        let requirements = self.state.requirement_ledgers_for_goal(goal_id)?;
        let audit_contract = build_audit_contract(&goal, &requirements);

        let mut fragments: Vec<ContinuationContextFragment> = Vec::new();

        // Requirements (the audit contract entries) are also surfaced as fragments
        // so the prompt's sourced-context section is self-describing.
        for requirement in &requirements {
            fragments.push(requirement_fragment(requirement, limits.max_summary_chars));
        }

        // Latest agent reports / story rows: newest first, bounded. An
        // agent-reported claim keeps its `observed=false` tag; observed evidence
        // rows keep `observed=true`. A referenced body artifact is named with its
        // content hash and redaction; never inlined.
        let mut reports = self.state.goal_reports_for_goal(goal_id)?;
        reports.reverse();
        for report in reports.iter().take(limits.max_reports) {
            fragments.push(self.report_fragment(report, limits.max_summary_chars)?);
        }

        // Observed evidence, review findings, and memory packets: newest first,
        // bounded. These MUST survive across attempts (GO7): a continuation rebinds
        // the goal to a fresh attempt session (`goals.session_id` is mutable), so a
        // read scoped to the current `goal.session_id` would silently drop every
        // prior-attempt observation -- defeating the whole reason GA3 distinguishes
        // observed evidence from claims. The goal's TASK is the stable cross-attempt
        // key (evidence/reviews/memory all carry `task_id`), so we scope by task
        // when the goal has one and fall back to the current session only for a
        // task-less goal.
        let evidence = match goal.task_id.as_ref() {
            Some(task_id) => self.state.evidence_for_task(task_id)?,
            None => match goal.session_id.as_ref() {
                Some(session_id) => self.state.evidence_for_session(session_id)?,
                None => Vec::new(),
            },
        };
        let reviews = match goal.task_id.as_ref() {
            Some(task_id) => self.state.review_findings_for_task(task_id)?,
            None => match goal.session_id.as_ref() {
                Some(session_id) => self.state.review_findings_for_session(session_id)?,
                None => Vec::new(),
            },
        };
        let memory_packets = match goal.task_id.as_ref() {
            Some(task_id) => self.state.memory_packets_for_task(task_id)?,
            None => match goal.session_id.as_ref() {
                Some(session_id) => self.state.memory_packets_for_session(session_id)?,
                None => Vec::new(),
            },
        };

        // Evidence is always observed.
        let mut evidence = evidence;
        evidence.reverse();
        for row in evidence.iter().take(limits.max_evidence) {
            fragments.push(self.evidence_fragment(row, limits.max_summary_chars)?);
        }

        let mut reviews = reviews;
        reviews.reverse();
        for review in reviews.iter().take(limits.max_reviews) {
            fragments.push(self.review_fragment(review, limits.max_summary_chars)?);
        }

        // Memory packets reference the packet artifact; we do not inline contents.
        let mut memory_packets = memory_packets;
        memory_packets.reverse();
        for packet in memory_packets.iter().take(limits.max_memory_packets) {
            fragments.push(self.memory_packet_fragment(packet, limits.max_summary_chars)?);
        }

        // Continuation decisions: newest first, bounded.
        let mut continuations = self.state.goal_continuations_for_goal(goal_id)?;
        continuations.reverse();
        for continuation in continuations.iter().take(limits.max_continuations) {
            fragments.push(continuation_fragment(
                continuation,
                limits.max_summary_chars,
            ));
        }

        // Delegated-provider goal observations (observed, not authoritative).
        for delegated in self.state.delegated_provider_goals_for_goal(goal_id)? {
            fragments.push(self.delegated_fragment(&delegated, limits.max_summary_chars)?);
        }

        // The goal's task, when present, is its workpad/source anchor: a REFERENCE
        // only, never the file contents.
        if let Some(task_id) = goal.task_id.as_ref() {
            fragments.push(workpad_fragment(task_id.as_str(), limits.max_summary_chars));
        }

        let content_hash = packet_content_hash(&audit_contract, &fragments);
        Ok(ContinuationContextPacket {
            audit_contract,
            fragments,
            content_hash,
        })
    }

    fn report_fragment(
        &self,
        report: &GoalReportProjection,
        max_summary_chars: usize,
    ) -> StateResult<ContinuationContextFragment> {
        let (body_content_hash, redacted) = self.artifact_provenance(&report.body_artifact_id)?;
        let observed = report.is_observed_evidence();
        let summary = clamp_summary(
            &format!(
                "{} [{}]{}: {}",
                report.report_kind,
                report.source,
                report
                    .confidence
                    .map(|c| format!(" confidence {c}"))
                    .unwrap_or_default(),
                report.summary,
            ),
            max_summary_chars,
        );
        let kind = if observed {
            ContinuationSourceKind::ObservedEvidence
        } else {
            ContinuationSourceKind::AgentReport
        };
        Ok(fragment(
            kind,
            report.goal_report_id.clone(),
            summary,
            Some(observed),
            report.body_artifact_id.clone(),
            body_content_hash,
            redacted,
        ))
    }

    fn evidence_fragment(
        &self,
        evidence: &EvidenceProjection,
        max_summary_chars: usize,
    ) -> StateResult<ContinuationContextFragment> {
        let (body_content_hash, redacted) = self.artifact_provenance(&evidence.artifact_id)?;
        let summary = clamp_summary(
            &format!(
                "observed evidence kind={} confidence={}",
                evidence.kind, evidence.confidence
            ),
            max_summary_chars,
        );
        Ok(fragment(
            ContinuationSourceKind::ObservedEvidence,
            evidence.evidence_id.as_str().to_string(),
            summary,
            Some(true),
            evidence.artifact_id.clone(),
            body_content_hash,
            redacted,
        ))
    }

    /// Look up a referenced artifact's content hash and whether it must be carried
    /// as a redacted reference (non-`safe`). Returns `(None, false)` when the
    /// fragment references no artifact, and `(None, true)` when the referenced
    /// artifact is absent (degraded: named, can't be inlined).
    fn artifact_provenance(
        &self,
        artifact_id: &Option<String>,
    ) -> StateResult<(Option<String>, bool)> {
        let Some(artifact_id) = artifact_id.as_ref() else {
            return Ok((None, false));
        };
        match self.state.artifact_by_id(artifact_id)? {
            Some(artifact) => {
                let redacted = artifact.redaction_state != RedactionState::Safe;
                Ok((Some(artifact.content_hash), redacted))
            }
            // A referenced-but-missing artifact degrades clearly: keep the
            // reference, mark it redacted/unavailable, never invent content.
            None => Ok((None, true)),
        }
    }

    /// GA3 (GO7): a review finding. Its `evidence_artifact_id` body is named with
    /// the artifact's content hash + redaction state via [`Self::artifact_provenance`]
    /// (a method so it can reach the artifact store), so a redacted/missing review
    /// body degrades to a redacted reference rather than silently claiming `safe`.
    fn review_fragment(
        &self,
        review: &ReviewFindingProjection,
        max_summary_chars: usize,
    ) -> StateResult<ContinuationContextFragment> {
        let (body_content_hash, redacted) =
            self.artifact_provenance(&review.evidence_artifact_id)?;
        let summary = clamp_summary(
            &format!(
                "review {} [{}/{}]: {}",
                review.reviewer, review.finding_kind, review.severity, review.summary
            ),
            max_summary_chars,
        );
        Ok(fragment(
            ContinuationSourceKind::ReviewFinding,
            review.review_finding_id.clone(),
            summary,
            None,
            review.evidence_artifact_id.clone(),
            body_content_hash,
            redacted,
        ))
    }

    /// GA3 (GO7): a memory-packet reference. Its `packet_artifact_id` body is named
    /// with the artifact's content hash + redaction state (see
    /// [`Self::artifact_provenance`]); we reference the packet, never inline it.
    fn memory_packet_fragment(
        &self,
        packet: &MemoryPacketProjection,
        max_summary_chars: usize,
    ) -> StateResult<ContinuationContextFragment> {
        let (body_content_hash, redacted) = self.artifact_provenance(&packet.packet_artifact_id)?;
        let summary = clamp_summary(
            &format!("memory packet: {}", packet.purpose),
            max_summary_chars,
        );
        Ok(fragment(
            ContinuationSourceKind::MemoryPacket,
            packet.memory_packet_id.as_str().to_string(),
            summary,
            None,
            packet.packet_artifact_id.clone(),
            body_content_hash,
            redacted,
        ))
    }

    /// GA3 (GO7): a delegated-provider goal observation (observed-not-authoritative
    /// evidence the auditor weighs). Its `body_artifact_id` body is named with the
    /// artifact's content hash + redaction state (see [`Self::artifact_provenance`]).
    fn delegated_fragment(
        &self,
        delegated: &DelegatedProviderGoalProjection,
        max_summary_chars: usize,
    ) -> StateResult<ContinuationContextFragment> {
        let (body_content_hash, redacted) =
            self.artifact_provenance(&delegated.body_artifact_id)?;
        let summary = clamp_summary(
            &format!(
                "delegated {} state={} [{}]",
                delegated.provider_kind, delegated.provider_state, delegated.source
            ),
            max_summary_chars,
        );
        // A provider-native completion is observed-not-authoritative evidence the
        // auditor weighs; it is never an authoritative completion, so it is tagged
        // observed but kept distinct as its own source kind.
        Ok(fragment(
            ContinuationSourceKind::DelegatedProviderGoal,
            delegated.delegated_goal_id.clone(),
            summary,
            Some(true),
            delegated.body_artifact_id.clone(),
            body_content_hash,
            redacted,
        ))
    }
}

fn build_audit_contract(
    goal: &GoalProjection,
    requirements: &[RequirementLedgerProjection],
) -> ContinuationAuditContract {
    let requirements: Vec<ContinuationRequirement> = requirements
        .iter()
        .map(|ledger| ContinuationRequirement {
            requirement_id: ledger.requirement_id.as_str().to_string(),
            summary: ledger.summary.clone(),
            status: ledger.status.clone(),
            observed_status: capo_tools::source_is_observed_evidence(&ledger.last_status_source),
        })
        .collect();
    // The audit-contract digest covers ONLY the transcript-independent objective
    // and contract -- the fields that must rebuild identically across a restart.
    let digest_input = format!(
        "goal={}|objective={}|status={}|success={}|constraints={}|verification={}|stop={}|blocker={}|reqs={}",
        goal.goal_id.as_str(),
        goal.objective,
        goal.status,
        goal.success_criteria_json,
        goal.constraints_json,
        goal.verification_surface_json,
        goal.stop_conditions_json,
        goal.blocker_reason,
        requirements
            .iter()
            .map(|r| format!(
                "{}={}:{}:{}",
                r.requirement_id, r.summary, r.status, r.observed_status
            ))
            .collect::<Vec<_>>()
            .join(","),
    );
    ContinuationAuditContract {
        goal_id: goal.goal_id.as_str().to_string(),
        objective: goal.objective.clone(),
        status: goal.status.clone(),
        success_criteria_json: goal.success_criteria_json.clone(),
        constraints_json: goal.constraints_json.clone(),
        verification_surface_json: goal.verification_surface_json.clone(),
        stop_conditions_json: goal.stop_conditions_json.clone(),
        blocker_reason: goal.blocker_reason.clone(),
        requirements,
        content_hash: stable_hash(digest_input.as_bytes()),
    }
}

fn requirement_fragment(
    requirement: &RequirementLedgerProjection,
    max_summary_chars: usize,
) -> ContinuationContextFragment {
    let observed = capo_tools::source_is_observed_evidence(&requirement.last_status_source);
    let summary = clamp_summary(
        &format!("[{}] {}", requirement.status, requirement.summary),
        max_summary_chars,
    );
    fragment(
        ContinuationSourceKind::Requirement,
        requirement.requirement_id.as_str().to_string(),
        summary,
        Some(observed),
        None,
        None,
        false,
    )
}

fn continuation_fragment(
    continuation: &GoalContinuationProjection,
    max_summary_chars: usize,
) -> ContinuationContextFragment {
    let summary = clamp_summary(
        &format!(
            "decision {} — {}",
            continuation.decision, continuation.reason
        ),
        max_summary_chars,
    );
    fragment(
        ContinuationSourceKind::ContinuationDecision,
        continuation.continuation_id.clone(),
        summary,
        None,
        None,
        None,
        false,
    )
}

fn workpad_fragment(task_id: &str, max_summary_chars: usize) -> ContinuationContextFragment {
    let summary = clamp_summary(
        &format!("workpad/source anchor: {task_id}"),
        max_summary_chars,
    );
    fragment(
        ContinuationSourceKind::WorkpadRef,
        task_id.to_string(),
        summary,
        None,
        None,
        None,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn fragment(
    kind: ContinuationSourceKind,
    source_ref: String,
    summary: String,
    observed: Option<bool>,
    body_artifact_id: Option<String>,
    body_content_hash: Option<String>,
    redacted: bool,
) -> ContinuationContextFragment {
    let content_hash = stable_hash(
        format!(
            "{}|{}|{}|{:?}|{:?}|{:?}|{}",
            kind.as_str(),
            source_ref,
            summary,
            observed,
            body_artifact_id,
            body_content_hash,
            redacted,
        )
        .as_bytes(),
    );
    ContinuationContextFragment {
        kind,
        source_ref,
        summary,
        observed,
        body_artifact_id,
        body_content_hash,
        redacted,
        content_hash,
    }
}

/// Truncate a summary to `max_chars`, appending an explicit ellipsis when cut, so
/// no single fragment can inline a whole body. Bounding is on CHARS (not bytes) so
/// the cut never splits a UTF-8 codepoint.
fn clamp_summary(summary: &str, max_chars: usize) -> String {
    if summary.chars().count() <= max_chars {
        return summary.to_string();
    }
    let kept = max_chars.saturating_sub(1);
    let truncated: String = summary.chars().take(kept).collect();
    format!("{truncated}…")
}

/// A stable digest over the whole packet: the audit-contract digest plus every
/// fragment's content hash, in order. The same persisted state always yields the
/// same packet hash, so a restart-rebuilt packet is provably identical.
fn packet_content_hash(
    audit_contract: &ContinuationAuditContract,
    fragments: &[ContinuationContextFragment],
) -> String {
    let mut input = String::new();
    input.push_str(&audit_contract.content_hash);
    for fragment in fragments {
        input.push('|');
        input.push_str(&fragment.content_hash);
    }
    stable_hash(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_core::RequirementId;
    use capo_state::{
        ArtifactRecord, EvidenceProjection, GoalContinuationProjection, GoalProjection,
        GoalReportProjection, MemoryPacketProjection, NewEvent, ProjectionRecord,
        RequirementLedgerProjection, ReviewFindingProjection,
    };

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-ga3-{name}-{nanos}-{n}"))
    }

    const PROJECT: &str = "project-capo";
    const GOAL: &str = "goal-ga3";
    const SESSION: &str = "session-ga3";

    fn open() -> (FakeBoundaryController, PathBuf) {
        let state_root = temp_root("state");
        let controller =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("controller");
        (controller, state_root)
    }

    /// Append one event carrying the given projection records, keyed for idempotent
    /// replay. The controller test seeds goal state directly through the store, the
    /// same way GA1/GA2 project it, so the packet assembly reads real persisted
    /// rows.
    fn seed(controller: &FakeBoundaryController, event_id: &str, records: &[ProjectionRecord]) {
        let mut event = NewEvent::new(event_id, EventKind::GoalReportRecorded, "test-seed");
        event.project_id = Some(ProjectId::new(PROJECT));
        event.idempotency_key = Some(event_id.to_string());
        event.item_id = Some(event_id.to_string());
        controller
            .state
            .append_event(event, records)
            .expect("seed append");
    }

    fn goal_projection(status: &str) -> GoalProjection {
        GoalProjection {
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new("task-ga3")),
            agent_id: Some(AgentId::new("agent-ga3")),
            session_id: Some(SessionId::new(SESSION)),
            parent_goal_id: None,
            attempt_run_id: Some(RunId::new("run-ga3")),
            objective: "Ship the GA3 continuation packet".to_string(),
            status: status.to_string(),
            success_criteria_json: r#"{"must":["build green"]}"#.to_string(),
            constraints_json: r#"{"no_network":true}"#.to_string(),
            verification_surface_json: r#"{"cmd":"cargo test"}"#.to_string(),
            budget_json: r#"{"max_turns":8}"#.to_string(),
            stop_conditions_json: r#"{"on":"blocker"}"#.to_string(),
            blocker_reason: String::new(),
            updated_sequence: 0,
        }
    }

    fn requirement(
        id: &str,
        summary: &str,
        status: &str,
        source: &str,
    ) -> RequirementLedgerProjection {
        RequirementLedgerProjection {
            requirement_id: RequirementId::new(id),
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            summary: summary.to_string(),
            status: status.to_string(),
            last_status_source: source.to_string(),
            updated_sequence: 0,
        }
    }

    fn seed_baseline_goal(controller: &FakeBoundaryController) {
        seed(
            controller,
            "seed-goal",
            &[
                ProjectionRecord::Goal(goal_projection(GoalProjection::ACTIVE)),
                ProjectionRecord::RequirementLedger(requirement(
                    "req-1",
                    "packet assembled from persisted state",
                    RequirementLedgerProjection::SUPPORTED,
                    "runtime_output",
                )),
                ProjectionRecord::RequirementLedger(requirement(
                    "req-2",
                    "objective survives restart",
                    RequirementLedgerProjection::UNVERIFIED,
                    "controller",
                )),
            ],
        );
    }

    #[test]
    fn continuation_context_packet_selects_bounded_sourced_fragments() {
        let (controller, _root) = open();
        seed_baseline_goal(&controller);

        // An agent-reported claim and an observed evidence row, distinguished by
        // their source tags.
        seed(
            &controller,
            "seed-report-claim",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "report-claim".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: Some(RequirementId::new("req-2")),
                report_kind: "capo.report_progress".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(60),
                summary: "I believe req-2 is nearly done".to_string(),
                body_artifact_id: None,
                evidence_id: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-evidence",
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: EvidenceId::new("evidence-1"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "test".to_string(),
                artifact_id: None,
                confidence: 100,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-review",
            &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: "review-1".to_string(),
                project_id: ProjectId::new(PROJECT),
                task_id: TaskId::new("task-ga3"),
                session_id: SessionId::new(SESSION),
                run_id: Some(RunId::new("run-ga3")),
                tool_call_id: None,
                workpad_task_id: None,
                reviewer: "reviewer-ga3".to_string(),
                finding_kind: "correctness".to_string(),
                severity: "low".to_string(),
                summary: "looks good".to_string(),
                status: "open".to_string(),
                evidence_artifact_id: None,
                follow_up: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-memory",
            &[ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new("packet-1"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                agent_id: Some(AgentId::new("agent-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                turn_id: None,
                packet_artifact_id: Some("artifact-packet-1".to_string()),
                purpose: "carry GA1/GA2 context forward".to_string(),
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-continuation",
            &[ProjectionRecord::GoalContinuation(
                GoalContinuationProjection {
                    continuation_id: "continuation-1".to_string(),
                    goal_id: GoalId::new(GOAL),
                    project_id: ProjectId::new(PROJECT),
                    attempt_run_id: Some(RunId::new("run-ga3")),
                    decision: "continue".to_string(),
                    reason: "safe_boundary".to_string(),
                    updated_sequence: 0,
                },
            )],
        );

        let packet = controller
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet");

        // The objective + audit contract are reconstructed from persisted state.
        assert_eq!(
            packet.audit_contract.objective,
            "Ship the GA3 continuation packet"
        );
        assert_eq!(packet.audit_contract.requirements.len(), 2);
        // The supported requirement was driven by observed evidence; the unverified
        // one by the controller (not observed).
        let req1 = packet
            .audit_contract
            .requirements
            .iter()
            .find(|r| r.requirement_id == "req-1")
            .expect("req-1");
        assert!(req1.observed_status, "supported-by-observed is observed");
        let req2 = packet
            .audit_contract
            .requirements
            .iter()
            .find(|r| r.requirement_id == "req-2")
            .expect("req-2");
        assert!(!req2.observed_status);

        // Every fragment carries a non-empty source ref and content hash.
        assert!(!packet.fragments.is_empty());
        for fragment in &packet.fragments {
            assert!(!fragment.source_ref.is_empty(), "fragment names its source");
            assert!(
                !fragment.content_hash.is_empty(),
                "fragment is content-hashed"
            );
        }

        // The agent claim is tagged reported; the evidence row is tagged observed.
        let claim = packet
            .fragments
            .iter()
            .find(|f| f.source_ref == "report-claim")
            .expect("claim fragment");
        assert_eq!(claim.kind, ContinuationSourceKind::AgentReport);
        assert_eq!(claim.observed, Some(false));
        let evidence = packet
            .fragments
            .iter()
            .find(|f| f.source_ref == "evidence-1")
            .expect("evidence fragment");
        assert_eq!(evidence.kind, ContinuationSourceKind::ObservedEvidence);
        assert_eq!(evidence.observed, Some(true));

        // The packet is content-hashed as a whole.
        assert!(!packet.content_hash.is_empty());

        // The rendered prompt leads with the objective and never inlines a body.
        let prompt = packet.render_prompt();
        assert!(prompt.contains("Objective: Ship the GA3 continuation packet"));
        assert!(prompt.contains("Audit contract"));
    }

    #[test]
    fn continuation_context_is_bounded_and_does_not_dump_whole_bodies() {
        let (controller, _root) = open();
        seed_baseline_goal(&controller);

        // Seed MORE reports than the limit allows. Each carries a DISTINCT, ordered
        // body (a long unique prefix so we also prove bodies are not dumped) so the
        // test can prove the surviving fragments are the NEWEST N in newest-first
        // order -- not just that exactly N survive. Reports are seeded report-0
        // first .. report-19 last, so report-19 is the newest.
        let long = "x".repeat(5_000);
        for i in 0..20 {
            seed(
                &controller,
                &format!("seed-report-{i}"),
                &[ProjectionRecord::GoalReport(GoalReportProjection {
                    goal_report_id: format!("report-{i:02}"),
                    goal_id: GoalId::new(GOAL),
                    project_id: ProjectId::new(PROJECT),
                    session_id: Some(SessionId::new(SESSION)),
                    requirement_id: None,
                    report_kind: "capo.report_progress".to_string(),
                    source: "agent_reported".to_string(),
                    confidence: Some(50),
                    summary: format!("report body {i} {long}"),
                    body_artifact_id: None,
                    evidence_id: None,
                    updated_sequence: 0,
                })],
            );
        }

        let limits = ContinuationContextLimits {
            max_reports: 3,
            ..ContinuationContextLimits::default()
        };
        let packet = controller
            .continuation_context_packet_with_limits(&GoalId::new(GOAL), limits)
            .expect("packet");

        let report_fragments: Vec<_> = packet
            .fragments
            .iter()
            .filter(|f| f.kind == ContinuationSourceKind::AgentReport)
            .collect();
        assert_eq!(
            report_fragments.len(),
            3,
            "report fragments are bounded by max_reports"
        );
        // The surviving fragments are exactly the NEWEST three, newest first. This
        // fails if the `.reverse()` in assembly is dropped (oldest-3 kept) or if an
        // arbitrary 3 are selected, so the recency guarantee is PROVEN, not asserted
        // by construction.
        let surviving_refs: Vec<&str> = report_fragments
            .iter()
            .map(|f| f.source_ref.as_str())
            .collect();
        assert_eq!(
            surviving_refs,
            vec!["report-19", "report-18", "report-17"],
            "the newest three reports survive, newest first"
        );
        for fragment in report_fragments {
            assert!(
                fragment.summary.chars().count() <= limits.max_summary_chars,
                "no fragment dumps a whole body: {} chars",
                fragment.summary.chars().count()
            );
        }
    }

    #[test]
    fn continuation_context_preserves_artifact_content_hash_and_redacts_unsafe_bodies() {
        let (controller, _root) = open();
        seed_baseline_goal(&controller);

        // A safe artifact: its content hash is preserved on the fragment, body not
        // inlined.
        controller
            .state
            .record_artifact(ArtifactRecord {
                artifact_id: "artifact-safe".to_string(),
                project_id: Some(ProjectId::new(PROJECT)),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "report_body".to_string(),
                uri: "file:///tmp/report-body.txt".to_string(),
                content_hash: "sha256:deadbeef".to_string(),
                size_bytes: 1234,
                redaction_state: RedactionState::Safe,
            })
            .expect("record safe artifact");
        // A redacted artifact: included as a redacted reference only.
        controller
            .state
            .record_artifact(ArtifactRecord {
                artifact_id: "artifact-redacted".to_string(),
                project_id: Some(ProjectId::new(PROJECT)),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "report_body".to_string(),
                uri: "file:///tmp/redacted.txt".to_string(),
                content_hash: "sha256:cafef00d".to_string(),
                size_bytes: 99,
                redaction_state: RedactionState::Redacted,
            })
            .expect("record redacted artifact");

        seed(
            &controller,
            "seed-report-safe",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "report-safe".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: None,
                report_kind: "capo.test_run".to_string(),
                source: "runtime_output".to_string(),
                confidence: None,
                summary: "test run output".to_string(),
                body_artifact_id: Some("artifact-safe".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-report-redacted",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "report-redacted".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: None,
                report_kind: "capo.report_progress".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(40),
                summary: "redacted body report".to_string(),
                body_artifact_id: Some("artifact-redacted".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        );
        // A report referencing a MISSING artifact degrades clearly.
        seed(
            &controller,
            "seed-report-missing",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "report-missing".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: None,
                report_kind: "capo.report_progress".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(30),
                summary: "missing body".to_string(),
                body_artifact_id: Some("artifact-does-not-exist".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        );

        let packet = controller
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet");

        let safe = packet
            .fragments
            .iter()
            .find(|f| f.source_ref == "report-safe")
            .expect("safe report fragment");
        assert_eq!(safe.body_content_hash.as_deref(), Some("sha256:deadbeef"));
        assert!(!safe.redacted, "a safe artifact is not redacted");
        // The body is never inlined: the summary is the report summary, not the
        // artifact contents.
        assert!(!safe.summary.contains("deadbeef"));

        let redacted = packet
            .fragments
            .iter()
            .find(|f| f.source_ref == "report-redacted")
            .expect("redacted report fragment");
        assert!(
            redacted.redacted,
            "a non-safe artifact is a redacted reference"
        );
        assert_eq!(
            redacted.body_content_hash.as_deref(),
            Some("sha256:cafef00d")
        );

        let missing = packet
            .fragments
            .iter()
            .find(|f| f.source_ref == "report-missing")
            .expect("missing report fragment");
        assert!(
            missing.redacted,
            "a missing artifact degrades to a redacted reference"
        );
        assert_eq!(missing.body_content_hash, None);
    }

    #[test]
    fn continuation_objective_and_audit_contract_survive_server_restart_and_rebuild() {
        let (controller, state_root) = open();
        seed_baseline_goal(&controller);

        // Seed SOURCED fragments before capturing `before`, so the post-rebuild
        // `before == after` equality actually exercises fragment selection,
        // newest-N ordering, and the `artifact_by_id` provenance path across a
        // restart -- not just the transcript-independent audit contract.
        controller
            .state
            .record_artifact(ArtifactRecord {
                artifact_id: "restart-artifact-safe".to_string(),
                project_id: Some(ProjectId::new(PROJECT)),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "report_body".to_string(),
                uri: "file:///tmp/restart-safe.txt".to_string(),
                content_hash: "sha256:safe-restart".to_string(),
                size_bytes: 64,
                redaction_state: RedactionState::Safe,
            })
            .expect("record safe artifact");
        controller
            .state
            .record_artifact(ArtifactRecord {
                artifact_id: "restart-artifact-redacted".to_string(),
                project_id: Some(ProjectId::new(PROJECT)),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "report_body".to_string(),
                uri: "file:///tmp/restart-redacted.txt".to_string(),
                content_hash: "sha256:redacted-restart".to_string(),
                size_bytes: 64,
                redaction_state: RedactionState::Redacted,
            })
            .expect("record redacted artifact");
        // Two reports: one referencing a safe body, one a redacted body.
        seed(
            &controller,
            "seed-restart-report-safe",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "restart-report-safe".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: None,
                report_kind: "capo.test_run".to_string(),
                source: "runtime_output".to_string(),
                confidence: None,
                summary: "restart safe report".to_string(),
                body_artifact_id: Some("restart-artifact-safe".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-restart-report-redacted",
            &[ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "restart-report-redacted".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: None,
                report_kind: "capo.report_progress".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(40),
                summary: "restart redacted report".to_string(),
                body_artifact_id: Some("restart-artifact-redacted".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        );
        // Observed evidence and a memory packet (task-scoped, cross-attempt).
        seed(
            &controller,
            "seed-restart-evidence",
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: EvidenceId::new("restart-evidence-1"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "test".to_string(),
                artifact_id: Some("restart-artifact-safe".to_string()),
                confidence: 100,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-restart-memory",
            &[ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new("restart-packet-1"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                agent_id: Some(AgentId::new("agent-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                turn_id: None,
                packet_artifact_id: Some("restart-artifact-redacted".to_string()),
                purpose: "carry context across restart".to_string(),
                updated_sequence: 0,
            })],
        );

        // A blocker so the contract carries current-blocker state too.
        seed(
            &controller,
            "seed-block",
            &[ProjectionRecord::Goal(GoalProjection {
                blocker_reason: "waiting on safety-gates lock".to_string(),
                status: GoalProjection::BLOCKED.to_string(),
                ..goal_projection(GoalProjection::BLOCKED)
            })],
        );

        let before = controller
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet before restart");

        // The seeded sourced fragments are present before restart, so the equality
        // below is load-bearing for fragment assembly (not just the contract).
        let safe_before = before
            .fragments
            .iter()
            .find(|f| f.source_ref == "restart-report-safe")
            .expect("safe report fragment present before restart");
        assert_eq!(
            safe_before.body_content_hash.as_deref(),
            Some("sha256:safe-restart")
        );
        assert!(!safe_before.redacted);
        let redacted_before = before
            .fragments
            .iter()
            .find(|f| f.source_ref == "restart-report-redacted")
            .expect("redacted report fragment present before restart");
        assert!(redacted_before.redacted);
        assert!(
            before
                .fragments
                .iter()
                .any(|f| f.source_ref == "restart-evidence-1"),
            "observed evidence present before restart"
        );
        assert!(
            before
                .fragments
                .iter()
                .any(|f| f.source_ref == "restart-packet-1"),
            "memory packet present before restart"
        );

        // Simulate a server restart: drop the controller, RE-OPEN over the same
        // state root, and REBUILD all projections from the event log. The objective
        // and audit contract must reconstruct from persisted state, not a
        // transcript.
        drop(controller);
        let restarted = FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root)
            .expect("restarted controller");
        restarted
            .state
            .rebuild_projections()
            .expect("rebuild projections");

        let after = restarted
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet after restart");

        // The transcript-independent audit contract is byte-for-byte identical.
        assert_eq!(before.audit_contract, after.audit_contract);
        assert_eq!(
            before.audit_contract.content_hash, after.audit_contract.content_hash,
            "the audit-contract digest survives restart + rebuild"
        );
        assert_eq!(
            after.audit_contract.objective,
            "Ship the GA3 continuation packet"
        );
        assert_eq!(after.audit_contract.status, GoalProjection::BLOCKED);
        assert_eq!(
            after.audit_contract.blocker_reason,
            "waiting on safety-gates lock"
        );
        // The whole packet rebuilds identically.
        assert_eq!(before, after, "the whole packet rebuilds identically");
        // The prompt is reconstructable from persisted state.
        assert!(
            after
                .render_prompt()
                .contains("Objective: Ship the GA3 continuation packet")
        );
    }

    #[test]
    fn observed_evidence_reviews_and_memory_survive_a_goal_rebind_to_a_new_attempt_session() {
        let (controller, _root) = open();
        // Attempt 1: the goal is bound to SESSION, and a first attempt records
        // observed evidence, a review finding, and a memory packet against that
        // session (but always under the stable task id).
        seed_baseline_goal(&controller);
        seed(
            &controller,
            "seed-attempt1-evidence",
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: EvidenceId::new("attempt1-evidence"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "test".to_string(),
                artifact_id: None,
                confidence: 100,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-attempt1-review",
            &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: "attempt1-review".to_string(),
                project_id: ProjectId::new(PROJECT),
                task_id: TaskId::new("task-ga3"),
                session_id: SessionId::new(SESSION),
                run_id: Some(RunId::new("run-ga3")),
                tool_call_id: None,
                workpad_task_id: None,
                reviewer: "reviewer-ga3".to_string(),
                finding_kind: "correctness".to_string(),
                severity: "low".to_string(),
                summary: "attempt 1 review".to_string(),
                status: "open".to_string(),
                evidence_artifact_id: None,
                follow_up: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-attempt1-memory",
            &[ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new("attempt1-packet"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                agent_id: Some(AgentId::new("agent-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                turn_id: None,
                packet_artifact_id: None,
                purpose: "attempt 1 context".to_string(),
                updated_sequence: 0,
            })],
        );

        // A continuation rebinds the SAME goal to a fresh attempt session. This is
        // exactly the `ON CONFLICT(goal_id) DO UPDATE SET session_id = ...` path.
        const SESSION_2: &str = "session-ga3-attempt-2";
        seed(
            &controller,
            "seed-rebind",
            &[ProjectionRecord::Goal(GoalProjection {
                session_id: Some(SessionId::new(SESSION_2)),
                attempt_run_id: Some(RunId::new("run-ga3-attempt-2")),
                ..goal_projection(GoalProjection::ACTIVE)
            })],
        );

        let packet = controller
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet after rebind");

        // The prior attempt's observed evidence / review / memory MUST still be in
        // the packet even though the goal is now bound to a different session. A
        // session-scoped read (the bug) would drop all three.
        assert!(
            packet
                .fragments
                .iter()
                .any(|f| f.source_ref == "attempt1-evidence"),
            "prior-attempt observed evidence survives a goal rebind"
        );
        assert!(
            packet
                .fragments
                .iter()
                .any(|f| f.source_ref == "attempt1-review"),
            "prior-attempt review finding survives a goal rebind"
        );
        assert!(
            packet
                .fragments
                .iter()
                .any(|f| f.source_ref == "attempt1-packet"),
            "prior-attempt memory packet survives a goal rebind"
        );
    }

    #[test]
    fn memory_review_and_delegated_bodies_carry_artifact_hash_and_redaction() {
        let (controller, _root) = open();
        seed_baseline_goal(&controller);

        // A redacted artifact referenced by a memory packet, a review finding, and a
        // delegated-provider observation. Each must degrade to a redacted reference
        // with the artifact's content hash -- not silently report redacted=false /
        // body_content_hash=None.
        controller
            .state
            .record_artifact(ArtifactRecord {
                artifact_id: "shared-redacted".to_string(),
                project_id: Some(ProjectId::new(PROJECT)),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                kind: "body".to_string(),
                uri: "file:///tmp/shared-redacted.txt".to_string(),
                content_hash: "sha256:shared".to_string(),
                size_bytes: 10,
                redaction_state: RedactionState::Redacted,
            })
            .expect("record redacted artifact");

        seed(
            &controller,
            "seed-memory-redacted",
            &[ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new("packet-redacted"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new("task-ga3")),
                agent_id: Some(AgentId::new("agent-ga3")),
                session_id: Some(SessionId::new(SESSION)),
                run_id: Some(RunId::new("run-ga3")),
                turn_id: None,
                packet_artifact_id: Some("shared-redacted".to_string()),
                purpose: "carries a sensitive packet body".to_string(),
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-review-redacted",
            &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: "review-redacted".to_string(),
                project_id: ProjectId::new(PROJECT),
                task_id: TaskId::new("task-ga3"),
                session_id: SessionId::new(SESSION),
                run_id: Some(RunId::new("run-ga3")),
                tool_call_id: None,
                workpad_task_id: None,
                reviewer: "reviewer-ga3".to_string(),
                finding_kind: "correctness".to_string(),
                severity: "high".to_string(),
                summary: "review with sensitive evidence".to_string(),
                status: "open".to_string(),
                evidence_artifact_id: Some("shared-redacted".to_string()),
                follow_up: None,
                updated_sequence: 0,
            })],
        );
        seed(
            &controller,
            "seed-delegated-redacted",
            &[ProjectionRecord::DelegatedProviderGoal(
                DelegatedProviderGoalProjection {
                    delegated_goal_id: "delegated-redacted".to_string(),
                    goal_id: GoalId::new(GOAL),
                    project_id: ProjectId::new(PROJECT),
                    session_id: Some(SessionId::new(SESSION)),
                    provider_kind: "codex".to_string(),
                    provider_goal_ref: None,
                    provider_state: "completed".to_string(),
                    source: "adapter_event".to_string(),
                    body_artifact_id: Some("shared-redacted".to_string()),
                    updated_sequence: 0,
                },
            )],
        );

        let packet = controller
            .continuation_context_packet(&GoalId::new(GOAL))
            .expect("packet");

        for source_ref in ["packet-redacted", "review-redacted", "delegated-redacted"] {
            let frag = packet
                .fragments
                .iter()
                .find(|f| f.source_ref == source_ref)
                .unwrap_or_else(|| panic!("{source_ref} fragment"));
            assert!(
                frag.redacted,
                "{source_ref}: a non-safe artifact body is a redacted reference"
            );
            assert_eq!(
                frag.body_content_hash.as_deref(),
                Some("sha256:shared"),
                "{source_ref}: the referenced artifact's content hash is carried"
            );
        }
    }
}
