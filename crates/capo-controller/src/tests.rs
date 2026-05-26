use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

static TEMP_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn controller_plans_local_adapter_dispatch_without_runtime_execution() {
    let state_root = temp_root();
    let workspace = temp_root();
    let artifacts = temp_root();
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &state_root)
        .expect("open controller");
    controller
        .register_agent("codex-worker")
        .expect("register agent");

    let plan = controller
        .plan_local_adapter_dispatch(
            "codex",
            "codex-worker",
            "Summarize the current workpad.",
            workspace.clone(),
            artifacts.clone(),
        )
        .expect("plan dispatch");

    assert_eq!(plan.agent_name, "codex-worker");
    assert_eq!(plan.runtime_program, "codex");
    assert_eq!(plan.runtime_cwd, workspace);
    assert_eq!(plan.request_env_count, 0);
    assert_eq!(plan.launch_plan.provider_kind, "codex_subscription");
    assert_eq!(plan.launch_plan.credential_scope, "user_local_subscription");
    assert_eq!(plan.launch_plan.artifact_root, artifacts);
    assert!(plan.launch_plan.assert_subscription_safe().is_ok());
    assert!(
        plan.launch_plan
            .argv
            .windows(2)
            .any(|args| args == ["--sandbox", "read-only"])
    );
    assert_eq!(controller.state().run(&plan.run_id).unwrap(), None);
}

#[test]
fn controller_rejects_unknown_local_adapter_dispatch_plan() {
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), temp_root())
        .expect("open controller");
    controller.register_agent("worker").expect("register agent");

    let error = controller
        .plan_local_adapter_dispatch("unknown", "worker", "Do work.", temp_root(), temp_root())
        .unwrap_err();

    assert!(error.contains("unsupported local adapter dispatch plan"));
}

#[test]
fn fake_boundaries_drive_controller_state_and_interrupt_from_read_models() {
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), temp_root())
        .expect("open controller");
    let registration = controller.register_agent("fake-codex").expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Inspect the project and write a status summary",
        )
        .expect("send task");

    let observation = controller.observe(&refs).expect("observe");
    assert_eq!(observation.task.capo_execution_status, "active");
    assert_eq!(observation.agent.status, "running");
    assert_eq!(observation.session.status, "active");
    assert_eq!(observation.session.latest_confidence, Some(82));
    assert_eq!(observation.run.status, "running");
    assert!(
        observation
            .recent_events
            .iter()
            .any(|event| event.kind == "tool.call_completed")
    );
    for expected_kind in [
        "permission.decided",
        "capability.grant_created",
        "capability.grant_used",
        "tool.call_requested",
        "tool.invocation_started",
        "tool.output_artifact_recorded",
        "tool.output_observed",
        "tool.call_completed",
        "tool.result_delivered",
        "memory.packet_built",
        "evidence.recorded",
    ] {
        assert!(
            observation
                .recent_events
                .iter()
                .any(|event| event.kind == expected_kind),
            "{expected_kind}"
        );
    }
    let packets = controller
        .state()
        .memory_packets_for_session(&refs.session_id)
        .expect("memory packet projection");
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].turn_id.as_deref(), Some("turn-fake-codex"));
    assert_eq!(packets[0].run_id.as_ref(), Some(&refs.run_id));
    assert_eq!(
        packets[0].packet_artifact_id.as_deref(),
        Some("artifact-memory-packet-packet-fake-codex")
    );
    let memory_event = observation
        .recent_events
        .iter()
        .find(|event| event.kind == "memory.packet_built")
        .expect("memory packet event");
    assert!(memory_event.payload_json.contains("\"included_count\":3"));
    assert!(memory_event.payload_json.contains("\"excluded_count\":1"));
    assert!(
        memory_event
            .payload_json
            .contains("explanation_artifact_id")
    );

    let interrupted = controller
        .interrupt(&registration, &refs, "P3 smoke interrupt")
        .expect("interrupt");
    assert_eq!(interrupted.task.capo_execution_status, "canceled");
    assert_eq!(interrupted.agent.status, "available");
    assert_eq!(interrupted.session.status, "canceled");
    assert_eq!(interrupted.run.status, "stopping");
    assert!(
        interrupted
            .recent_events
            .iter()
            .any(|event| event.kind == "session.interrupted")
    );

    let reopened = SqliteStateStore::open(controller.state().db_path().parent().unwrap())
        .expect("reopen state");
    assert_eq!(
        reopened
            .session(&refs.session_id)
            .expect("read session")
            .expect("session")
            .status,
        "canceled"
    );
}

#[test]
fn denied_static_permission_stops_tool_invocation_in_controller_path() {
    let controller = FakeBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        temp_root(),
        PermissionPolicy::static_read_only_local(),
    )
    .expect("open controller");
    let registration = controller.register_agent("fake-codex").expect("agent");
    let refs = controller
        .send_task(&registration, "Inspect the project with static policy")
        .expect("send task");
    let observation = controller.observe(&refs).expect("observe");

    assert_eq!(observation.task.capo_execution_status, "blocked");
    assert_eq!(observation.agent.status, "paused");
    assert_eq!(observation.session.status, "waiting_for_permission");
    assert!(
        observation
            .session
            .latest_blocker
            .as_deref()
            .expect("blocker")
            .contains("memory:build_packet:session")
    );
    assert!(observation.recent_events.iter().any(|event| {
        event.kind == "permission.decided"
            && event.payload_json.contains("\"effect\":\"deny\"")
            && event.event_id.contains("grant-")
    }));
    assert!(
        !observation
            .recent_events
            .iter()
            .any(|event| event.kind == "capability.grant_used")
    );
    assert!(
        !observation
            .recent_events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );
    assert!(
        !observation
            .recent_events
            .iter()
            .any(|event| event.kind == "tool.call_completed")
    );

    let grants = controller.state().capability_grants().expect("grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].effect, "deny");
    assert_eq!(grants[0].decision_source, "static_policy:read-only-local");
    assert!(grants[0].capability_grant_id.contains("-deny-"));

    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].status, "denied");
    assert!(tools[0].output_artifact_id.is_none());
    assert!(
        controller
            .state()
            .memory_packets_for_session(&refs.session_id)
            .expect("memory packets")
            .is_empty()
    );
    assert!(
        controller
            .state()
            .evidence_for_session(&refs.session_id)
            .expect("evidence")
            .is_empty()
    );
}

#[test]
fn scripted_mock_agent_drives_multi_turn_controller_state() {
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), temp_root())
        .expect("open controller");
    let registration = controller.register_agent("mock-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run a deterministic scripted mock agent")
        .expect("send task");

    let first_turn = capo_adapters::ScriptedMockTurn::new("turn-1")
        .message_delta("msg-1", "inspecting state")
        .message_delta("msg-1", "still inspecting state")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .message_completed("msg-2", "state inspected")
        .turn_completed("done-1");
    let first_report = controller
        .apply_scripted_mock_turn(&refs, &first_turn)
        .expect("apply first mock turn");

    assert_eq!(first_report.input_event_count, 6);
    assert_eq!(first_report.summary_event_count, 3);
    assert_eq!(first_report.tool_event_count, 2);
    assert_eq!(first_report.completed_turn_count, 1);

    let redirected = controller
        .redirect(&registration, &refs, "Now report the blocker state")
        .expect("redirect");
    assert_eq!(
        redirected.session.current_goal,
        "Now report the blocker state"
    );

    let second_turn = capo_adapters::ScriptedMockTurn::new("turn-2")
        .message_completed("msg-3", "no blockers found")
        .turn_completed("done-2");
    let second_report = controller
        .apply_scripted_mock_turn(&refs, &second_turn)
        .expect("apply second mock turn");

    assert_eq!(second_report.input_event_count, 2);
    assert_eq!(second_report.summary_event_count, 1);
    assert_eq!(second_report.completed_turn_count, 1);

    let control_turn = capo_adapters::ScriptedMockTurn::new("turn-3")
        .permission_requested("permission-1", "[\"tool:invoke:capo.agent_status\"]")
        .failed("failure-1", "scripted failure branch")
        .interrupted("interrupt-1", "scripted interrupt branch");
    let control_report = controller
        .apply_scripted_mock_turn(&refs, &control_turn)
        .expect("apply control mock turn");

    assert_eq!(control_report.input_event_count, 3);
    assert_eq!(control_report.appended_event_count, 3);

    let observation = controller.observe(&refs).expect("observe");
    assert_eq!(observation.session.status, "canceled");
    assert!(
        observation
            .session
            .latest_blocker
            .as_deref()
            .unwrap_or_default()
            .contains("content_hash=")
    );
    assert!(
        observation
            .session
            .latest_summary
            .as_deref()
            .unwrap_or_default()
            .contains("content_hash=")
    );
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_name == "capo.agent_status"
            && tool.tool_origin == "adapter_native:mock"
            && tool.status == "completed"
    }));
    let observations = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    assert!(observations.iter().any(|observation| {
        observation.tool_name == "capo.agent_status"
            && observation.source == "adapter_event:mock"
            && observation.observed_status == "completed"
            && observation.instrumentation_level == "observed_only"
            && observation.tool_call_id.is_some()
    }));
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 32)
        .expect("recent events");
    assert!(events.iter().any(|event| {
        event.kind == "permission.requested"
            && event
                .payload_json
                .contains("\"normalized_kind\":\"adapter.permission_requested\"")
    }));
    assert!(events.iter().any(|event| {
        event.kind == "run.exited"
            && event
                .payload_json
                .contains("\"normalized_kind\":\"adapter.turn_failed\"")
    }));
    assert!(events.iter().any(|event| {
        event.kind == "session.interrupted"
            && event
                .payload_json
                .contains("\"normalized_kind\":\"adapter.turn_interrupted\"")
    }));
    let evidence = controller
        .state()
        .evidence_for_session(&refs.session_id)
        .expect("evidence");
    assert!(
        evidence
            .iter()
            .any(|item| item.kind == "adapter_replay:mock")
    );

    let interrupted = controller
        .interrupt(&registration, &refs, "scripted mock done")
        .expect("interrupt");
    assert_eq!(interrupted.task.capo_execution_status, "canceled");
    assert_eq!(interrupted.agent.status, "available");
    assert_eq!(interrupted.run.status, "stopping");
}

#[test]
fn acp_fixture_replay_dedupes_stable_tool_updates_in_state() {
    let store = SqliteStateStore::open(temp_root()).expect("open state");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-acp");
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capo-adapters/fixtures/acp-replay.jsonl"
    ))
    .expect("read ACP fixture");
    let parsed = capo_adapters::AcpAdapter::parse_replay_jsonl(&fixture).expect("parse ACP");
    let tool_events = parsed
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.timeline_key.as_deref() == Some("acp:acp-session-1:tool:tool-1"))
        .collect::<Vec<_>>();

    assert_eq!(tool_events.len(), 4);
    for pass in 0..2 {
        for (index, adapter_event) in &tool_events {
            let Some(idempotency_key) = adapter_event.idempotency_key.clone() else {
                continue;
            };
            let (kind, status) = match adapter_event.kind.as_str() {
                "adapter.tool_call_requested" => (EventKind::ToolCallRequested, "requested"),
                "adapter.tool_call_started" => (EventKind::ToolInvocationStarted, "started"),
                "adapter.tool_call_completed" => (EventKind::ToolCallCompleted, "completed"),
                other => panic!("unexpected ACP tool event kind: {other}"),
            };
            let tool_name = adapter_event
                .tool_name
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            store
                    .append_event(
                        NewEvent {
                            event_id: format!("event-acp-replay-{pass}-{index}"),
                            kind,
                            actor: "acp-replay".to_string(),
                            project_id: Some(project_id.clone()),
                            task_id: None,
                            agent_id: None,
                            session_id: Some(session_id.clone()),
                            run_id: None,
                            turn_id: Some("turn-acp".to_string()),
                            item_id: adapter_event.external_item_ref.clone(),
                            payload_json: format!(
                                "{{\"adapter_kind\":\"acp\",\"provider_event_kind\":\"{}\",\"status\":\"{}\"}}",
                                escape_json(&adapter_event.provider_event_kind),
                                status
                            ),
                            idempotency_key: Some(idempotency_key),
                            redaction_state: RedactionState::Safe,
                        },
                        &[ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                            tool_call_id: ToolCallId::new("tool-acp-tool-1"),
                            session_id: session_id.clone(),
                            turn_id: Some("turn-acp".to_string()),
                            tool_name,
                            tool_origin: "adapter_native".to_string(),
                            status: status.to_string(),
                            input_artifact_id: None,
                            output_artifact_id: adapter_event
                                .content
                                .as_ref()
                                .map(|_| "artifact-acp-tool-1-output".to_string()),
                            updated_sequence: 0,
                        })],
                    )
                    .expect("append normalized ACP event");
        }
    }

    assert_eq!(store.event_count().unwrap(), 3);
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.event_count().unwrap(), 3);
    let tool_calls = store
        .tool_calls_for_session(&session_id)
        .expect("tool call read model");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].status, "completed");
    assert_eq!(tool_calls[0].tool_origin, "adapter_native");
}

#[test]
fn codex_fixture_replay_updates_controller_read_models_without_raw_content_payloads() {
    let root = temp_root();
    let controller =
        FakeBoundaryController::open(ProjectId::new("project-capo"), &root).expect("controller");
    let registration = controller
        .register_agent("real-codex-replay")
        .expect("register replay agent");
    let refs = controller
        .send_task(&registration, "Replay a normalized Codex fixture")
        .expect("send task");
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capo-adapters/fixtures/codex-exec.jsonl"
    ))
    .expect("read Codex fixture");
    let parsed = capo_adapters::CodexExecAdapter::parse_jsonl(&fixture).expect("parse Codex");
    let report = controller
        .apply_normalized_adapter_events(&refs, &parsed.deduped_by_idempotency())
        .expect("apply adapter replay");

    assert_eq!(report.input_event_count, 5);
    assert_eq!(report.summary_event_count, 1);
    assert_eq!(report.tool_event_count, 2);
    assert_eq!(report.completed_turn_count, 1);
    let observation = controller.observe(&refs).expect("observe replay");
    assert!(
        observation
            .session
            .latest_summary
            .as_deref()
            .unwrap_or_default()
            .contains("content_hash=")
    );
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_name == "exec_command"
            && tool.tool_origin == "adapter_native:codex_exec"
            && tool.status == "completed"
    }));
    let observations = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    assert!(observations.iter().any(|observation| {
        observation.tool_name == "exec_command"
            && observation.source == "adapter_event:codex_exec"
            && observation.observed_status == "completed"
            && observation.instrumentation_level == "observed_only"
            && observation.confidence == "high"
            && observation.tool_call_id.is_some()
    }));
    let evidence = controller
        .state()
        .evidence_for_session(&refs.session_id)
        .expect("evidence");
    assert!(
        evidence
            .iter()
            .any(|item| item.kind == "adapter_replay:codex_exec")
    );
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 16)
        .expect("recent events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "session.summary_updated")
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded")
    );
    for event in events {
        assert!(!event.payload_json.contains("Codex fixture response."));
        assert!(!event.payload_json.contains("cargo test"));
        assert!(event.redaction_state != "contains_sensitive");
    }
}

#[test]
fn claude_fixture_replay_updates_controller_read_models_without_raw_content_payloads() {
    let root = temp_root();
    let controller =
        FakeBoundaryController::open(ProjectId::new("project-capo"), &root).expect("controller");
    let registration = controller
        .register_agent("real-claude-replay")
        .expect("register replay agent");
    let refs = controller
        .send_task(&registration, "Replay a normalized Claude fixture")
        .expect("send task");
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capo-adapters/fixtures/claude-code-stream.jsonl"
    ))
    .expect("read Claude fixture");
    let parsed =
        capo_adapters::ClaudeCodeAdapter::parse_stream_json(&fixture).expect("parse Claude");
    let report = controller
        .apply_normalized_adapter_events(&refs, &parsed.deduped_by_idempotency())
        .expect("apply adapter replay");

    assert_eq!(report.input_event_count, 5);
    assert_eq!(report.summary_event_count, 1);
    assert_eq!(report.tool_event_count, 2);
    assert_eq!(report.completed_turn_count, 1);
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_name == "Bash"
            && tool.tool_origin == "adapter_native:claude_code"
            && tool.status == "completed"
    }));
    let observations = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    assert!(observations.iter().any(|observation| {
        observation.tool_name == "Bash"
            && observation.source == "adapter_event:claude_code"
            && observation.observed_status == "completed"
            && observation.instrumentation_level == "observed_only"
            && observation.tool_call_id.is_some()
    }));
    let evidence = controller
        .state()
        .evidence_for_session(&refs.session_id)
        .expect("evidence");
    assert!(
        evidence
            .iter()
            .any(|item| item.kind == "adapter_replay:claude_code")
    );
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 16)
        .expect("recent events");
    for event in events {
        assert!(!event.payload_json.contains("Claude fixture response."));
        assert!(!event.payload_json.contains("cargo test"));
        assert!(!event.payload_json.contains("tests passed"));
        assert!(event.redaction_state != "contains_sensitive");
    }
}

fn temp_root() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("capo-controller-{nanos}-{counter}"))
}
