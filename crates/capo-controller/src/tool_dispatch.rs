//! ACI1: real tool dispatch wired into the loop.
//!
//! This is the seam the `tools-aci` workpad's first task lands: a tool-invoking
//! turn DRIVES the existing execution substrate (the same `append_event` /
//! projection path the loop already uses) instead of forking a second
//! pipeline. The controller takes a typed [`capo_tools::ToolExposureRequest`],
//! routes it through [`capo_tools::ToolExposure::authorize_and_invoke`] -- which
//! calls the REAL [`capo_tools::CapoToolRegistry::authorize_and_invoke`] /
//! [`capo_tools::RuntimeToolWrappers::authorize_and_invoke`], never the fake
//! summary shim -- and then NORMALIZES the resulting typed audit events onto the
//! canonical `tool.*`/`permission.*`/`capability.*` event kinds, keyed to the
//! turn.
//!
//! It deliberately reuses the loop's existing primitives (`scoped_event`,
//! `append_event`, `ToolCallProjection`) and does NOT call
//! `append_dispatch_run_exit`: a tool call annotates the in-flight run with its
//! observed evidence; it does not duplicate run-completion semantics.

use std::time::{SystemTime, UNIX_EPOCH};

use capo_tools::{
    AgentReportRecord, CapoToolResult, ToolAuditEvent, ToolExposure, ToolExposureRequest,
    ToolExposureResult, WrapperToolResult,
};

use super::*;

/// Wall-clock millis-since-epoch for the per-call `started_at`/`completed_at`
/// timing (ACI7), consistent with the ACI6 typed-test timing fields. A clock
/// before the epoch (impossible in practice) is clamped to 0.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a dispatched tool call hangs on the loop's scope tree, so the persisted
/// events carry the same task/agent/session/run/turn provenance the rest of the
/// loop uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDispatchScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub tool_call_id: ToolCallId,
}

/// The persisted outcome of one real tool dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDispatchOutcome {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub tool_origin: String,
    pub status: String,
    pub output_artifact_id: Option<String>,
    /// The canonical event kinds appended for this call, in order, so a test can
    /// assert the real audit event sequence.
    pub observed_event_kinds: Vec<String>,
    /// The raw typed result, so callers can inspect the narrow output.
    pub result: ToolExposureResult,
}

impl FakeBoundaryController {
    /// Dispatch a real tool call through `exposure.authorize_and_invoke` and
    /// persist the canonical tool-call event sequence keyed to the turn.
    ///
    /// `exposure` is the REAL registry/wrappers handle (the production
    /// [`RealBoundaryController`] holds one; the test-only fake exposure is never
    /// the default). The typed result's audit events are normalized onto the
    /// loop's existing event kinds, so the persisted sequence is the same one the
    /// `tool-exposure.md` design specifies and the loop's
    /// `reconstruct_turn_finished` already consumes.
    pub fn dispatch_tool_call(
        &self,
        exposure: &ToolExposure,
        scope: &ToolDispatchScope,
        request: ToolExposureRequest,
    ) -> StateResult<ToolDispatchOutcome> {
        // ACI7: capture wall-clock timing around the real authorize+invoke so the
        // persisted `ToolCall` projection carries `started_at`/`completed_at` for
        // later evaluation.
        let started_at = epoch_millis();
        let result = exposure.authorize_and_invoke(request, &self.permission_policy);
        let completed_at = epoch_millis();
        let normalized = NormalizedToolResult::from_result(&result);
        // ACI7: the per-call provenance that ties command -> turn -> permission ->
        // tool -> artifact into one queryable chain. The `correlation_id` is the
        // turn-scoped identity already stamped on every event's item ref (the
        // tool_call_id), and the permission-decision / grant-use ids pin the
        // authorization that allowed (or denied) the call.
        let provenance = capo_state::ToolCallProvenance {
            correlation_id: Some(dispatch_correlation_id(scope)),
            permission_decision_id: normalized.permission_decision_id.clone(),
            capability_grant_use_id: normalized
                .capability_grant_id
                .as_ref()
                .map(|grant_id| dispatch_grant_use_id(scope, grant_id)),
            started_at: Some(started_at),
            completed_at: Some(completed_at),
        };
        // The persistable canonical kinds for this dispatch, in order. Audit
        // events with no loop counterpart (`tool.call_canceled`/`tool.call_failed`)
        // are dropped here, but their terminal STATUS is still applied to the
        // projection below so a denied/failed call never sticks at "requested".
        let persistable: Vec<(usize, EventKind, &ToolAuditEvent)> = normalized
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, audit_event)| {
                tool_audit_event_kind(audit_event).map(|kind| (index, kind, audit_event))
            })
            .collect();
        // Whether the registry/wrappers reached a `tool.call_completed`. If not
        // (deny/fail), the loop never runs the `ToolCallCompleted` projection
        // branch, so we must stamp the terminal projection onto the LAST
        // persisted event instead -- otherwise the persisted projection (read by
        // `tool_calls_for_session`, dashboards, recovery) stays at "requested"
        // even though the dispatch outcome is "denied"/"failed".
        let reaches_completed = persistable
            .iter()
            .any(|(_, kind, _)| matches!(kind, EventKind::ToolCallCompleted));
        let last_index = persistable.len().saturating_sub(1);
        let mut observed_event_kinds = Vec::with_capacity(persistable.len());
        for (position, (index, kind, audit_event)) in persistable.iter().enumerate() {
            let kind = *kind;
            let mut projections =
                self.dispatch_event_projections(kind, scope, &normalized, &provenance);
            // Terminal projection for the non-completed paths: attach the final
            // `ToolCall` projection (status == normalized.status, i.e.
            // "denied"/"failed") to the dispatch's last persisted event so every
            // path drives the projection to its true terminal status.
            if !reaches_completed && position == last_index {
                projections.push(terminal_tool_call_projection(
                    scope,
                    &normalized,
                    &provenance,
                ));
            }
            self.state.append_event(
                scoped_event(
                    &format!(
                        "event-tool-dispatch-{}-{}-{}",
                        scope.session_id,
                        scope.tool_call_id,
                        dispatch_event_suffix(kind, *index)
                    ),
                    kind,
                    &self.project_id,
                    &scope.task_id,
                    &scope.agent_id,
                    &scope.session_id,
                    &scope.run_id,
                )
                .with_turn(scope.turn_id.to_string())
                // Stamp a shared item ref (the tool_call_id) across every event of
                // this one tool call so `persisted_turn_ref`/`reconstruct_turn_finished`
                // dedup collapse them to a SINGLE observed tool ref per call,
                // matching the loop's documented replay-identity invariant.
                .with_item(scope.tool_call_id.to_string())
                .with_payload(dispatch_event_payload(
                    kind,
                    scope,
                    &normalized,
                    audit_event,
                )),
                &projections,
            )?;
            observed_event_kinds.push(kind.as_str().to_string());
        }
        Ok(ToolDispatchOutcome {
            tool_call_id: scope.tool_call_id.clone(),
            tool_name: normalized.tool_name,
            tool_origin: normalized.tool_origin,
            status: normalized.status,
            output_artifact_id: normalized.output_artifact_id,
            observed_event_kinds,
            result,
        })
    }

    fn dispatch_event_projections(
        &self,
        kind: EventKind,
        scope: &ToolDispatchScope,
        normalized: &NormalizedToolResult,
        provenance: &capo_state::ToolCallProvenance,
    ) -> Vec<ProjectionRecord> {
        match kind {
            EventKind::ToolCallRequested => {
                vec![ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                    tool_call_id: scope.tool_call_id.clone(),
                    session_id: scope.session_id.clone(),
                    turn_id: Some(scope.turn_id.to_string()),
                    tool_name: normalized.tool_name.clone(),
                    tool_origin: normalized.tool_origin.clone(),
                    status: "requested".to_string(),
                    input_artifact_id: normalized.input_artifact_id.clone(),
                    output_artifact_id: None,
                    provenance: provenance.clone(),
                    updated_sequence: 0,
                })]
            }
            EventKind::ToolObservationRecorded => {
                // ACI8: persist the agent report as a DISTINCT observation class
                // tagged `source=agent_reported` (carrying confidence), separate
                // from observed runtime/adapter evidence, so completion is never
                // reachable by agent assertion alone. Observed tools (Capo
                // registry / runtime wrappers) carry no `agent_report`, so this
                // projection is emitted ONLY for the reporting surface.
                match &normalized.agent_report {
                    Some(report) => vec![ProjectionRecord::ToolObservation(
                        capo_state::ToolObservationProjection {
                            tool_observation_id: format!("agent-report-obs-{}", scope.tool_call_id),
                            session_id: scope.session_id.clone(),
                            tool_call_id: Some(scope.tool_call_id.clone()),
                            source: report.source.clone(),
                            external_tool_ref: None,
                            tool_name: normalized.tool_name.clone(),
                            observed_status: "reported".to_string(),
                            instrumentation_level: "structured_observed".to_string(),
                            confidence: report.confidence.to_string(),
                            raw_event_hash: format!("agent-report:{}", scope.tool_call_id),
                            artifact_id: None,
                            updated_sequence: 0,
                        },
                    )],
                    None => Vec::new(),
                }
            }
            EventKind::ToolCallCompleted => {
                vec![terminal_tool_call_projection(scope, normalized, provenance)]
            }
            _ => Vec::new(),
        }
    }
}

/// The stable correlation id for one tool dispatch (ACI7): a turn-scoped value
/// that ties the command -> turn -> permission -> tool -> artifact chain. The
/// tool_call_id is the join key already stamped on every event's item ref; the
/// session/run/turn prefix keeps the id self-describing for cross-projection
/// queries.
fn dispatch_correlation_id(scope: &ToolDispatchScope) -> String {
    format!(
        "corr-{}-{}-{}-{}",
        scope.session_id, scope.run_id, scope.turn_id, scope.tool_call_id
    )
}

/// The per-invocation capability-grant-use id (ACI7): the grant the permission
/// decision issued, scoped to THIS tool call, so two calls that reuse the same
/// grant still carry distinct grant-use ids.
fn dispatch_grant_use_id(scope: &ToolDispatchScope, grant_id: &str) -> String {
    format!("grant-use-{}-{}", scope.tool_call_id, grant_id)
}

/// The terminal `ToolCall` projection for a dispatch: status == the normalized
/// dispatch status (`completed`/`denied`/`failed`). Used both for the
/// `tool.call_completed` (allow+success) branch AND, for the deny/fail paths that
/// carry no `tool.call_completed` event, attached to the last persisted event so
/// the persisted projection always reaches its true terminal status.
fn terminal_tool_call_projection(
    scope: &ToolDispatchScope,
    normalized: &NormalizedToolResult,
    provenance: &capo_state::ToolCallProvenance,
) -> ProjectionRecord {
    ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
        tool_call_id: scope.tool_call_id.clone(),
        session_id: scope.session_id.clone(),
        turn_id: Some(scope.turn_id.to_string()),
        tool_name: normalized.tool_name.clone(),
        tool_origin: normalized.tool_origin.clone(),
        status: normalized.status.clone(),
        input_artifact_id: normalized.input_artifact_id.clone(),
        output_artifact_id: normalized.output_artifact_id.clone(),
        provenance: provenance.clone(),
        updated_sequence: 0,
    })
}

/// A variant-erased view of the typed tool result, so the canonical normalization
/// is one code path regardless of whether the tool was Capo-owned or a runtime
/// wrapper.
struct NormalizedToolResult {
    tool_name: String,
    tool_origin: String,
    status: String,
    input_artifact_id: Option<String>,
    output_artifact_id: Option<String>,
    /// The capability grant the permission decision issued for this call (ACI7).
    /// `None` only for a degenerate result with no decision.
    capability_grant_id: Option<String>,
    /// The permission-decision id pinned to this call (ACI7), derived from the
    /// issued grant so the projection can join back to the authorization.
    permission_decision_id: Option<String>,
    /// ACI8: the agent-report classification for a `GO2` reporting tool, carrying
    /// the `agent_reported` source tag and the agent's self-declared confidence.
    /// `None` for observed tools (Capo registry / runtime wrappers), so a report
    /// is never persisted indistinguishably from observed evidence.
    agent_report: Option<AgentReportObservation>,
    events: Vec<ToolAuditEvent>,
}

/// ACI8: the distinct classification an agent report carries onto the persisted
/// `tool.observation_recorded` projection: `source=agent_reported` (never an
/// observed-evidence source) plus the agent's self-declared confidence.
struct AgentReportObservation {
    source: String,
    confidence: i64,
}

impl NormalizedToolResult {
    fn from_result(result: &ToolExposureResult) -> Self {
        match result {
            ToolExposureResult::Capo(result) => Self::from_capo(result),
            ToolExposureResult::Runtime(result) => Self::from_runtime(result),
            ToolExposureResult::AgentReport(result) => Self::from_agent_report(result),
            ToolExposureResult::Fake(_) => {
                // The fake summary shim is not a real dispatch result; ACI1's
                // real path never routes it here.
                panic!(
                    "dispatch_tool_call received a fake tool result; the real loop \
                     dispatches Capo/Runtime/AgentReport tools only"
                )
            }
        }
    }

    fn from_agent_report(result: &AgentReportRecord) -> Self {
        let status = if result.accepted {
            "completed".to_string()
        } else {
            "denied".to_string()
        };
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            // ACI8: a Capo-owned reporting tool, but tagged distinctly through
            // the agent-report observation below so a report's persisted
            // observation reads `source=agent_reported`, not observed evidence.
            tool_origin: "capo".to_string(),
            status,
            input_artifact_id: None,
            output_artifact_id: None,
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            agent_report: Some(AgentReportObservation {
                source: result.source.clone(),
                confidence: result.confidence,
            }),
            events: result.events.clone(),
        }
    }

    fn from_capo(result: &CapoToolResult) -> Self {
        let status = if result.permission_decision.effect == "allow" {
            "completed".to_string()
        } else {
            "denied".to_string()
        };
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "capo".to_string(),
            status,
            input_artifact_id: None,
            output_artifact_id: artifact_or_none(&result.output_artifact_id),
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            agent_report: None,
            events: result.events.clone(),
        }
    }

    fn from_runtime(result: &WrapperToolResult) -> Self {
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "runtime".to_string(),
            // Normalize onto the dispatch terminal-status vocabulary
            // (`completed`/`failed`/`denied`) that downstream consumers
            // (dashboards, safety-gates score_run, goal-autonomy evidence) match
            // on. A wrapper may carry finer-grained terminal statuses for the
            // loop (e.g. `precondition_failed`, or the runtime's `exited`), but
            // the persisted dispatch status must stay in the shared vocabulary so
            // a non-completed outcome is never mis-bucketed as a completion.
            status: normalize_runtime_status(&result.status),
            input_artifact_id: result
                .input_artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.clone()),
            output_artifact_id: result
                .output_artifacts
                .first()
                .map(|artifact| artifact.artifact_id.clone()),
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            agent_report: None,
            events: result.events.clone(),
        }
    }
}

/// Derive the stable permission-decision id from the issued capability grant
/// (ACI7). The grant id is the decision's identity; the `decision-` prefix marks
/// it as the decision-projection join key rather than the raw grant.
fn permission_decision_id(grant_id: &str) -> String {
    format!("decision-{grant_id}")
}

/// Fold a runtime wrapper's terminal status onto the dispatch terminal-status
/// vocabulary downstream consumers match on.
///
/// Most wrapper statuses are already canonical (`completed`/`denied`, and the
/// process-runner's `exited`/`failed` which the loop understands). The
/// non-completing guards (`precondition_failed` for a stale file_write, and
/// `no_match` for an apply_patch hunk no strategy could locate) are real
/// terminal FAILURES for dispatch/projection purposes (no write, no artifact),
/// so they are folded onto `failed` here; the precise `precondition_failed` /
/// `no_match` semantics (expected/actual hashes, rejected hunk index, nearest
/// candidate) still travel on the wrapper result's own status and typed output
/// for the loop to reflect and retry.
fn normalize_runtime_status(status: &str) -> String {
    match status {
        "precondition_failed" | "no_match" => "failed".to_string(),
        other => other.to_string(),
    }
}

fn artifact_or_none(artifact_id: &str) -> Option<String> {
    if artifact_id == "none" {
        None
    } else {
        Some(artifact_id.to_string())
    }
}

/// Map a typed tool-audit event onto the canonical loop [`EventKind`], or `None`
/// for audit events that have no persisted-loop counterpart (e.g. the
/// `tool.call_canceled` denial marker, surfaced through the projected status
/// instead).
fn tool_audit_event_kind(event: &ToolAuditEvent) -> Option<EventKind> {
    match event.kind.as_str() {
        "tool.call_requested" => Some(EventKind::ToolCallRequested),
        "permission.requested" => Some(EventKind::PermissionRequested),
        "permission.decided" => Some(EventKind::PermissionDecided),
        "capability.grant_used" => Some(EventKind::CapabilityGrantUsed),
        "tool.invocation_started" => Some(EventKind::ToolInvocationStarted),
        "tool.output_artifact_recorded" => Some(EventKind::ToolOutputArtifactRecorded),
        "tool.output_observed" => Some(EventKind::ToolOutputObserved),
        // ACI8: an agent report records a distinct `agent_reported` observation,
        // not observed runtime/adapter evidence.
        "tool.observation_recorded" => Some(EventKind::ToolObservationRecorded),
        "tool.call_completed" => Some(EventKind::ToolCallCompleted),
        "tool.result_delivered" => Some(EventKind::ToolResultDelivered),
        _ => None,
    }
}

/// A short, stable event-id suffix per canonical kind. The index disambiguates
/// the rare case where a kind repeats within one call.
fn dispatch_event_suffix(kind: EventKind, index: usize) -> String {
    format!("{}-{index}", kind.as_str().replace('.', "-"))
}

fn dispatch_event_payload(
    kind: EventKind,
    scope: &ToolDispatchScope,
    normalized: &NormalizedToolResult,
    audit_event: &ToolAuditEvent,
) -> String {
    match kind {
        EventKind::ToolOutputArtifactRecorded => format!(
            "{{\"tool_call_id\":\"{}\",\"output_artifact_id\":\"{}\",\"redaction_state\":\"{}\"}}",
            scope.tool_call_id,
            normalized.output_artifact_id.as_deref().unwrap_or("none"),
            escape_json(&audit_event.status)
        ),
        EventKind::ToolOutputObserved => format!(
            "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"status\":\"{}\"}}",
            scope.tool_call_id,
            escape_json(&normalized.tool_name),
            escape_json(&audit_event.status)
        ),
        // ACI8: the agent-report observation payload records its distinct
        // `source` (the audit event's status carries `agent_reported`) so the
        // persisted event is classifiable without re-deriving from the tool name.
        EventKind::ToolObservationRecorded => format!(
            "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"source\":\"{}\"}}",
            scope.tool_call_id,
            escape_json(&normalized.tool_name),
            escape_json(&audit_event.status)
        ),
        EventKind::ToolCallCompleted => format!(
            "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"output_artifact_id\":\"{}\"}}",
            scope.tool_call_id,
            escape_json(&normalized.tool_name),
            normalized.output_artifact_id.as_deref().unwrap_or("none")
        ),
        _ => format!(
            "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"status\":\"{}\"}}",
            scope.tool_call_id,
            escape_json(&normalized.tool_name),
            escape_json(&audit_event.status)
        ),
    }
}
