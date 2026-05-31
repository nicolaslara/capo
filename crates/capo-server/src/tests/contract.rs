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
    ]
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
    ]
}
