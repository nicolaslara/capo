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

#[test]
fn controller_drives_injected_scripted_mock_adapter_behind_the_trait() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // The controller holds the adapter behind the `AgentAdapter` trait via the
    // thin dispatch handle, so the scripted-mock implementation is substituted
    // for the fake default without naming a concrete `Fake*` request/output
    // type at the call site.
    let scripted = AgentAdapterHandle::scripted_mock(
        ScriptedMockAgent::new("scripted-injected-session").with_turn(
            ScriptedMockTurn::new("turn-mock-worker")
                .message_completed("msg-1", "scripted injected summary"),
        ),
    );
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open controller with injected adapter");
    let registration = controller.register_agent("mock-worker").expect("agent");

    let refs = controller
        .send_task(&registration, "Run a deterministic scripted mock turn")
        .expect("send task");

    // The send-turn output flows through the provider-neutral `TurnOutput`:
    // external session ref and summary come from the injected scripted mock,
    // confidence is the scripted-mock deterministic value (88, not the fake 82).
    assert_eq!(refs.external_session_ref, "scripted-injected-session");
    let observation = controller.observe(&refs).expect("observe");
    assert_eq!(
        observation.session.latest_summary.as_deref(),
        Some("scripted injected summary")
    );
    assert_eq!(observation.session.latest_confidence, Some(88));
    // The scripted mock derives status from its last event; a completed message
    // yields a "completed" turn status, distinct from the fake adapter's
    // "active". This proves the controller observes the injected adapter's
    // deterministic output rather than the fake default.
    assert_eq!(observation.session.status, "completed");

    // Interrupt and stop route through the injected adapter's trait methods,
    // producing the scripted-mock deterministic summaries.
    let interrupted = controller
        .interrupt(&registration, &refs, "operator paused")
        .expect("interrupt");
    assert_eq!(interrupted.session.status, "canceled");
    assert!(
        controller
            .state()
            .recent_events_for_session(&refs.session_id, 32)
            .expect("events")
            .iter()
            .any(|event| {
                event.kind == "session.interrupted"
                    && event
                        .payload_json
                        .contains("Scripted mock interrupted session: operator paused")
            })
    );
}

#[test]
fn command_path_re_derives_injected_adapter_external_session_ref() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // The command-driven loop (interrupt/stop_agent_name -> refs_for_agent_name)
    // must stay adapter-neutral: it re-derives refs from the persisted read
    // model, so the external session ref handed to the injected adapter is the
    // value that adapter reported at session.started, not a fake-adapter naming
    // template (`fake-adapter-session-{agent_name}`).
    let scripted = AgentAdapterHandle::scripted_mock(
        ScriptedMockAgent::new("scripted-injected-session").with_turn(
            ScriptedMockTurn::new("turn-mock-worker")
                .message_completed("msg-1", "scripted injected summary"),
        ),
    );
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open controller with injected adapter");
    let registration = controller.register_agent("mock-worker").expect("agent");
    let direct_refs = controller
        .send_task(&registration, "Run a deterministic scripted mock turn")
        .expect("send task");

    // The command path re-derives refs purely from persisted state. It resolves
    // the injected adapter's ref, never the fake template the old code baked in.
    let rederived = controller
        .refs_for_agent_name("mock-worker")
        .expect("re-derive refs from read model");
    assert_eq!(rederived.external_session_ref, "scripted-injected-session");
    assert_ne!(
        rederived.external_session_ref,
        "fake-adapter-session-mock-worker"
    );
    assert_eq!(
        rederived.external_session_ref,
        direct_refs.external_session_ref
    );

    // Driving interrupt_agent_name (the command entry point) attaches to the
    // injected ref and routes through the scripted mock's trait methods.
    let interrupted = controller
        .interrupt_agent_name("mock-worker", "operator paused")
        .expect("interrupt via command path");
    assert_eq!(interrupted.session.status, "canceled");
    assert_eq!(
        interrupted.session.external_session_ref.as_deref(),
        Some("scripted-injected-session")
    );
    assert!(
        controller
            .state()
            .recent_events_for_session(&direct_refs.session_id, 32)
            .expect("events")
            .iter()
            .any(|event| {
                event.kind == "session.interrupted"
                    && event
                        .payload_json
                        .contains("Scripted mock interrupted session: operator paused")
            })
    );
}

#[test]
fn command_path_stop_re_derives_injected_adapter_external_session_ref() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    let scripted = AgentAdapterHandle::scripted_mock(
        ScriptedMockAgent::new("scripted-injected-session").with_turn(
            ScriptedMockTurn::new("turn-mock-worker")
                .message_completed("msg-1", "scripted injected summary"),
        ),
    );
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open controller with injected adapter");
    let registration = controller.register_agent("mock-worker").expect("agent");
    let direct_refs = controller
        .send_task(&registration, "Run a deterministic scripted mock turn")
        .expect("send task");

    let stopped = controller
        .stop_agent_name("mock-worker", "operator stopped")
        .expect("stop via command path");
    assert_eq!(stopped.session.status, "completed");
    assert_eq!(
        stopped.session.external_session_ref.as_deref(),
        Some("scripted-injected-session")
    );
    assert!(
        controller
            .state()
            .recent_events_for_session(&direct_refs.session_id, 32)
            .expect("events")
            .iter()
            .any(|event| {
                event.kind == "session.stopped"
                    && event
                        .payload_json
                        .contains("Scripted mock stopped session: operator stopped")
            })
    );
}

#[test]
fn controller_default_open_keeps_fake_adapter_output_byte_for_byte() {
    // The default `open` constructor still injects the fake adapter, so its
    // deterministic summary/confidence/status are unchanged after the RTL2
    // injection refactor.
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), temp_root())
        .expect("open controller");
    let registration = controller.register_agent("fake-codex").expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Inspect the project and write a status summary",
        )
        .expect("send task");

    assert_eq!(refs.external_session_ref, "fake-adapter-session-fake-codex");
    let observation = controller.observe(&refs).expect("observe");
    assert_eq!(
        observation.session.latest_summary.as_deref(),
        Some(
            "Fake adapter processed goal for fake-codex: Inspect the project and write a status summary"
        )
    );
    assert_eq!(observation.session.latest_confidence, Some(82));
    assert_eq!(observation.session.status, "active");
}

#[test]
fn turn_loop_runs_a_scripted_single_turn_observe_project_emit_cycle() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // RTL3: a turn opens, the adapter produces normalized events, the controller
    // projects them, and the loop emits a TurnFinished carrying the stop reason,
    // summary refs, and observed tool refs.
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session"));
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open controller");
    let registration = controller.register_agent("loop-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run one observe->project->emit cycle")
        .expect("send task");

    let turn_id = TurnId::new("turn-loop-1");
    let batch = ScriptedMockTurn::new("turn-loop-1")
        .message_delta("msg-1", "inspecting state")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .message_completed("msg-2", "state inspected")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);

    let finished = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("run turn");

    // Emit: the outcome reports normal completion and is keyed to this turn.
    assert_eq!(finished.turn_id, turn_id);
    assert_eq!(finished.stop_reason, TurnStopReason::Completed);
    assert!(finished.observed_terminal_event());
    // Summary refs are the item events' refs (two: delta msg-1, completed msg-2);
    // observed tool refs are the tool event refs, deduped to one (tool-1).
    assert_eq!(finished.summary_refs, vec!["msg-1", "msg-2"]);
    assert_eq!(finished.observed_tool_refs, vec!["tool-1"]);
    // The replay report comes straight from the existing projection path.
    assert_eq!(finished.replay.input_event_count, 5);
    assert_eq!(finished.replay.summary_event_count, 2);
    assert_eq!(finished.replay.tool_event_count, 2);
    assert_eq!(finished.replay.completed_turn_count, 1);

    // Project: the loop drove the existing ingestion path, so the read models
    // are keyed to this turn and observe the scripted tool.
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_name == "capo.agent_status"
            && tool.status == "completed"
            && tool.turn_id.as_deref() == Some("turn-loop-1")
    }));
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 32)
        .expect("events");
    assert!(events.iter().any(|event| {
        event.kind == "session.summary_updated" && event.turn_id.as_deref() == Some("turn-loop-1")
    }));
    assert!(events.iter().any(|event| {
        event.kind == "evidence.recorded" && event.turn_id.as_deref() == Some("turn-loop-1")
    }));
}

#[test]
fn turn_loop_interrupt_and_stop_commands_map_onto_finished_outcomes() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    // Interrupt: drive a turn to a Finished/Interrupted outcome.
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session-int")),
    )
    .expect("open controller");
    let registration = controller.register_agent("loop-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "interrupt me")
        .expect("send");
    let interrupt_turn_id = TurnId::new("turn-int-1");
    let interrupted = controller
        .interrupt_turn(&registration, &refs, &interrupt_turn_id, "operator paused")
        .expect("interrupt turn");
    // The whole new behavior is mapping the command's turn identity onto the
    // loop, so assert the full TurnFinished shape (not just stop_reason, which
    // the underlying interrupt command already determines).
    assert_eq!(interrupted.turn_id, interrupt_turn_id);
    assert_eq!(interrupted.stop_reason, TurnStopReason::Interrupted);
    assert!(interrupted.observed_terminal_event());
    assert!(interrupted.summary_refs.is_empty());
    assert!(interrupted.observed_tool_refs.is_empty());
    assert_eq!(interrupted.replay, AdapterReplayReport::default());
    assert_eq!(
        controller.observe(&refs).expect("observe").session.status,
        "canceled"
    );
    // The turn id is persisted onto the terminal event, not cosmetic: an
    // observer querying events by this turn finds the interrupt.
    let interrupt_events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 32)
        .expect("events");
    assert!(interrupt_events.iter().any(|event| {
        event.kind == "session.interrupted" && event.turn_id.as_deref() == Some("turn-int-1")
    }));

    // Stop: drive a turn to a Finished/Stopped outcome.
    let stop_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session-stop")),
    )
    .expect("open controller");
    let stop_registration = stop_controller
        .register_agent("loop-worker")
        .expect("agent");
    let stop_refs = stop_controller
        .send_task(&stop_registration, "stop me")
        .expect("send");
    let stop_turn_id = TurnId::new("turn-stop-1");
    let stopped = stop_controller
        .stop_turn(
            &stop_registration,
            &stop_refs,
            &stop_turn_id,
            "operator stopped",
        )
        .expect("stop turn");
    assert_eq!(stopped.turn_id, stop_turn_id);
    assert_eq!(stopped.stop_reason, TurnStopReason::Stopped);
    assert!(stopped.observed_terminal_event());
    assert!(stopped.summary_refs.is_empty());
    assert!(stopped.observed_tool_refs.is_empty());
    assert_eq!(stopped.replay, AdapterReplayReport::default());
    assert_eq!(
        stop_controller
            .observe(&stop_refs)
            .expect("observe")
            .session
            .status,
        "completed"
    );
    let stop_events = stop_controller
        .state()
        .recent_events_for_session(&stop_refs.session_id, 32)
        .expect("events");
    assert!(stop_events.iter().any(|event| {
        event.kind == "session.stopped" && event.turn_id.as_deref() == Some("turn-stop-1")
    }));
}

#[test]
fn turn_loop_run_turn_maps_terminal_adapter_events_onto_stop_reasons() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // RTL3: a scripted batch ending in adapter.turn_failed resolves to
    // TurnStopReason::Failed (and projects run.exited keyed to the turn); a batch
    // ending in adapter.turn_interrupted resolves to Interrupted (and projects
    // session.interrupted). These terminal arms of finish_turn are otherwise only
    // reachable through the command path.
    // Failed branch.
    let fail_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session-fail")),
    )
    .expect("open controller");
    let fail_registration = fail_controller
        .register_agent("loop-worker")
        .expect("agent");
    let fail_refs = fail_controller
        .send_task(&fail_registration, "fail this turn")
        .expect("send task");
    let fail_turn = TurnId::new("turn-fail-1");
    let fail_batch = ScriptedMockTurn::new("turn-fail-1")
        .message_delta("msg-f1", "working")
        .failed("err-1", "boom")
        .normalized_events(&fail_refs.external_session_ref);
    let failed = fail_controller
        .run_turn(&fail_refs, &fail_turn, &fail_batch)
        .expect("run failed turn");
    assert_eq!(failed.stop_reason, TurnStopReason::Failed);
    assert!(failed.observed_terminal_event());
    let fail_events = fail_controller
        .state()
        .recent_events_for_session(&fail_refs.session_id, 32)
        .expect("events");
    assert!(fail_events.iter().any(|event| {
        event.kind == "run.exited" && event.turn_id.as_deref() == Some("turn-fail-1")
    }));

    // Interrupted branch via a scripted terminal event (not the command path).
    let int_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session-aint")),
    )
    .expect("open controller");
    let int_registration = int_controller.register_agent("loop-worker").expect("agent");
    let int_refs = int_controller
        .send_task(&int_registration, "interrupt this turn")
        .expect("send task");
    let int_turn = TurnId::new("turn-adapter-int-1");
    let int_batch = ScriptedMockTurn::new("turn-adapter-int-1")
        .message_delta("msg-i1", "working")
        .interrupted("int-1", "halted")
        .normalized_events(&int_refs.external_session_ref);
    let interrupted = int_controller
        .run_turn(&int_refs, &int_turn, &int_batch)
        .expect("run interrupted turn");
    assert_eq!(interrupted.stop_reason, TurnStopReason::Interrupted);
    assert!(interrupted.observed_terminal_event());
    let int_events = int_controller
        .state()
        .recent_events_for_session(&int_refs.session_id, 32)
        .expect("events");
    assert!(int_events.iter().any(|event| {
        event.kind == "session.interrupted"
            && event.turn_id.as_deref() == Some("turn-adapter-int-1")
    }));
}

#[test]
fn turn_loop_projected_turn_rebuilds_identically_after_restart_replay() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    let state_root = temp_root();
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session"));
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open controller");
    let registration = controller.register_agent("loop-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run a replay-stable turn")
        .expect("send task");
    let turn_id = TurnId::new("turn-replay-1");
    let batch = ScriptedMockTurn::new("turn-replay-1")
        .message_delta("msg-1", "inspecting state")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .message_completed("msg-2", "state inspected")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);

    let finished = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("run turn");

    // Capture the projected read models the turn produced.
    let session_before = controller
        .state()
        .session(&refs.session_id)
        .expect("session")
        .expect("session present");
    let tools_before = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let observations_before = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    let evidence_before = controller
        .state()
        .evidence_for_session(&refs.session_id)
        .expect("evidence");
    let event_count_before = controller.state().event_count().expect("event count");

    // Restart: reopen the state store from the same root and rebuild projections
    // purely from the persisted event log.
    let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");

    // The rebuilt read models are byte-identical: events are the source of
    // truth, the projection is a pure fold.
    assert_eq!(
        reopened
            .session(&refs.session_id)
            .expect("session")
            .expect("session present"),
        session_before
    );
    assert_eq!(
        reopened
            .tool_calls_for_session(&refs.session_id)
            .expect("tool calls"),
        tools_before
    );
    assert_eq!(
        reopened
            .tool_observations_for_session(&refs.session_id)
            .expect("tool observations"),
        observations_before
    );
    assert_eq!(
        reopened
            .evidence_for_session(&refs.session_id)
            .expect("evidence"),
        evidence_before
    );
    assert_eq!(
        reopened.event_count().expect("event count"),
        event_count_before
    );

    // Reconstruct the outcome from PERSISTED STATE on a fresh controller that
    // never saw the in-memory batch: open a new controller over the rebuilt
    // store and re-derive TurnFinished purely from the turn-keyed event log.
    // This is the genuine replay-stability proof -- nothing here re-feeds the
    // original `batch`, so equality cannot hold by construction.
    let reconstructed_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session")),
    )
    .expect("reopen controller");
    let reconstructed = reconstructed_controller
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct outcome");
    // The equality-significant outcome matches the live one (the volatile
    // `replay` append-count diagnostic is excluded by construction: the
    // reconstruction reports a default report, the live run reports the
    // first-pass counts).
    assert_eq!(reconstructed.turn_id, finished.turn_id);
    assert_eq!(reconstructed.stop_reason, finished.stop_reason);
    assert_eq!(
        reconstructed.observed_terminal_event(),
        finished.observed_terminal_event()
    );
    assert_eq!(reconstructed.summary_refs, finished.summary_refs);
    assert_eq!(
        reconstructed.observed_tool_refs,
        finished.observed_tool_refs
    );
    let mut expected_stable = finished.clone();
    expected_stable.replay = AdapterReplayReport::default();
    assert_eq!(reconstructed, expected_stable);

    // Re-running the loop over the same batch is also idempotent (idempotency
    // keys on every projected event): no new events are appended, and only the
    // volatile replay append-count changes.
    let replayed = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("replay turn");
    assert_eq!(replayed.replay.appended_event_count, 0);
    let mut replayed_stable = replayed.clone();
    replayed_stable.replay = AdapterReplayReport::default();
    assert_eq!(replayed_stable, expected_stable);
    assert_eq!(
        controller.state().event_count().expect("event count"),
        event_count_before
    );
}

#[test]
fn turn_loop_dispatch_derivation_matches_run_turn_for_the_same_batch() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // RTL4: the dispatch substrate ingests the normalized batch and then the
    // loop ANNOTATES that run with a TurnFinished. The server reuses
    // `derive_turn_finished` (the public outcome classifier) over the same
    // batch the dispatch run projected. This proves the dispatch-driven
    // annotation cannot drift from the in-loop `run_turn` outcome: both call the
    // one pure derivation, so there is a single completion model.
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("loop-session"));
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open controller");
    let registration = controller.register_agent("loop-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Reconcile loop outcome with dispatch")
        .expect("send task");

    let turn_id = TurnId::new("turn-reconcile-1");
    let batch = ScriptedMockTurn::new("turn-reconcile-1")
        .message_delta("msg-1", "inspecting state")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .message_completed("msg-2", "state inspected")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);

    // The loop's in-controller path: observe -> project -> emit.
    let via_loop = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("run turn");

    // The dispatch path's emit step: the server derives the outcome from the
    // same batch (with the dispatch run's replay counts) AFTER the dispatch
    // primitives ingested it. Equality-significant fields must match exactly.
    let via_dispatch =
        FakeBoundaryController::derive_turn_finished(&turn_id, &batch, via_loop.replay.clone());
    assert_eq!(via_dispatch, via_loop);
}

fn temp_root() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("capo-controller-{nanos}-{counter}"))
}
