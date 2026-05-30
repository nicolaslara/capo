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

use capo_tools::{
    CapoToolResult, ToolAuditEvent, ToolExposure, ToolExposureRequest, ToolExposureResult,
    WrapperToolResult,
};

use super::*;

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
        let result = exposure.authorize_and_invoke(request, &self.permission_policy);
        let normalized = NormalizedToolResult::from_result(&result);
        let mut observed_event_kinds = Vec::with_capacity(normalized.events.len());
        for (index, audit_event) in normalized.events.iter().enumerate() {
            let Some(kind) = tool_audit_event_kind(audit_event) else {
                continue;
            };
            let projections = self.dispatch_event_projections(kind, scope, &normalized);
            self.state.append_event(
                scoped_event(
                    &format!(
                        "event-tool-dispatch-{}-{}-{}",
                        scope.session_id,
                        scope.tool_call_id,
                        dispatch_event_suffix(kind, index)
                    ),
                    kind,
                    &self.project_id,
                    &scope.task_id,
                    &scope.agent_id,
                    &scope.session_id,
                    &scope.run_id,
                )
                .with_turn(scope.turn_id.to_string())
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
                    updated_sequence: 0,
                })]
            }
            EventKind::ToolCallCompleted => {
                vec![ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                    tool_call_id: scope.tool_call_id.clone(),
                    session_id: scope.session_id.clone(),
                    turn_id: Some(scope.turn_id.to_string()),
                    tool_name: normalized.tool_name.clone(),
                    tool_origin: normalized.tool_origin.clone(),
                    status: normalized.status.clone(),
                    input_artifact_id: normalized.input_artifact_id.clone(),
                    output_artifact_id: normalized.output_artifact_id.clone(),
                    updated_sequence: 0,
                })]
            }
            _ => Vec::new(),
        }
    }
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
    events: Vec<ToolAuditEvent>,
}

impl NormalizedToolResult {
    fn from_result(result: &ToolExposureResult) -> Self {
        match result {
            ToolExposureResult::Capo(result) => Self::from_capo(result),
            ToolExposureResult::Runtime(result) => Self::from_runtime(result),
            ToolExposureResult::Fake(_) => {
                // The fake summary shim is not a real dispatch result; ACI1's
                // real path never routes it here.
                panic!(
                    "dispatch_tool_call received a fake tool result; the real loop \
                     dispatches Capo/Runtime tools only"
                )
            }
        }
    }

    fn from_capo(result: &CapoToolResult) -> Self {
        let status = if result.permission_decision.effect == "allow" {
            "completed".to_string()
        } else {
            "denied".to_string()
        };
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "capo".to_string(),
            status,
            input_artifact_id: None,
            output_artifact_id: artifact_or_none(&result.output_artifact_id),
            events: result.events.clone(),
        }
    }

    fn from_runtime(result: &WrapperToolResult) -> Self {
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "runtime".to_string(),
            status: result.status.clone(),
            input_artifact_id: result
                .input_artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.clone()),
            output_artifact_id: result
                .output_artifacts
                .first()
                .map(|artifact| artifact.artifact_id.clone()),
            events: result.events.clone(),
        }
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
