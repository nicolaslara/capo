//! ACI8: the `GO2` agent-reporting and evidence tool surface.
//!
//! These are the goal-orchestration `GO2` reporting tools
//! (`workpads/goal-orchestration/tasks.md:86-104`): `capo.report_intent`,
//! `capo.report_progress`, `capo.record_evidence`, `capo.report_confidence`,
//! `capo.record_assumption`, `capo.raise_blocker`, `capo.request_review`,
//! `capo.record_review`, `capo.record_validation`, `capo.complete_requirement`,
//! and `capo.complete_subtask`.
//!
//! This task CITES `GO2` as the design source and does NOT redesign the goal
//! model or the report schema. Its scope is **emission + fakes**: register the
//! tools in the typed registry (schema / required_scopes / risk /
//! redaction_policy / mutates_state) and persist their output as a DISTINCT
//! event/projection class tagged [`EVIDENCE_SOURCE_AGENT_REPORTED`], carrying
//! confidence, separate from observed tool evidence tagged
//! [`EVIDENCE_SOURCE_RUNTIME_OUTPUT`] / [`EVIDENCE_SOURCE_ADAPTER_EVENT`]. The
//! load-bearing decision (from the knowledge doc) is that **reports are claims,
//! not proof**: a completion is never reachable by agent assertion alone, so an
//! agent report is structurally distinguishable from observed evidence and never
//! masquerades as it. The projection/audit semantics over this surface are
//! validated in `goal-autonomy` (`GA-2`/`GA-6`).

use capo_core::{BoundaryBinding, BoundaryKind, SessionId, ToolCallId};
use serde_json::Value;

use crate::{
    PermissionDecision, PermissionPolicy, PermissionRequest, ToolAuditEvent, ToolDefinition,
    content_hash, json_array,
};

/// The `GO2` agent-reporting / evidence tools, registered as an ACI concern.
pub const CAPO_REPORTING_TOOLS: &[&str] = &[
    "capo.report_intent",
    "capo.report_progress",
    "capo.record_evidence",
    "capo.report_confidence",
    "capo.record_assumption",
    "capo.raise_blocker",
    "capo.request_review",
    "capo.record_review",
    "capo.record_validation",
    "capo.complete_requirement",
    "capo.complete_subtask",
];

// The observed-vs-claim classification is the single load-bearing safety
// invariant of the goal loop, so it has ONE owner in `capo-core`. This crate
// re-exports it (and the source-tag constants) rather than keeping its own copy,
// so `capo-tools` and `capo-state` cannot drift on what counts as observed
// evidence.
pub use capo_core::{
    EVIDENCE_SOURCE_ADAPTER_EVENT, EVIDENCE_SOURCE_AGENT_REPORTED, EVIDENCE_SOURCE_RUNTIME_OUTPUT,
    source_is_observed_evidence,
};

/// The Capo-owned registry for the `GO2` agent-reporting / evidence tools.
///
/// A distinct registry (not folded into [`crate::CapoToolRegistry`]) because the
/// reporting surface is its own ACI concern: every tool here emits an
/// [`AgentReportRecord`] tagged [`EVIDENCE_SOURCE_AGENT_REPORTED`], never an
/// observed-evidence record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentReportRegistry;

impl AgentReportRegistry {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ToolExposure,
            variant: "capo-agent-reports",
            fake: false,
        }
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        CAPO_REPORTING_TOOLS
            .iter()
            .map(|tool_id| self.describe_tool(tool_id).expect("known reporting tool"))
            .collect()
    }

    pub fn describe_tool(&self, tool_id: &str) -> Option<ToolDefinition> {
        let spec = report_tool_spec(tool_id)?;
        let mut scopes = vec![format!("tool:invoke:{tool_id}")];
        scopes.extend(spec.extra_scopes.iter().map(|scope| scope.to_string()));
        Some(ToolDefinition {
            tool_id: tool_id.to_string(),
            display_name: spec.display_name.to_string(),
            origin: "capo".to_string(),
            handler_kind: "agent_report".to_string(),
            schema_json: spec.schema_json.to_string(),
            output_schema: AGENT_REPORT_OUTPUT_SCHEMA.to_string(),
            required_scopes_json: json_array(scopes.iter().map(String::as_str).collect()),
            risk: spec.risk.to_string(),
            redaction_policy_json: report_redaction_policy(spec.redact_fields),
            // Reports are agent-submitted claims surfaced to the operator and the
            // parent agent; they are not internal-only.
            exposure: "agent_visible".to_string(),
            // A report is a structured CLAIM the runtime records, not an
            // instrumented execution it observes, so it is `structured_observed`
            // rather than `full`.
            instrumentation_level: "structured_observed".to_string(),
            status: "available".to_string(),
            mutates_state: spec.mutates_state,
        })
    }

    /// Authorize a report submission against the permission policy, then emit a
    /// typed [`AgentReportRecord`] tagged [`EVIDENCE_SOURCE_AGENT_REPORTED`].
    ///
    /// A denied submission emits a record with `accepted=false` and no
    /// `agent_reported` evidence: a report the policy rejected is not a claim of
    /// record.
    pub fn authorize_and_invoke(
        &self,
        request: AgentReportRequest,
        policy: &PermissionPolicy,
    ) -> AgentReportRecord {
        let definition = self
            .describe_tool(&request.tool_id)
            .unwrap_or_else(|| unknown_report_definition(&request.tool_id));
        let permission = policy.decide(PermissionRequest {
            session_id: request.session_id.clone(),
            capability_profile_id: request.capability_profile_id.clone(),
            scope_json: definition.required_scopes_json.clone(),
        });
        let accepted = permission.effect == "allow";
        let idempotency_key = report_idempotency_key(
            &request.submission_id,
            request.session_id.as_str(),
            &request.tool_id,
            &request.body,
        );
        let mut events = vec![
            ToolAuditEvent::new("tool.call_requested", "requested"),
            ToolAuditEvent::new("permission.requested", "pending"),
            ToolAuditEvent::new("permission.decided", permission.effect.clone()),
        ];
        if accepted {
            events.extend([
                ToolAuditEvent::new("capability.grant_used", "used"),
                ToolAuditEvent::new("tool.invocation_started", "running"),
                // The report is recorded as an OBSERVATION tagged
                // `agent_reported`, NOT as `tool.output_observed` runtime
                // evidence: this is the structural separation that keeps a claim
                // from masquerading as observed proof.
                ToolAuditEvent::new("tool.observation_recorded", EVIDENCE_SOURCE_AGENT_REPORTED),
                ToolAuditEvent::new("tool.call_completed", "completed"),
                ToolAuditEvent::new("tool.result_delivered", "delivered"),
            ]);
        } else {
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "permission_denied",
            ));
        }
        AgentReportRecord {
            tool_call_id: request.tool_call_id,
            tool_id: request.tool_id,
            // The DISTINCT class tag: this record is an agent CLAIM, not observed
            // evidence. `is_observed_evidence()` is always false.
            source: EVIDENCE_SOURCE_AGENT_REPORTED.to_string(),
            session_id: request.session_id,
            confidence: request.confidence,
            body: request.body,
            // The idempotency key duplicate report submissions dedupe on
            // (`AgentReportLedger::record`): a per-submission key when the agent
            // supplies one, else a stable content hash over the
            // session/tool/body so a re-emitted IDENTICAL report collapses on
            // replay while two distinct reports stay distinct.
            idempotency_key,
            accepted,
            mutates_state: definition.mutates_state,
            permission_decision: permission,
            events,
        }
    }
}

/// A typed agent-report submission (the `GO2` reporting tool input envelope).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentReportRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub tool_id: String,
    pub capability_profile_id: String,
    /// The agent's self-declared confidence in this report (0-100). A report
    /// carries confidence; observed evidence does not.
    pub confidence: i64,
    /// The structured report body (already validated against the tool schema by
    /// the caller); kept opaque here so emission does not redesign `GO2`.
    pub body: Value,
    /// An optional agent-supplied submission id used as the idempotency key, so
    /// a retried submission of the SAME report dedupes on replay. When absent, a
    /// stable content hash over the session/tool/body is used.
    pub submission_id: Option<String>,
}

/// The typed, persisted-shape record an agent report emits.
///
/// This is the DISTINCT class the ACI8 acceptance asks for: `source` is always
/// [`EVIDENCE_SOURCE_AGENT_REPORTED`], it carries the agent's `confidence`, and
/// it is never indistinguishable from observed evidence
/// ([`Self::is_observed_evidence`] is always false).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentReportRecord {
    pub tool_call_id: ToolCallId,
    pub tool_id: String,
    pub source: String,
    pub session_id: SessionId,
    pub confidence: i64,
    pub body: Value,
    pub idempotency_key: String,
    /// Whether the permission policy accepted the submission. A denied report is
    /// recorded for audit but is not a claim of record.
    pub accepted: bool,
    pub mutates_state: bool,
    pub permission_decision: PermissionDecision,
    pub events: Vec<ToolAuditEvent>,
}

impl AgentReportRecord {
    /// An agent report is NEVER observed evidence; this is the structural
    /// guarantee that completion is not reachable by agent assertion alone.
    pub fn is_observed_evidence(&self) -> bool {
        source_is_observed_evidence(&self.source)
    }

    /// Whether this report claims a unit of work is complete (a `complete_*`
    /// tool). Such a claim is still only a claim; observed evidence is required
    /// elsewhere before completion is real.
    pub fn is_completion_claim(&self) -> bool {
        matches!(
            self.tool_id.as_str(),
            "capo.complete_requirement" | "capo.complete_subtask"
        )
    }

    /// The narrow typed output object validatable against
    /// [`AGENT_REPORT_OUTPUT_SCHEMA`].
    pub fn narrow_output(&self) -> Value {
        serde_json::json!({
            "source": self.source,
            "accepted": self.accepted,
            "confidence": self.confidence,
            "idempotency_key": self.idempotency_key,
        })
    }
}

/// A deterministic, replayable ledger of agent reports that dedupes duplicate
/// submissions by idempotency key (the fake/scripted implementation the ACI8
/// acceptance asks for).
///
/// This is the emission-side stand-in for the `goal-autonomy` projection: it
/// proves the idempotent-dedupe contract deterministically without a live
/// provider or a SQLite projection (which `GA-2`/`GA-6` own). On replay, a
/// re-submitted report with the same idempotency key collapses to one entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentReportLedger {
    records: Vec<AgentReportRecord>,
}

impl AgentReportLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a report, deduping on its idempotency key. Returns `true` when the
    /// report was newly recorded, `false` when it deduped against an existing
    /// entry with the same key (the replay-idempotency contract).
    pub fn record(&mut self, record: AgentReportRecord) -> bool {
        if self
            .records
            .iter()
            .any(|existing| existing.idempotency_key == record.idempotency_key)
        {
            return false;
        }
        self.records.push(record);
        true
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn records(&self) -> &[AgentReportRecord] {
        &self.records
    }

    /// The agent-reported (claim) records: every record this ledger holds is a
    /// claim, never observed evidence.
    pub fn agent_reported(&self) -> Vec<&AgentReportRecord> {
        self.records
            .iter()
            .filter(|record| !record.is_observed_evidence())
            .collect()
    }
}

struct ReportToolSpec {
    display_name: &'static str,
    schema_json: &'static str,
    risk: &'static str,
    mutates_state: bool,
    extra_scopes: &'static [&'static str],
    redact_fields: &'static [&'static str],
}

/// Per-tool registration spec for a `GO2` reporting tool.
///
/// The schemas are minimal envelopes aligned with the tool NAMES; they cite
/// `GO2` and do not redesign its schema. `mutates_state` follows the `GO2`
/// acceptance: pure intent/progress/confidence/assumption reports are
/// observations (no mutation), while evidence/blocker/review/validation/
/// completion records mutate the autonomy ledger.
fn report_tool_spec(tool_id: &str) -> Option<ReportToolSpec> {
    let spec = match tool_id {
        "capo.report_intent" => ReportToolSpec {
            display_name: "Report Intent",
            schema_json: "{\"input\":{\"intent\":\"string\",\"requirement_id\":\"string?\"}}",
            risk: "low",
            mutates_state: false,
            extra_scopes: &["state:write:agent_report"],
            redact_fields: &["intent"],
        },
        "capo.report_progress" => ReportToolSpec {
            display_name: "Report Progress",
            schema_json: "{\"input\":{\"summary\":\"string\",\"requirement_id\":\"string?\",\"percent\":\"integer?\"}}",
            risk: "low",
            mutates_state: false,
            extra_scopes: &["state:write:agent_report"],
            redact_fields: &["summary"],
        },
        "capo.record_evidence" => ReportToolSpec {
            display_name: "Record Evidence",
            schema_json: "{\"input\":{\"evidence\":\"string\",\"evidence_kind\":\"string\",\"artifact_id\":\"string?\"}}",
            risk: "medium",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:evidence"],
            redact_fields: &["evidence"],
        },
        "capo.report_confidence" => ReportToolSpec {
            display_name: "Report Confidence",
            schema_json: "{\"input\":{\"confidence\":\"integer\",\"rationale\":\"string?\"}}",
            risk: "low",
            mutates_state: false,
            extra_scopes: &["state:write:agent_report"],
            redact_fields: &["rationale"],
        },
        "capo.record_assumption" => ReportToolSpec {
            display_name: "Record Assumption",
            schema_json: "{\"input\":{\"assumption\":\"string\",\"impact\":\"string?\"}}",
            risk: "low",
            mutates_state: false,
            extra_scopes: &["state:write:agent_report"],
            redact_fields: &["assumption"],
        },
        "capo.raise_blocker" => ReportToolSpec {
            display_name: "Raise Blocker",
            schema_json: "{\"input\":{\"blocker\":\"string\",\"requirement_id\":\"string?\",\"needs\":\"string?\"}}",
            risk: "medium",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:blocker"],
            redact_fields: &["blocker", "needs"],
        },
        "capo.request_review" => ReportToolSpec {
            display_name: "Request Review",
            schema_json: "{\"input\":{\"summary\":\"string\",\"requirement_id\":\"string?\",\"reviewer\":\"string?\"}}",
            risk: "medium",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:review_request"],
            redact_fields: &["summary"],
        },
        "capo.record_review" => ReportToolSpec {
            display_name: "Record Review",
            schema_json: "{\"input\":{\"outcome\":\"string\",\"summary\":\"string\",\"requirement_id\":\"string?\"}}",
            risk: "medium",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:review"],
            redact_fields: &["summary"],
        },
        "capo.record_validation" => ReportToolSpec {
            display_name: "Record Validation",
            schema_json: "{\"input\":{\"outcome\":\"string\",\"summary\":\"string\",\"requirement_id\":\"string?\"}}",
            risk: "medium",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:validation"],
            redact_fields: &["summary"],
        },
        "capo.complete_requirement" => ReportToolSpec {
            display_name: "Complete Requirement",
            schema_json: "{\"input\":{\"requirement_id\":\"string\",\"summary\":\"string\"}}",
            risk: "high",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:requirement_status"],
            redact_fields: &["summary"],
        },
        "capo.complete_subtask" => ReportToolSpec {
            display_name: "Complete Subtask",
            schema_json: "{\"input\":{\"subtask_id\":\"string\",\"summary\":\"string\"}}",
            risk: "high",
            mutates_state: true,
            extra_scopes: &["state:write:agent_report", "state:write:requirement_status"],
            redact_fields: &["summary"],
        },
        _ => return None,
    };
    Some(spec)
}

/// Narrow typed output every reporting tool emits: the classification `source`
/// (always `agent_reported`), whether the policy `accepted` it, the agent
/// `confidence`, and the `idempotency_key` it deduped on.
pub(crate) const AGENT_REPORT_OUTPUT_SCHEMA: &str = "{\"output\":{\"source\":\"string\",\"accepted\":\"boolean\",\"confidence\":\"integer\",\"idempotency_key\":\"string\"}}";

fn report_redaction_policy(fields: &[&str]) -> String {
    let mut fields = fields.to_vec();
    // Every report records its `body`-derived free text plus the rendered
    // output, so the credential scan always covers them.
    fields.push("body");
    fields.push("output");
    let quoted = fields
        .into_iter()
        .map(|field| format!("\"{field}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"strategy\":\"credential_scan\",\"fields\":[{quoted}]}}")
}

pub(crate) fn unknown_report_definition(tool_id: &str) -> ToolDefinition {
    ToolDefinition {
        tool_id: tool_id.to_string(),
        display_name: tool_id.to_string(),
        origin: "capo".to_string(),
        handler_kind: "agent_report".to_string(),
        schema_json: "{}".to_string(),
        output_schema: AGENT_REPORT_OUTPUT_SCHEMA.to_string(),
        required_scopes_json: json_array(vec!["tool:invoke:capo"]),
        risk: "medium".to_string(),
        redaction_policy_json: report_redaction_policy(&[]),
        exposure: "internal".to_string(),
        instrumentation_level: "none".to_string(),
        status: "unhealthy".to_string(),
        mutates_state: false,
    }
}

/// The idempotency key a report dedupes on. An agent-supplied `submission_id`
/// is authoritative (a retried identical submission carries the same id); when
/// absent, a stable hash over the session, tool, and body keeps a re-emitted
/// IDENTICAL report from double-recording on replay while two distinct reports
/// stay distinct.
fn report_idempotency_key(
    submission_id: &Option<String>,
    session_id: &str,
    tool_id: &str,
    body: &Value,
) -> String {
    match submission_id {
        Some(submission_id) => format!("agent-report:{submission_id}"),
        None => {
            let mut encoded = Vec::new();
            for field in [session_id, tool_id] {
                encoded.extend_from_slice(field.len().to_string().as_bytes());
                encoded.push(0);
                encoded.extend_from_slice(field.as_bytes());
            }
            // `to_string` on a serde_json::Value is stable for a given value
            // (object keys are emitted in insertion/sorted order consistently),
            // so an identical body hashes identically on replay.
            let body = body.to_string();
            encoded.extend_from_slice(body.as_bytes());
            format!(
                "agent-report:{}",
                content_hash(&encoded).replace("fnv1a64:", "")
            )
        }
    }
}
