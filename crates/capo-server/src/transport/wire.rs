use serde_json::Value;

use crate::{ServerError, ServerInputOrigin};

use super::{TransportError, TransportResult};

pub(super) fn input_origin_name(origin: ServerInputOrigin) -> &'static str {
    match origin {
        ServerInputOrigin::Cli => "cli",
        ServerInputOrigin::Dashboard => "dashboard",
        ServerInputOrigin::Mobile => "mobile",
        ServerInputOrigin::Voice => "voice",
        ServerInputOrigin::Api => "api",
        ServerInputOrigin::System => "system",
    }
}

pub(super) fn parse_input_origin(value: &str) -> TransportResult<ServerInputOrigin> {
    match value {
        "cli" => Ok(ServerInputOrigin::Cli),
        "dashboard" => Ok(ServerInputOrigin::Dashboard),
        "mobile" => Ok(ServerInputOrigin::Mobile),
        "voice" => Ok(ServerInputOrigin::Voice),
        "api" => Ok(ServerInputOrigin::Api),
        "system" => Ok(ServerInputOrigin::System),
        other => Err(TransportError::Protocol(format!(
            "unknown input origin: {other}"
        ))),
    }
}

pub(super) fn parse_value(line: &str) -> TransportResult<Value> {
    serde_json::from_str(line).map_err(TransportError::Json)
}

pub(super) fn required_value<'a>(value: &'a Value, key: &str) -> TransportResult<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key}")))
}

pub(super) fn required_string(value: &Value, key: &str) -> TransportResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} string")))
}

pub(super) fn optional_string(value: &Value, key: &str) -> TransportResult<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| TransportError::Protocol(format!("{key} must be a string"))),
    }
}

pub(super) fn optional_bool(value: &Value, key: &str) -> TransportResult<Option<bool>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| TransportError::Protocol(format!("{key} must be a boolean"))),
    }
}

pub(super) fn optional_i64(value: &Value, key: &str) -> TransportResult<Option<i64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| TransportError::Protocol(format!("{key} must be an integer"))),
    }
}

pub(super) fn required_i64(value: &Value, key: &str) -> TransportResult<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} integer")))
}

pub(super) fn required_string_array(value: &Value, key: &str) -> TransportResult<Vec<String>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| TransportError::Protocol(format!("{key} must contain strings")))
        })
        .collect()
}

pub(super) fn required_usize(value: &Value, key: &str) -> TransportResult<usize> {
    let number = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} integer")))?;
    usize::try_from(number).map_err(|_| TransportError::Protocol(format!("{key} is too large")))
}

pub(super) fn required_bool(value: &Value, key: &str) -> TransportResult<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} boolean")))
}

pub(super) fn transport_error_wire(error: &TransportError) -> (&'static str, String) {
    match error {
        TransportError::Io(error) => ("io", error.to_string()),
        TransportError::Json(error) => ("json", error.to_string()),
        TransportError::Protocol(message) => ("protocol", message.clone()),
        TransportError::Server(error) => server_error_wire(error),
        TransportError::Remote { kind, message } => ("remote", format!("{kind}: {message}")),
        TransportError::Cancelled { request_id } => (
            "cancelled",
            format!("request {request_id} cancelled by in-band cancel"),
        ),
        TransportError::Interrupted { session_id, reason } => (
            "interrupted",
            format!("turn for session {session_id} interrupted mid-turn: {reason}"),
        ),
    }
}

fn server_error_wire(error: &ServerError) -> (&'static str, String) {
    match error {
        ServerError::State(error) => ("state", format!("{error:?}")),
        ServerError::AdapterFixture(message) => ("adapter_fixture", message.clone()),
        ServerError::UnknownAgent { agent_name } => {
            ("unknown_agent", format!("unknown agent: {agent_name}"))
        }
        ServerError::AgentHasNoActiveSession { agent_name } => (
            "agent_has_no_active_session",
            format!("agent has no active session: {agent_name}"),
        ),
        ServerError::AgentAlreadyHasSession {
            agent_name,
            session_id,
            run_status,
        } => (
            "agent_already_has_session",
            format!(
                "agent {agent_name} already has session {session_id} with run_status={}",
                run_status.as_deref().unwrap_or("none")
            ),
        ),
        ServerError::SessionAlreadyExists { session_id } => (
            "session_already_exists",
            format!("session already exists: {session_id}"),
        ),
        ServerError::RunAlreadyExists { run_id } => (
            "run_already_exists",
            format!("run already exists: {run_id}"),
        ),
        ServerError::UnknownDispatchPlan { dispatch_plan_id } => (
            "unknown_dispatch_plan",
            format!("unknown dispatch plan: {dispatch_plan_id}"),
        ),
        ServerError::UnknownSession { session_id } => {
            ("unknown_session", format!("unknown session: {session_id}"))
        }
        ServerError::RunSessionMismatch {
            session_id,
            run_id,
            actual_session_id,
        } => (
            "run_session_mismatch",
            format!(
                "run {run_id} belongs to session {actual_session_id}, not requested session {session_id}"
            ),
        ),
        ServerError::AdapterSessionMismatch {
            session_id,
            session_adapter,
            requested_adapter,
        } => (
            "adapter_session_mismatch",
            format!(
                "session {session_id} uses adapter {session_adapter}, not requested adapter {requested_adapter}"
            ),
        ),
        ServerError::UnsupportedChatAdapter { adapter } => (
            "unsupported_chat_adapter",
            format!("unsupported chat adapter `{adapter}`; expected `fake` (default) or `codex`"),
        ),
        ServerError::UnknownGoal { goal_id } => {
            ("unknown_goal", format!("unknown goal: {goal_id}"))
        }
        ServerError::GoalCompleteNotALifecycleCommand { goal_id } => (
            "goal_complete_not_a_lifecycle_command",
            format!(
                "goal {goal_id} cannot be completed via a lifecycle command; \
                 completion is reachable only through the evidence-gated auditor"
            ),
        ),
        ServerError::IllegalGoalStatusTransition {
            goal_id,
            requested_status,
        } => (
            "illegal_goal_status_transition",
            format!(
                "goal {goal_id} cannot transition to `{requested_status}`; \
                 lifecycle statuses are active/paused/blocked/cleared"
            ),
        ),
        ServerError::UnclassifiableReportSource { source } => (
            "unclassifiable_report_source",
            format!(
                "report source `{source}` is neither an agent claim nor a recognized \
                 observed-evidence source"
            ),
        ),
        ServerError::InvalidRuntimeTargetField {
            field,
            value,
            expected,
        } => (
            "invalid_runtime_target_field",
            format!("invalid runtime target {field} `{value}`; expected {expected}"),
        ),
    }
}
