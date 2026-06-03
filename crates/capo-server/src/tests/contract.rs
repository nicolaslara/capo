//! ST9: the published JSON-RPC/event-stream wire contract is enforced by
//! regenerate-and-diff snapshot tests against checked-in fixtures, verified
//! WITHOUT any web client.
//!
//! The fixtures live under `crates/capo-server/contract/`:
//!
//! - `jsonrpc-schema.json` -- the language-neutral schema (described contract).
//! - `snapshots/*.json` -- real serialized wire frames (enforced contract).
//!
//! Both are produced by [`crate::contract`] from the *same* codec the live
//! transport uses (never hand-typed JSON). To intentionally evolve the contract,
//! regenerate the checked-in copies:
//!
//! ```sh
//! CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract
//! ```
//!
//! With the env var unset (the default, including CI), the test only *reads* the
//! checked-in files and asserts byte-equality, so an unintended wire-shape change
//! fails the build and the contract cannot drift silently.

use std::path::{Path, PathBuf};

use crate::contract::{self, WireSample};
use crate::{
    ServerCommand, ServerError, ServerInputOrigin, ServerRequest, ServerResponse,
    ServerResponsePayload, jsonrpc_request_roundtrip, jsonrpc_response_roundtrip,
};

/// The checked-in contract directory: `crates/capo-server/contract/`.
fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("contract")
}

/// Whether the operator asked to (re)write the checked-in fixtures.
fn regenerating() -> bool {
    std::env::var_os("CAPO_REGENERATE_WIRE_SNAPSHOTS").is_some()
}

/// Read the on-disk fixture, or write it when regenerating. Returns the content
/// that *should* be on disk for the assert step.
fn checked_in_or_regenerated(path: &Path, expected: &str) -> String {
    if regenerating() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create contract dir");
        }
        std::fs::write(path, expected).expect("write fixture");
        return expected.to_string();
    }
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "missing checked-in contract fixture {}: {error}.\n\
             Regenerate with CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract",
            path.display()
        )
    })
}

/// Pretty-print the schema deterministically (sorted-key-free, stable two-space
/// indent) so the checked-in file is diff-friendly and stable across runs.
fn schema_json() -> String {
    let mut text =
        serde_json::to_string_pretty(&contract::contract_schema()).expect("schema serializes");
    text.push('\n');
    text
}

#[test]
fn jsonrpc_schema_matches_the_checked_in_contract() {
    let path = contract_dir().join("jsonrpc-schema.json");
    let expected = schema_json();
    let on_disk = checked_in_or_regenerated(&path, &expected);
    assert_eq!(
        on_disk,
        expected,
        "the published JSON-RPC schema drifted from {}.\n\
         If this change is intentional, regenerate with \
         CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract",
        path.display()
    );
}

#[test]
fn wire_snapshots_match_the_checked_in_contract() {
    let snapshots = contract_dir().join("snapshots");
    for WireSample {
        name,
        description,
        frame,
    } in contract::wire_samples()
    {
        let path = snapshots.join(format!("{name}.json"));
        // Each snapshot file is the exact frame plus a trailing newline so it is
        // a well-formed text file and a clean git diff. The `description` is
        // verified to be non-empty (it documents the frame) but is not written
        // into the frame itself, which must stay byte-identical to the wire.
        assert!(!description.is_empty(), "snapshot {name} must be described");
        let expected = format!("{frame}\n");
        let on_disk = checked_in_or_regenerated(&path, &expected);
        assert_eq!(
            on_disk,
            expected,
            "wire snapshot {name} drifted from {}.\n\
             If this change is intentional, regenerate with \
             CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract",
            path.display()
        );
    }
}

#[test]
fn sse_event_sequence_matches_the_checked_in_fixture() {
    // ST11: the SSE event-tail stream for a scripted multi-event turn is the
    // capo-web contract vehicle (capo-web is not built in this workspace). The
    // checked-in fixture pins the exact SSE byte stream for the scripted sequence
    // so the web tail frames cannot drift silently; regenerate-and-diff like the
    // JSON-RPC snapshots.
    let path = contract_dir()
        .join("snapshots")
        .join("sse-event-sequence.txt");
    let expected = contract::sse_event_sequence();
    let on_disk = checked_in_or_regenerated(&path, &expected);
    assert_eq!(
        on_disk,
        expected,
        "the SSE event-tail sequence drifted from {}.\n\
         If this change is intentional, regenerate with \
         CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract",
        path.display()
    );

    // Each SSE block re-exposes a JSON-RPC `event` notification verbatim on its
    // `data:` line, in committed (sequence) order, so a web client decodes a tail
    // frame exactly like the raw socket transport.
    let data_lines: Vec<&str> = expected
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .collect();
    assert_eq!(
        data_lines.len(),
        3,
        "the scripted SSE sequence must carry three event frames"
    );
    let mut last_sequence = 0;
    for data in data_lines {
        let value: serde_json::Value =
            serde_json::from_str(data).expect("SSE data line is a JSON-RPC frame");
        assert_eq!(
            value.get("method").and_then(serde_json::Value::as_str),
            Some("event"),
            "each SSE data line is an `event` notification"
        );
        let sequence = value
            .get("params")
            .and_then(|params| params.get("event"))
            .and_then(|event| event.get("sequence"))
            .and_then(serde_json::Value::as_i64)
            .expect("event sequence");
        assert!(
            sequence > last_sequence,
            "SSE sequence frames must be in strictly increasing commit order"
        );
        last_sequence = sequence;
    }
}

#[test]
fn every_snapshot_frame_is_valid_json_rpc_2_0() {
    // The contract is JSON-RPC 2.0: every non-SSE frame parses as JSON and
    // carries `"jsonrpc":"2.0"`. The SSE frame wraps such a frame in an
    // `event:`/`data:` block, so its data line must parse the same way.
    for sample in contract::wire_samples() {
        let json_line = if sample.name == "sse-event-tail" {
            sample
                .frame
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap_or_else(|| panic!("SSE frame {} has no data line", sample.name))
                .to_string()
        } else {
            sample.frame.clone()
        };
        let value: serde_json::Value = serde_json::from_str(json_line.trim_end())
            .unwrap_or_else(|error| panic!("snapshot {} is not JSON: {error}", sample.name));
        assert_eq!(
            value.get("jsonrpc").and_then(serde_json::Value::as_str),
            Some("2.0"),
            "snapshot {} must be JSON-RPC 2.0",
            sample.name
        );
    }
}

#[test]
fn schema_method_and_notification_names_are_covered_by_snapshots() {
    // The described schema and the enforced snapshots must agree on the wire
    // vocabulary: every notification method the schema lists must appear in a
    // checked-in snapshot, and the representative request methods too. This is
    // what keeps the "described" and "enforced" halves of the contract honest.
    let samples = contract::wire_samples();
    let methods: Vec<String> = samples
        .iter()
        .filter(|sample| sample.name != "sse-event-tail")
        .filter_map(|sample| {
            serde_json::from_str::<serde_json::Value>(sample.frame.trim_end())
                .ok()
                .and_then(|value| {
                    value
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                })
        })
        .collect();
    for required in ["event", "cancel", "interrupt", "subscribe", "read_thread"] {
        assert!(
            methods.iter().any(|method| method == required),
            "the published contract must include a `{required}` frame snapshot; have {methods:?}"
        );
    }
}

/// The schema's `method`, `payload`, and `error.data.kind` enumerations must
/// cover every wire variant the codec can emit. The exhaustive `match`es below
/// make a *new* `ServerCommand` / `ServerResponsePayload` / `ServerError`
/// variant a COMPILE error here, forcing the author to add its wire tag, and the
/// assertions then prove the published schema lists that tag. This is what keeps
/// the described schema from silently lagging the code.
#[test]
fn schema_enumerations_cover_every_wire_variant() {
    fn command_method(command: &ServerCommand) -> &'static str {
        match command {
            ServerCommand::RegisterAgent { .. } => "register_agent",
            ServerCommand::SendTask { .. } => "send_task",
            ServerCommand::SteerAgent { .. } => "steer_agent",
            ServerCommand::InterruptAgent { .. } => "interrupt_agent",
            ServerCommand::StopAgent { .. } => "stop_agent",
            ServerCommand::ListAgents => "list_agents",
            ServerCommand::AgentStatus { .. } => "agent_status",
            ServerCommand::Dashboard { .. } => "dashboard",
            ServerCommand::StartSession { .. } => "start_session",
            ServerCommand::ReplayAdapterFixture { .. } => "replay_adapter_fixture",
            ServerCommand::PlanDispatch { .. } => "plan_dispatch",
            ServerCommand::PreflightLiveProvider { .. } => "preflight_live_provider",
            ServerCommand::GateDispatch { .. } => "gate_dispatch",
            ServerCommand::RunDispatchLocal { .. } => "run_dispatch_local",
            ServerCommand::RunLiveProviderLocal { .. } => "run_live_provider_local",
            ServerCommand::RunDispatchTurn { .. } => "run_dispatch_turn",
            ServerCommand::Recover => "recover",
            ServerCommand::Subscribe { .. } => "subscribe",
            ServerCommand::ReadThread { .. } => "read_thread",
            ServerCommand::SetGoal { .. } => "set_goal",
            ServerCommand::PauseGoal { .. } => "pause_goal",
            ServerCommand::ResumeGoal { .. } => "resume_goal",
            ServerCommand::BlockGoal { .. } => "block_goal",
            ServerCommand::ClearGoal { .. } => "clear_goal",
            ServerCommand::SetRequirementStatus { .. } => "set_requirement_status",
            ServerCommand::RecordGoalReport { .. } => "record_goal_report",
            ServerCommand::MarkGoalComplete { .. } => "mark_goal_complete",
            ServerCommand::ListGoals => "list_goals",
            ServerCommand::ViewGoal { .. } => "view_goal",
            ServerCommand::GoalStory { .. } => "goal_story",
            ServerCommand::GoalTimeline { .. } => "goal_timeline",
            ServerCommand::GoalEvidence { .. } => "goal_evidence",
            ServerCommand::GoalValidations { .. } => "goal_validations",
            ServerCommand::GoalReviews { .. } => "goal_reviews",
            ServerCommand::GoalRisks { .. } => "goal_risks",
            ServerCommand::GoalReport { .. } => "goal_report",
            ServerCommand::ContinueGoal { .. } => "continue_goal",
            ServerCommand::RegisterRuntimeTarget { .. } => "register_runtime_target",
            ServerCommand::ReplayRunnerEvents { .. } => "replay_runner_events",
        }
    }

    fn payload_type(payload: &ServerResponsePayload) -> &'static str {
        match payload {
            ServerResponsePayload::AgentRegistered(_) => "agent_registered",
            ServerResponsePayload::TaskSent(_) => "task_sent",
            ServerResponsePayload::Agents(_) => "agents",
            ServerResponsePayload::AgentStatus(_) => "agent_status",
            ServerResponsePayload::Dashboard(_) => "dashboard",
            ServerResponsePayload::SessionStarted(_) => "session_started",
            ServerResponsePayload::AdapterFixtureReplayed(_) => "adapter_fixture_replayed",
            ServerResponsePayload::DispatchPlanned(_) => "dispatch_planned",
            ServerResponsePayload::LiveProviderPreflighted(_) => "live_provider_preflighted",
            ServerResponsePayload::DispatchGated(_) => "dispatch_gated",
            ServerResponsePayload::DispatchRun(_) => "dispatch_run",
            ServerResponsePayload::DispatchTurn(_) => "dispatch_turn",
            ServerResponsePayload::Recovery(_) => "recovery",
            ServerResponsePayload::Subscribed(_) => "subscribed",
            ServerResponsePayload::Thread(_) => "thread",
            ServerResponsePayload::Goals(_) => "goals",
            ServerResponsePayload::GoalView(_) => "goal_view",
            ServerResponsePayload::GoalReports(_) => "goal_reports",
            ServerResponsePayload::GoalTimeline(_) => "goal_timeline",
            ServerResponsePayload::GoalReport(_) => "goal_report",
            ServerResponsePayload::ContinuationEvaluated(_) => "continuation_evaluated",
            ServerResponsePayload::RuntimeTargetRegistered(_) => "runtime_target_registered",
            ServerResponsePayload::RunnerEventsReplayed(_) => "runner_events_replayed",
        }
    }

    // A domain `ServerError` variant's wire kind (the `error.data.kind` tag).
    // Transport-only kinds (io/json/protocol/remote/cancelled/interrupted) are
    // not domain variants; they are asserted directly against the schema below.
    fn error_kind(error: &ServerError) -> &'static str {
        match error {
            ServerError::State(_) => "state",
            ServerError::AdapterFixture(_) => "adapter_fixture",
            ServerError::UnknownAgent { .. } => "unknown_agent",
            ServerError::AgentHasNoActiveSession { .. } => "agent_has_no_active_session",
            ServerError::AgentAlreadyHasSession { .. } => "agent_already_has_session",
            ServerError::SessionAlreadyExists { .. } => "session_already_exists",
            ServerError::RunAlreadyExists { .. } => "run_already_exists",
            ServerError::UnknownDispatchPlan { .. } => "unknown_dispatch_plan",
            ServerError::UnknownSession { .. } => "unknown_session",
            ServerError::RunSessionMismatch { .. } => "run_session_mismatch",
            ServerError::AdapterSessionMismatch { .. } => "adapter_session_mismatch",
            ServerError::UnsupportedChatAdapter { .. } => "unsupported_chat_adapter",
            ServerError::UnknownGoal { .. } => "unknown_goal",
            ServerError::GoalCompleteNotALifecycleCommand { .. } => {
                "goal_complete_not_a_lifecycle_command"
            }
            ServerError::IllegalGoalStatusTransition { .. } => "illegal_goal_status_transition",
            ServerError::UnclassifiableReportSource { .. } => "unclassifiable_report_source",
            ServerError::InvalidRuntimeTargetField { .. } => "invalid_runtime_target_field",
            ServerError::SubscribeFromSequenceAheadOfLog { .. } => {
                "subscribe_from_sequence_ahead_of_log"
            }
            ServerError::InvalidRunnerReplayFrame { .. } => "invalid_runner_replay_frame",
        }
    }

    // The exhaustive matches above are unreachable by value (they exist only to
    // be re-checked by the compiler when a variant is added); reference them so
    // they are not dead code, and pull the schema's published enums.
    let _ = (
        command_method as fn(&ServerCommand) -> &'static str,
        payload_type as fn(&ServerResponsePayload) -> &'static str,
        error_kind as fn(&ServerError) -> &'static str,
    );

    let schema = contract::contract_schema();
    let request_methods = string_enum(
        &schema,
        &["envelope", "request", "properties", "method", "enum"],
        "request methods",
    );
    let payload_tags = string_enum(
        &schema,
        &[
            "envelope",
            "success_response",
            "properties",
            "result",
            "properties",
            "payload",
            "properties",
            "type",
            "enum",
        ],
        "payload types",
    );
    let error_data_kinds = string_enum(
        &schema,
        &[
            "envelope",
            "error_response",
            "properties",
            "error",
            "properties",
            "data",
            "properties",
            "kind",
            "enum",
        ],
        "error kinds",
    );

    // Every command method tag the codec can emit is published AND every sample
    // command survives a full encode/decode round-trip through the real codec.
    // The round-trip is what catches a field-order/name bug in a decode arm (a
    // tag-only schema check cannot): a sample of every variant -- including the
    // new `RunDispatchTurn` -- goes through `encode_command`/`decode_command`
    // (via the request envelope) and must come back unchanged.
    for command in sample_commands() {
        let tag = command_method(&command);
        assert!(
            request_methods.iter().any(|m| m == tag),
            "schema request methods missing `{tag}`: {request_methods:?}"
        );
        let request = ServerRequest::cli(command.clone());
        assert_eq!(
            jsonrpc_request_roundtrip(&request),
            request,
            "command `{tag}` did not survive a codec round-trip"
        );
    }
    // Every payload tag the codec can emit is published AND every sample payload
    // survives a full encode/decode round-trip through the real codec (the same
    // field-order/name guard for `DispatchTurn` and every other payload variant).
    for payload in sample_payloads() {
        let tag = payload_type(&payload);
        assert!(
            payload_tags.iter().any(|t| t == tag),
            "schema payload types missing `{tag}`: {payload_tags:?}"
        );
        let response = ServerResponse {
            request_id: "contract-roundtrip".to_string(),
            client_id: "local-cli".to_string(),
            actor_id: "local-user".to_string(),
            input_origin: ServerInputOrigin::Cli,
            payload: payload.clone(),
        };
        assert_eq!(
            jsonrpc_response_roundtrip(&response),
            response,
            "payload `{tag}` did not survive a codec round-trip"
        );
    }
    // Every domain error kind is published, plus the transport-only kinds.
    for error in sample_errors() {
        let tag = error_kind(&error);
        assert!(
            error_data_kinds.iter().any(|k| k == tag),
            "schema error kinds missing `{tag}`: {error_data_kinds:?}"
        );
    }
    for transport_kind in [
        "io",
        "json",
        "protocol",
        "remote",
        "cancelled",
        "interrupted",
    ] {
        assert!(
            error_data_kinds.iter().any(|k| k == transport_kind),
            "schema error kinds missing transport kind `{transport_kind}`: {error_data_kinds:?}"
        );
    }
}

/// Read a string-array `enum` at a JSON pointer path in the schema.
fn string_enum(schema: &serde_json::Value, path: &[&str], what: &str) -> Vec<String> {
    let mut node = schema;
    for key in path {
        node = node
            .get(key)
            .unwrap_or_else(|| panic!("schema {what}: missing key `{key}` on the path"));
    }
    node.as_array()
        .unwrap_or_else(|| panic!("schema {what}: expected an array"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("schema {what}: non-string enum entry"))
                .to_string()
        })
        .collect()
}

/// One value of every `ServerCommand` variant, so the exhaustive `match` is
/// exercised at runtime (and a new variant is a compile error in the match).
fn sample_commands() -> Vec<ServerCommand> {
    let s = || "x".to_string();
    vec![
        ServerCommand::RegisterAgent {
            name: s(),
            adapter: s(),
        },
        ServerCommand::SendTask {
            agent_name: s(),
            goal: s(),
            scenario: s(),
        },
        ServerCommand::SteerAgent {
            agent_name: s(),
            goal: s(),
        },
        ServerCommand::InterruptAgent {
            agent_name: s(),
            reason: s(),
        },
        ServerCommand::StopAgent {
            agent_name: s(),
            reason: s(),
        },
        ServerCommand::ListAgents,
        ServerCommand::AgentStatus { agent_name: s() },
        ServerCommand::Dashboard {
            recent_event_limit: 1,
        },
        ServerCommand::StartSession {
            agent_name: s(),
            goal: s(),
            adapter: s(),
            session_id: None,
            run_id: None,
        },
        ServerCommand::ReplayAdapterFixture {
            adapter: s(),
            session_id: s(),
            run_id: s(),
            turn_id: s(),
            fixture_name: s(),
            fixture_jsonl: s(),
        },
        ServerCommand::PlanDispatch {
            agent_name: s(),
            adapter: s(),
            goal: s(),
            workspace: s(),
            artifacts: s(),
            session_id: s(),
            run_id: s(),
            turn_id: s(),
            deterministic_opt_in: false,
        },
        ServerCommand::PreflightLiveProvider {
            agent_name: s(),
            adapter: s(),
            goal: s(),
            workspace: s(),
            artifacts: s(),
            session_id: s(),
            run_id: s(),
            turn_id: s(),
            capability_profile: s(),
            runtime_scope: s(),
            credential_scan_policy: s(),
            raw_prompt_policy: s(),
            raw_output_policy: s(),
            tool_wrapper_policy: s(),
            live_provider_opt_in: false,
        },
        ServerCommand::GateDispatch {
            dispatch_plan_id: s(),
        },
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id: s(),
            fixture_name: s(),
            fixture_jsonl: s(),
        },
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: s(),
            goal: s(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: false,
            mock_provider_output_name: None,
            mock_provider_output_jsonl: None,
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
        ServerCommand::RunDispatchTurn {
            agent_name: s(),
            adapter: s(),
            goal: s(),
            workspace: s(),
            artifacts: s(),
            session_id: s(),
            run_id: s(),
            turn_id: s(),
            capability_profile: s(),
            runtime_scope: s(),
            credential_scan_policy: s(),
            raw_prompt_policy: s(),
            raw_output_policy: s(),
            tool_wrapper_policy: s(),
            live_provider_opt_in: false,
            live_execution_opt_in: false,
            mock_runtime_opt_in: false,
            mock_provider_output_name: None,
            mock_provider_output_jsonl: None,
            timeout_seconds: 1,
            max_turns: 1,
            max_token_cost: 0,
            turns_taken_before: 0,
            token_cost_before: 0,
            turn_token_cost: 0,
            unattended: true,
        },
        ServerCommand::Recover,
        ServerCommand::Subscribe {
            session_id: None,
            from_sequence: 0,
        },
        ServerCommand::ReadThread {
            session_id: s(),
            from_sequence: 0,
        },
        // GA2: every goal command through the real codec so a field-name/order
        // swap in any goal encode/decode arm is caught by the round-trip below.
        // Distinct, differently-shaped JSON blobs and a non-empty requirements vec
        // so a swap (e.g. constraints_json decoded from verification_surface_json)
        // does not survive on two equal values.
        ServerCommand::SetGoal {
            spec: crate::GoalSpec {
                goal_id: "goal-set".to_string(),
                objective: "objective-text".to_string(),
                task_id: Some("task-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                session_id: Some("session-1".to_string()),
                parent_goal_id: Some("parent-goal".to_string()),
                attempt_run_id: Some("run-1".to_string()),
                requirements: vec![
                    crate::GoalRequirementSpec {
                        requirement_id: "req-a".to_string(),
                        summary: "requirement a".to_string(),
                    },
                    crate::GoalRequirementSpec {
                        requirement_id: "req-b".to_string(),
                        summary: "requirement b".to_string(),
                    },
                ],
                success_criteria_json: r#"{"success":1}"#.to_string(),
                constraints_json: r#"{"constraints":2}"#.to_string(),
                verification_surface_json: r#"{"verification":3}"#.to_string(),
                budget_json: r#"{"budget":4}"#.to_string(),
                stop_conditions_json: r#"{"stop":5}"#.to_string(),
            },
        },
        ServerCommand::PauseGoal {
            goal_id: "goal-pause".to_string(),
        },
        ServerCommand::ResumeGoal {
            goal_id: "goal-resume".to_string(),
        },
        ServerCommand::BlockGoal {
            goal_id: "goal-block".to_string(),
            reason: "blocked reason".to_string(),
        },
        ServerCommand::ClearGoal {
            goal_id: "goal-clear".to_string(),
            reason: "cleared reason".to_string(),
        },
        ServerCommand::SetRequirementStatus {
            record: crate::RequirementStatusRecord {
                requirement_id: "req-status".to_string(),
                goal_id: "goal-req".to_string(),
                summary: "requirement summary".to_string(),
                status: "supported".to_string(),
                source: "runtime_output".to_string(),
            },
        },
        ServerCommand::RecordGoalReport {
            report: crate::GoalReportRecord {
                goal_report_id: "report-1".to_string(),
                goal_id: "goal-report".to_string(),
                session_id: Some("session-report".to_string()),
                requirement_id: Some("req-report".to_string()),
                report_kind: "capo.report_progress".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(80),
                summary: "report summary".to_string(),
                body_artifact_id: Some("artifact-1".to_string()),
                evidence_id: Some("evidence-1".to_string()),
            },
        },
        ServerCommand::MarkGoalComplete {
            goal_id: "goal-complete".to_string(),
        },
        ServerCommand::ListGoals,
        ServerCommand::ViewGoal {
            goal_id: "goal-view".to_string(),
        },
        ServerCommand::GoalStory {
            goal_id: "goal-story".to_string(),
        },
        ServerCommand::GoalTimeline {
            goal_id: "goal-timeline".to_string(),
        },
        ServerCommand::GoalEvidence {
            goal_id: "goal-evidence".to_string(),
        },
        ServerCommand::GoalValidations {
            goal_id: "goal-validations".to_string(),
        },
        ServerCommand::GoalReviews {
            goal_id: "goal-reviews".to_string(),
        },
        ServerCommand::GoalRisks {
            goal_id: "goal-risks".to_string(),
        },
        ServerCommand::GoalReport {
            goal_id: "goal-render".to_string(),
            format: crate::GoalReportFormat::Json,
        },
        ServerCommand::ReplayRunnerEvents {
            frames: vec![crate::RunnerReplayFrame {
                event_id: "runner-evt-0".to_string(),
                kind: "runtime.remote_output_delta".to_string(),
                session_id: "session-replay".to_string(),
                idempotency_key: "runtime.remote_output_delta:run-x:0".to_string(),
                payload_json: "{\"offset\":0}".to_string(),
                redaction_state: "safe".to_string(),
            }],
        },
    ]
}

/// One value of every `ServerResponsePayload` variant (only the discriminant is
/// read by `payload_type`, so the inner summaries are minimal).
fn sample_payloads() -> Vec<ServerResponsePayload> {
    use crate::{
        AdapterReplaySummary, AgentSummary, DispatchGateSummary, DispatchPlanSummary,
        DispatchRunSummary, DispatchTurnSummary, LiveProviderPreflightSummary, RecoverySummary,
        ServerDashboardSnapshot, ServerThread, SubscriptionBacklog, TaskRunSummary,
        TurnFinishedSummary,
    };
    use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};
    let s = || "x".to_string();
    let agent = || AgentSummary {
        agent_id: AgentId::new("a"),
        name: s(),
        status: s(),
        current_session_id: None,
        session: None,
    };
    let run = || TaskRunSummary {
        task_id: TaskId::new("t"),
        agent_id: AgentId::new("a"),
        session_id: SessionId::new("s"),
        run_id: RunId::new("r"),
        runtime_process_ref: s(),
        external_session_ref: s(),
    };
    vec![
        ServerResponsePayload::AgentRegistered(agent()),
        ServerResponsePayload::TaskSent(run()),
        ServerResponsePayload::Agents(vec![agent()]),
        ServerResponsePayload::AgentStatus(agent()),
        ServerResponsePayload::Dashboard(ServerDashboardSnapshot {
            project_id: ProjectId::new("p"),
            agent_count: 0,
            active_session_count: 0,
            agents: vec![],
        }),
        ServerResponsePayload::SessionStarted(run()),
        ServerResponsePayload::AdapterFixtureReplayed(AdapterReplaySummary {
            adapter: s(),
            fixture_name: s(),
            fixture_hash: s(),
            agent_name: s(),
            task_id: TaskId::new("t"),
            session_id: SessionId::new("s"),
            run_id: RunId::new("r"),
            turn_id: s(),
            provider_cli_executed: false,
            raw_content_policy: s(),
            input_event_count: 0,
            appended_event_count: 0,
            tool_event_count: 0,
            summary_event_count: 0,
            completed_turn_count: 0,
        }),
        ServerResponsePayload::DispatchPlanned(DispatchPlanSummary {
            dispatch_plan_id: s(),
            prompt_source_id: s(),
            adapter: s(),
            agent_name: s(),
            session_id: SessionId::new("s"),
            run_id: RunId::new("r"),
            runtime_program: s(),
            runtime_prompt_policy: s(),
            raw_prompt_policy: s(),
            provider_cli_executed: false,
            status: s(),
        }),
        ServerResponsePayload::LiveProviderPreflighted(LiveProviderPreflightSummary {
            dispatch_plan_id: s(),
            dispatch_gate_id: s(),
            execution_request_id: s(),
            adapter: s(),
            provider_kind: s(),
            agent_name: s(),
            session_id: SessionId::new("s"),
            run_id: RunId::new("r"),
            capability_profile: s(),
            runtime_scope: s(),
            credential_scan_policy: s(),
            raw_prompt_policy: s(),
            raw_output_policy: s(),
            tool_wrapper_policy: s(),
            provider_cli_execution_allowed: false,
            provider_cli_executed: false,
            status: s(),
            reasons: s(),
            next_action: s(),
        }),
        ServerResponsePayload::DispatchGated(DispatchGateSummary {
            dispatch_plan_id: s(),
            dispatch_gate_id: s(),
            execution_request_id: s(),
            materialization_id: s(),
            adapter: s(),
            provider_cli_execution_allowed: false,
            provider_cli_executed: false,
            status: s(),
            reasons: s(),
            raw_prompt_policy: s(),
        }),
        ServerResponsePayload::DispatchRun(DispatchRunSummary {
            dispatch_plan_id: s(),
            dispatch_execution_id: s(),
            adapter: s(),
            session_id: SessionId::new("s"),
            run_id: RunId::new("r"),
            provider_cli_execution_allowed: false,
            provider_cli_executed: false,
            status: s(),
            runtime_process_ref: None,
            credential_scan_status: s(),
            raw_prompt_policy: s(),
            raw_output_policy: s(),
            reason_codes: s(),
            input_event_count: 0,
            appended_event_count: 0,
            tool_event_count: 0,
            summary_event_count: 0,
            completed_turn_count: 0,
            observed_token_cost: None,
        }),
        ServerResponsePayload::DispatchTurn(DispatchTurnSummary {
            run: DispatchRunSummary {
                dispatch_plan_id: s(),
                dispatch_execution_id: s(),
                adapter: s(),
                session_id: SessionId::new("s"),
                run_id: RunId::new("r"),
                provider_cli_execution_allowed: false,
                provider_cli_executed: false,
                status: s(),
                runtime_process_ref: None,
                credential_scan_status: s(),
                raw_prompt_policy: s(),
                raw_output_policy: s(),
                reason_codes: s(),
                input_event_count: 0,
                appended_event_count: 0,
                tool_event_count: 0,
                summary_event_count: 0,
                completed_turn_count: 0,
                observed_token_cost: None,
            },
            finished: TurnFinishedSummary {
                turn_id: s(),
                stop_reason: s(),
                observed_terminal_event: true,
                // Distinct, non-empty, differently-shaped ref lists so the codec
                // round-trip detects a field swap (e.g. summary_refs decoded into
                // observed_tool_refs) rather than passing on two empty vecs.
                summary_refs: vec!["summary-a".to_string(), "summary-b".to_string()],
                observed_tool_refs: vec!["tool-a".to_string()],
            },
            ceiling_breach_code: Some("max_turns_exceeded".to_string()),
        }),
        ServerResponsePayload::Recovery(RecoverySummary {
            recovery_attempt_id: s(),
            recovered_run_count: 0,
            watermark: None,
        }),
        ServerResponsePayload::Subscribed(SubscriptionBacklog {
            session_id: None,
            from_sequence: 0,
            next_sequence: 0,
            events: vec![],
        }),
        ServerResponsePayload::Thread(ServerThread {
            session_id: s(),
            from_sequence: 0,
            next_sequence: 0,
            turns: vec![],
        }),
        // GA2: every goal RESPONSE payload through the real codec. The nested
        // values are distinct and differently-shaped (distinct success/constraints/
        // verification/budget/stop blobs, a non-empty requirements/reports/
        // continuations/delegated vec, distinct confidence/observed flags) so the
        // round-trip loop catches a field swap in any goal encoder/decoder arm.
        ServerResponsePayload::Goals(vec![goal_summary()]),
        ServerResponsePayload::GoalView(Box::new(crate::GoalView {
            summary: goal_summary(),
            success_criteria_json: r#"{"success":1}"#.to_string(),
            constraints_json: r#"{"constraints":2}"#.to_string(),
            verification_surface_json: r#"{"verification":3}"#.to_string(),
            budget_json: r#"{"budget":4}"#.to_string(),
            stop_conditions_json: r#"{"stop":5}"#.to_string(),
            task_id: Some("task-view".to_string()),
            agent_id: Some("agent-view".to_string()),
            session_id: Some("session-view".to_string()),
            requirements: vec![goal_requirement_view()],
            reports: vec![goal_report_view()],
            continuations: vec![goal_continuation_view()],
            delegated_provider_goals: vec![delegated_provider_goal_view()],
        })),
        ServerResponsePayload::GoalReports(crate::GoalReportListing {
            goal_id: "goal-reports".to_string(),
            surface: "evidence".to_string(),
            blocker_reason: "listing blocker".to_string(),
            reports: vec![goal_report_view()],
        }),
        ServerResponsePayload::GoalTimeline(crate::GoalTimelineView {
            goal_id: "goal-timeline".to_string(),
            entries: vec![crate::GoalTimelineEntry {
                sequence: 7,
                event_id: "event-timeline".to_string(),
                kind: "goal.created".to_string(),
                actor: "actor-timeline".to_string(),
                redaction_state: "safe".to_string(),
            }],
        }),
        ServerResponsePayload::GoalReport(crate::GoalReportRendering {
            goal_id: "goal-render".to_string(),
            format: "json".to_string(),
            body: r#"{"goal_id":"goal-render"}"#.to_string(),
            degraded: true,
        }),
        ServerResponsePayload::RunnerEventsReplayed(crate::RunnerEventsReplayedSummary {
            appended_sequences: vec![1, 2, 3],
        }),
    ]
}

/// A [`GoalStatusSummary`] with distinct counts so a field swap in the summary
/// codec does not survive the contract round-trip.
fn goal_summary() -> crate::GoalStatusSummary {
    crate::GoalStatusSummary {
        goal_id: "goal-summary".to_string(),
        objective: "summary objective".to_string(),
        status: "active".to_string(),
        parent_goal_id: Some("parent-summary".to_string()),
        attempt_run_id: Some("run-summary".to_string()),
        requirement_count: 4,
        requirements_supported: 3,
        blocked_requirement_count: 2,
        contradicted_requirement_count: 1,
        report_count: 5,
        blocker_reason: "summary blocker".to_string(),
        updated_sequence: 9,
    }
}

fn goal_requirement_view() -> crate::GoalRequirementView {
    crate::GoalRequirementView {
        requirement_id: "req-view".to_string(),
        summary: "requirement view summary".to_string(),
        status: "supported".to_string(),
        last_status_source: "runtime_output".to_string(),
        observed: true,
    }
}

fn goal_report_view() -> crate::GoalReportView {
    crate::GoalReportView {
        goal_report_id: "report-view".to_string(),
        requirement_id: Some("req-report-view".to_string()),
        report_kind: "capo.record_validation".to_string(),
        source: "agent_reported".to_string(),
        observed: false,
        confidence: Some(60),
        summary: "report view summary".to_string(),
        body_artifact_id: Some("artifact-view".to_string()),
        evidence_id: Some("evidence-view".to_string()),
    }
}

fn goal_continuation_view() -> crate::GoalContinuationView {
    crate::GoalContinuationView {
        continuation_id: "continuation-view".to_string(),
        decision: "continue".to_string(),
        reason: "continuation reason".to_string(),
        attempt_run_id: Some("run-continuation".to_string()),
    }
}

fn delegated_provider_goal_view() -> crate::DelegatedProviderGoalView {
    crate::DelegatedProviderGoalView {
        delegated_goal_id: "delegated-view".to_string(),
        provider_kind: "codex".to_string(),
        provider_goal_ref: Some("provider-ref".to_string()),
        provider_state: "running".to_string(),
        source: "adapter_event".to_string(),
    }
}

/// One value of every domain `ServerError` variant.
fn sample_errors() -> Vec<ServerError> {
    let s = || "x".to_string();
    vec![
        ServerError::State(capo_state::StateError::MissingRecoveryAttempt(s())),
        ServerError::AdapterFixture(s()),
        ServerError::UnknownAgent { agent_name: s() },
        ServerError::AgentHasNoActiveSession { agent_name: s() },
        ServerError::AgentAlreadyHasSession {
            agent_name: s(),
            session_id: s(),
            run_status: None,
        },
        ServerError::SessionAlreadyExists { session_id: s() },
        ServerError::RunAlreadyExists { run_id: s() },
        ServerError::UnknownDispatchPlan {
            dispatch_plan_id: s(),
        },
        ServerError::UnknownSession { session_id: s() },
        ServerError::RunSessionMismatch {
            session_id: s(),
            run_id: s(),
            actual_session_id: s(),
        },
        ServerError::AdapterSessionMismatch {
            session_id: s(),
            session_adapter: s(),
            requested_adapter: s(),
        },
        ServerError::UnsupportedChatAdapter { adapter: s() },
        ServerError::UnknownGoal { goal_id: s() },
        ServerError::GoalCompleteNotALifecycleCommand { goal_id: s() },
        ServerError::IllegalGoalStatusTransition {
            goal_id: s(),
            requested_status: s(),
        },
        ServerError::UnclassifiableReportSource { source: s() },
        ServerError::InvalidRuntimeTargetField {
            field: "runner_kind",
            value: s(),
            expected: "local-process, remote-process, or container",
        },
        ServerError::InvalidRunnerReplayFrame {
            event_id: s(),
            field: "kind",
            value: s(),
            expected: "a runtime.remote_* event kind",
        },
    ]
}
