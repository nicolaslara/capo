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
                            provenance: Default::default(),
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

/// FIX2 regression: two interrupts on the SAME session but DIFFERENT turns must
/// each persist a DISTINCT, turn-keyed `session.interrupted` event, and each
/// must reconstruct correctly via `reconstruct_turn_finished` scoped to its own
/// turn.
///
/// Before FIX2(a) the interrupt event_id (and thus the idempotency key) was
/// `event-session-interrupted-{session_id}` -- session-scoped, no turn id. A
/// second interrupt in the same session (different turn) deduped against the
/// first key, so NO second event was persisted even though `interrupt_turn`
/// still returned a `TurnFinished` claiming `Interrupted`. Including the turn id
/// in the event_id makes the two interrupts distinct and individually
/// reconstructable by turn.
#[test]
fn two_interrupts_on_same_session_distinct_turns_persist_and_reconstruct() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("two-interrupt-session")),
    )
    .expect("open controller");
    let registration = controller
        .register_agent("loop-worker")
        .expect("register agent");
    let refs = controller
        .send_task(&registration, "interrupt me twice")
        .expect("send task");

    let first_turn = TurnId::new("turn-interrupt-a");
    let second_turn = TurnId::new("turn-interrupt-b");

    let first = controller
        .interrupt_turn(&registration, &refs, &first_turn, "first interrupt")
        .expect("first interrupt turn");
    let second = controller
        .interrupt_turn(&registration, &refs, &second_turn, "second interrupt")
        .expect("second interrupt turn");

    assert_eq!(first.turn_id, first_turn);
    assert_eq!(second.turn_id, second_turn);

    // Both terminal events were actually persisted, keyed to their own turn --
    // the second did NOT dedup away against a session-scoped key.
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 64)
        .expect("events");
    let interrupted_turns: Vec<&str> = events
        .iter()
        .filter(|event| event.kind == "session.interrupted")
        .filter_map(|event| event.turn_id.as_deref())
        .collect();
    assert!(
        interrupted_turns.contains(&"turn-interrupt-a"),
        "first turn's interrupt must be persisted, got {interrupted_turns:?}"
    );
    assert!(
        interrupted_turns.contains(&"turn-interrupt-b"),
        "second turn's interrupt must be persisted (no dedup collision), got {interrupted_turns:?}"
    );

    // Distinct event ids / idempotency keys (the FIX2(a) keying), so a replay
    // leaves each interrupt persisted exactly once.
    let first_event = events
        .iter()
        .find(|event| {
            event.kind == "session.interrupted"
                && event.turn_id.as_deref() == Some("turn-interrupt-a")
        })
        .expect("first interrupted event");
    let second_event = events
        .iter()
        .find(|event| {
            event.kind == "session.interrupted"
                && event.turn_id.as_deref() == Some("turn-interrupt-b")
        })
        .expect("second interrupted event");
    assert_ne!(first_event.event_id, second_event.event_id);
    assert_ne!(first_event.idempotency_key, second_event.idempotency_key);

    // Each turn reconstructs correctly and INDEPENDENTLY from the persisted,
    // turn-keyed event log.
    let reconstructed_first = controller
        .reconstruct_turn_finished(&refs, &first_turn)
        .expect("reconstruct first turn");
    let reconstructed_second = controller
        .reconstruct_turn_finished(&refs, &second_turn)
        .expect("reconstruct second turn");
    assert_eq!(reconstructed_first.turn_id, first_turn);
    assert_eq!(reconstructed_first.stop_reason, TurnStopReason::Interrupted);
    assert!(reconstructed_first.observed_terminal_event());
    assert_eq!(reconstructed_second.turn_id, second_turn);
    assert_eq!(
        reconstructed_second.stop_reason,
        TurnStopReason::Interrupted
    );
    assert!(reconstructed_second.observed_terminal_event());
}

/// FIX2(b): a command-path interrupt/stop (which names an agent, not a turn)
/// must resolve the session's active turn and persist a NON-NULL `turn_id`, so
/// the terminal event is reconstructable by turn rather than orphaned at
/// `turn_id = NULL`.
#[test]
fn command_path_interrupt_and_stop_persist_non_null_turn_id() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    // Interrupt via the command entry point.
    let interrupt_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("cmd-interrupt-session")),
    )
    .expect("open controller");
    let interrupt_registration = interrupt_controller
        .register_agent("loop-worker")
        .expect("register agent");
    let interrupt_refs = interrupt_controller
        .send_task(&interrupt_registration, "command interrupt me")
        .expect("send task");
    interrupt_controller
        .interrupt_agent_name("loop-worker", "operator paused")
        .expect("interrupt via command path");

    let interrupt_event = interrupt_controller
        .state()
        .recent_events_for_session(&interrupt_refs.session_id, 64)
        .expect("events")
        .into_iter()
        .find(|event| event.kind == "session.interrupted")
        .expect("session.interrupted event persisted");
    let interrupt_turn = interrupt_event
        .turn_id
        .clone()
        .expect("command-path interrupt must persist a non-null turn_id");
    // The resolved turn is the session's active (send_task) turn.
    assert_eq!(interrupt_turn, "turn-loop-worker");
    // ...and the terminal event reconstructs by that turn.
    let reconstructed = interrupt_controller
        .reconstruct_turn_finished(&interrupt_refs, &TurnId::new(interrupt_turn))
        .expect("reconstruct command-path interrupt");
    assert_eq!(reconstructed.stop_reason, TurnStopReason::Interrupted);
    assert!(reconstructed.observed_terminal_event());

    // Stop via the command entry point.
    let stop_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("cmd-stop-session")),
    )
    .expect("open controller");
    let stop_registration = stop_controller
        .register_agent("loop-worker")
        .expect("register agent");
    let stop_refs = stop_controller
        .send_task(&stop_registration, "command stop me")
        .expect("send task");
    stop_controller
        .stop_agent_name("loop-worker", "operator stopped")
        .expect("stop via command path");

    let stop_event = stop_controller
        .state()
        .recent_events_for_session(&stop_refs.session_id, 64)
        .expect("events")
        .into_iter()
        .find(|event| event.kind == "session.stopped")
        .expect("session.stopped event persisted");
    let stop_turn = stop_event
        .turn_id
        .expect("command-path stop must persist a non-null turn_id");
    assert_eq!(stop_turn, "turn-loop-worker");
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

/// Replay-identity on a LONG multi-turn session: flood a session with far more
/// than 256 persisted events across many turns, then reconstruct an EARLY turn
/// purely from the persisted, turn-keyed event log and assert the reconstructed
/// `TurnFinished` is structurally identical to the live one the loop emitted.
///
/// This is the regression for the truncation bug: `reconstruct_turn_finished`
/// previously read a 256-event recency WINDOW (`recent_events_for_session`),
/// so once the session accrued >256 later events, an early turn's events fell
/// out of the window and the early turn reconstructed with empty
/// summary_refs/observed_tool_refs, `observed_terminal_event = false`, and a
/// fallback `stop_reason = Completed`. With the turn-scoped UNBOUNDED query
/// (`events_for_session_turn`) the early turn re-derives from its COMPLETE
/// event set, so the live and reconstructed outcomes match.
///
/// FAILS against the old 256-cap (early turn reconstructs empty/wrong);
/// PASSES with the unbounded turn-scoped query.
#[test]
fn turn_loop_reconstructs_early_turn_on_long_session_past_256_event_cap() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    let state_root = temp_root();
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("flood-session"));
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open controller");
    let registration = controller.register_agent("flood-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Flood a session across many turns")
        .expect("send task");

    // Drive the FIRST (early) turn with a distinctive, non-empty batch: a tool
    // round-trip + summaries + a terminal `turn_completed`. This is the turn we
    // later reconstruct from the persisted log.
    let early_turn = TurnId::new("turn-early-1");
    let early_batch = ScriptedMockTurn::new("turn-early-1")
        .message_delta("early-msg-1", "inspecting state")
        .tool_requested("early-tool-1", "capo.agent_status")
        .tool_completed("early-tool-1", "capo.agent_status", "agent is running")
        .message_completed("early-msg-2", "state inspected")
        .turn_completed("early-done-1")
        .normalized_events(&refs.external_session_ref);
    let early_finished = controller
        .run_turn(&refs, &early_turn, &early_batch)
        .expect("run early turn");

    // Sanity: the live early turn really did observe summaries, a tool, a
    // terminal completed event -- the exact content the truncation bug erased.
    assert!(
        !early_finished.summary_refs.is_empty(),
        "live early turn must have summary refs"
    );
    assert!(
        !early_finished.observed_tool_refs.is_empty(),
        "live early turn must have tool refs"
    );
    assert!(early_finished.observed_terminal_event());
    assert_eq!(early_finished.stop_reason, TurnStopReason::Completed);

    // Flood the SAME session with many LATER turns so the persisted event count
    // far exceeds the old 256-event recency window, pushing the early turn's
    // events out of that window entirely.
    let event_count_before_flood = controller.state().event_count().expect("event count");
    for index in 0..60usize {
        let turn_id = TurnId::new(format!("turn-flood-{index}"));
        let batch = ScriptedMockTurn::new(format!("turn-flood-{index}"))
            .message_delta(format!("flood-msg-{index}-1"), "working")
            .tool_requested(format!("flood-tool-{index}"), "capo.agent_status")
            .tool_completed(
                format!("flood-tool-{index}"),
                "capo.agent_status",
                "still running",
            )
            .message_completed(format!("flood-msg-{index}-2"), "progress")
            .turn_completed(format!("flood-done-{index}"))
            .normalized_events(&refs.external_session_ref);
        controller
            .run_turn(&refs, &turn_id, &batch)
            .expect("run flood turn");
    }

    // The session now holds well over 256 events, so the early turn's events sit
    // outside any 256-event recency window.
    let total_events = controller.state().event_count().expect("event count");
    let session_events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 1)
        .expect("latest event");
    let latest_sequence = session_events.last().map_or(0, |event| event.sequence);
    assert!(
        total_events > event_count_before_flood + 256,
        "flood must push the session past the 256-event cap (saw {total_events} total events)"
    );
    let early_max_sequence = controller
        .state()
        .events_for_session_turn(&refs.session_id, early_turn.as_str())
        .expect("early turn events")
        .iter()
        .map(|event| event.sequence)
        .max()
        .expect("early turn has events");
    assert!(
        latest_sequence - early_max_sequence > 256,
        "early turn must be more than 256 events behind the latest event \
         (gap = {})",
        latest_sequence - early_max_sequence
    );

    // Reconstruct the EARLY turn from PERSISTED STATE on a fresh controller that
    // never saw the in-memory batch: reopen over the rebuilt store and re-derive
    // purely from the turn-keyed event log. With the old 256-cap this returns an
    // empty/Completed-by-fallback outcome; with the unbounded turn-scoped query
    // it matches the live early outcome exactly.
    let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");
    let reconstructed_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("flood-session")),
    )
    .expect("reopen controller");
    let reconstructed = reconstructed_controller
        .reconstruct_turn_finished(&refs, &early_turn)
        .expect("reconstruct early outcome");

    // Equality-significant fields match the live early outcome (the volatile
    // `replay` append-count diagnostic is excluded: the reconstruction reports a
    // default report).
    assert_eq!(reconstructed.turn_id, early_finished.turn_id);
    assert_eq!(reconstructed.stop_reason, early_finished.stop_reason);
    assert_eq!(
        reconstructed.observed_terminal_event(),
        early_finished.observed_terminal_event()
    );
    assert_eq!(reconstructed.summary_refs, early_finished.summary_refs);
    assert_eq!(
        reconstructed.observed_tool_refs,
        early_finished.observed_tool_refs
    );
    let mut expected_stable = early_finished.clone();
    expected_stable.replay = AdapterReplayReport::default();
    assert_eq!(reconstructed, expected_stable);
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

// --- RTL5: RealBoundaryController -----------------------------------------

/// Drive an identical scripted register -> send_task -> turn sequence on both
/// the fake handle and the real handle over the SAME scripted adapter, and
/// assert the persisted read models are byte-compatible.
///
/// This is the RTL5 parity proof: the real controller is the production
/// consumer of the RTL3 loop and the RTL1 trait, but it persists through the
/// one `append_event`/projection path, so identical scripted output yields
/// identical projections (the only divergence allowed is the in-memory
/// `external_session_ref`, which both handles derive from their own session
/// label -- here we hand both the same label).
#[test]
fn real_controller_read_models_match_fake_path_for_identical_scripted_output() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    fn run_sequence_on<C>(
        open: C,
    ) -> (
        SessionProjection,
        Vec<capo_state::ToolCallProjection>,
        Vec<capo_state::ToolObservationProjection>,
        Vec<capo_state::EvidenceProjection>,
        i64,
        TurnFinished,
    )
    where
        C: FnOnce(AgentAdapterHandle) -> SqliteStateStoreBundle,
    {
        let scripted =
            AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl5-parity-session"));
        let bundle = open(scripted);
        let registration = bundle.register("rtl5-worker");
        let refs = bundle.send_task(&registration, "Run an RTL5 parity turn");
        let turn_id = TurnId::new("turn-rtl5-parity-1");
        let batch = ScriptedMockTurn::new("turn-rtl5-parity-1")
            .message_delta("msg-1", "inspecting state")
            .tool_requested("tool-1", "capo.agent_status")
            .tool_completed("tool-1", "capo.agent_status", "agent is running")
            .message_completed("msg-2", "state inspected")
            .turn_completed("done-1")
            .normalized_events(&refs.external_session_ref);
        let finished = bundle.run_turn(&refs, &turn_id, &batch);
        let state = bundle.state();
        (
            state
                .session(&refs.session_id)
                .expect("session")
                .expect("session present"),
            state
                .tool_calls_for_session(&refs.session_id)
                .expect("tool calls"),
            state
                .tool_observations_for_session(&refs.session_id)
                .expect("tool observations"),
            state
                .evidence_for_session(&refs.session_id)
                .expect("evidence"),
            state.event_count().expect("event count"),
            finished,
        )
    }

    let fake_root = temp_root();
    let fake = run_sequence_on(|adapter| {
        SqliteStateStoreBundle::Fake(Box::new(
            FakeBoundaryController::open_with_adapter(
                ProjectId::new("project-capo"),
                &fake_root,
                adapter,
            )
            .expect("open fake controller"),
        ))
    });

    let real_root = temp_root();
    let real = run_sequence_on(|adapter| {
        SqliteStateStoreBundle::Real(Box::new(
            RealBoundaryController::open_with_adapter(
                ProjectId::new("project-capo"),
                &real_root,
                adapter,
            )
            .expect("open real controller"),
        ))
    });

    assert_eq!(real.0, fake.0, "session projection diverged");
    assert_eq!(real.1, fake.1, "tool-call projections diverged");
    assert_eq!(real.2, fake.2, "tool-observation projections diverged");
    assert_eq!(real.3, fake.3, "evidence projections diverged");
    assert_eq!(real.4, fake.4, "event count diverged");
    assert_eq!(real.5, fake.5, "TurnFinished outcome diverged");
}

/// Restart/replay: a turn driven through the real controller rebuilds
/// byte-identically from the persisted event log, and a re-run is idempotent
/// (0 new events). Mirrors
/// `turn_loop_projected_turn_rebuilds_identically_after_restart_replay` for the
/// production handle, satisfying the RTL5 restart/replay verification.
#[test]
fn real_controller_projected_turn_rebuilds_identically_after_restart_replay() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    let state_root = temp_root();
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl5-replay-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("rtl5-replay-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Run a real replay-stable turn")
        .expect("send task");
    let turn_id = TurnId::new("turn-rtl5-replay-1");
    let batch = ScriptedMockTurn::new("turn-rtl5-replay-1")
        .message_delta("msg-1", "inspecting state")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .message_completed("msg-2", "state inspected")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);

    let finished = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("run turn");

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

    // Restart: reopen the state store and rebuild projections from the log.
    let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");

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

    // Reconstruct the outcome from PERSISTED STATE on a fresh real controller
    // that never saw the in-memory batch.
    let reconstructed_controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl5-replay-session")),
    )
    .expect("reopen real controller");
    let reconstructed = reconstructed_controller
        .core()
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct outcome");
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

    // Idempotent re-run: no new events, only the volatile replay counts change.
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

// --- RTL12: parity criterion + parity-equivalence -------------------------

/// A stable, adapter-identity-independent summary of where a lifecycle landed:
/// the terminal `(task, agent, session, run)` statuses, the causal session
/// event-kind sequence, and the event count. Two routings are "equivalent" when
/// their fingerprints match exactly.
type LifecycleFingerprint = (String, String, String, String, Vec<String>, i64);

/// The causal session event-kind sequence (sequence order), dropping the
/// per-request audit envelope whose idempotency key embeds the command id. This
/// is the "event sequence modulo adapter-identity fields" the RTL12
/// parity-equivalence criterion compares.
fn session_event_kind_sequence(state: &SqliteStateStore, session_id: &SessionId) -> Vec<String> {
    let mut events = state
        .recent_events_for_session(session_id, 256)
        .expect("session events");
    events.sort_by_key(|event| event.sequence);
    events
        .into_iter()
        .filter(|event| event.kind != "server.request_handled")
        .map(|event| event.kind)
        .collect()
}

/// A stable, adapter-identity-independent summary of where a lifecycle landed.
fn lifecycle_fingerprint(
    bundle: &SqliteStateStoreBundle,
    refs: &FakeRunRefs,
) -> LifecycleFingerprint {
    let state = bundle.state();
    let task = state
        .task(&refs.task_id)
        .expect("task")
        .expect("task present");
    let agent = state
        .agent(&refs.agent_id)
        .expect("agent")
        .expect("agent present");
    let session = state
        .session(&refs.session_id)
        .expect("session")
        .expect("session present");
    let run = state.run(&refs.run_id).expect("run").expect("run present");
    (
        task.capo_execution_status,
        agent.status,
        session.status,
        run.status,
        session_event_kind_sequence(state, &refs.session_id),
        state.event_count().expect("event count"),
    )
}

/// RTL12 parity criterion: `RealBoundaryController` passes the IDENTICAL
/// deterministic suite (`send`/`steer`/`interrupt`/`stop`, restart/replay) that
/// `FakeBoundaryController` passes.
///
/// What this proves, and what it does NOT: `RealBoundaryController` is, by
/// construction, a zero-cost pass-through over the same `FakeBoundaryController`
/// orchestration core (see `real_controller.rs`: every method forwards to
/// `self.core.<same_method>`; the return types are aliases). So parity here
/// holds by construction -- there is no second implementation that could
/// disagree. This test is therefore a REGRESSION GUARD that the real handle
/// keeps delegating to the one shared core (i.e. that the core is never forked
/// into a divergent real path), NOT a proof that two independent
/// implementations were independently validated and found to agree. The same
/// `register -> send -> steer -> interrupt` and `-> stop` sequences are driven
/// over BOTH handles, over the same scripted-mock adapter and the same session
/// label, and the resulting lifecycles are asserted equal (terminal statuses +
/// causal session event-kind sequence + event count); both then rebuild
/// identically from their persisted event logs, satisfying the restart/replay
/// half of the suite. Given the shared core, the only way these assertions can
/// fail is if the real handle stops delegating or the one core becomes
/// nondeterministic -- exactly the regressions the guard exists to catch.
#[test]
fn real_controller_passes_the_identical_send_steer_interrupt_stop_suite() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    /// Drive `register -> send -> steer -> <terminal>` on one handle and return
    /// its fingerprint, plus the refs and root for the restart/replay step.
    fn run_suite(
        open: impl FnOnce(AgentAdapterHandle) -> SqliteStateStoreBundle,
        terminal: Terminal,
    ) -> (LifecycleFingerprint, FakeRunRefs, SqliteStateStoreBundle) {
        let adapter =
            AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl12-parity-session"));
        let bundle = open(adapter);
        let registration = bundle.register("rtl12-parity-worker");
        let refs = bundle.send_task(&registration, "Run the RTL12 parity suite");
        bundle.redirect(&registration, &refs, "Refocus on the highest-value subtask");
        match terminal {
            Terminal::Interrupt => {
                bundle.interrupt(&registration, &refs, "operator pause");
            }
            Terminal::Stop => {
                bundle.stop(&registration, &refs, "operator stop");
            }
        }
        let fingerprint = lifecycle_fingerprint(&bundle, &refs);
        (fingerprint, refs, bundle)
    }

    for terminal in [Terminal::Interrupt, Terminal::Stop] {
        let fake_root = temp_root();
        let (fake_fp, fake_refs, _fake) = run_suite(
            |adapter| {
                SqliteStateStoreBundle::Fake(Box::new(
                    FakeBoundaryController::open_with_adapter(
                        ProjectId::new("project-capo"),
                        &fake_root,
                        adapter,
                    )
                    .expect("open fake controller"),
                ))
            },
            terminal,
        );

        let real_root = temp_root();
        let (real_fp, real_refs, real) = run_suite(
            |adapter| {
                SqliteStateStoreBundle::Real(Box::new(
                    RealBoundaryController::open_with_adapter(
                        ProjectId::new("project-capo"),
                        &real_root,
                        adapter,
                    )
                    .expect("open real controller"),
                ))
            },
            terminal,
        );

        // The real handle passes the same suite: identical terminal lifecycle.
        assert_eq!(
            real_fp, fake_fp,
            "real controller diverged from fake on the {terminal:?} suite"
        );
        assert_eq!(real_refs.session_id, fake_refs.session_id);

        // Restart/replay half: the real handle's projections rebuild identically
        // from its persisted event log (the fake half is the established RTL3/RTL5
        // restart/replay coverage; here we prove the real handle satisfies it on
        // the same suite).
        let reopened = SqliteStateStore::open(&real_root).expect("reopen real state");
        reopened.rebuild_projections().expect("rebuild");
        assert_eq!(
            reopened
                .session(&real_refs.session_id)
                .expect("session")
                .expect("session present"),
            real.state()
                .session(&real_refs.session_id)
                .expect("session")
                .expect("session present"),
            "real controller session diverged after restart/replay on the {terminal:?} suite"
        );
        assert_eq!(
            reopened
                .run(&real_refs.run_id)
                .expect("run")
                .expect("run present"),
            real.state()
                .run(&real_refs.run_id)
                .expect("run")
                .expect("run present"),
        );
        assert_eq!(
            reopened.event_count().expect("event count"),
            real.state().event_count().expect("event count"),
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Terminal {
    Interrupt,
    Stop,
}

/// RTL12 parity-equivalence: for a scripted turn, the fake and real paths
/// produce equivalent event sequences.
///
/// Both handles drive the SAME scripted multi-event turn through the RTL3 loop
/// (`run_turn`) over the same adapter and session label. The persisted causal
/// session event-kind sequence, the stable projections, and the `TurnFinished`
/// outcome must match -- the equivalence the RTL12 cutover gates on. This is the
/// turn-loop-level companion to the RTL11 command-surface equivalence test
/// (`both_routings_handle_send_steer_and_interrupt_equivalently`).
///
/// As with the sibling suite above, the real path delegates to the same core as
/// the fake path, so this equivalence holds by construction; the test is a
/// regression guard against that core forking, not a cross-validation of two
/// implementations. Both runs use the identical adapter and session label, so
/// there are no adapter-identity fields to differ -- the comparison is over the
/// full persisted sequence/projection/outcome, not "modulo" anything.
#[test]
fn fake_and_real_paths_produce_equivalent_event_sequences_for_a_scripted_turn() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    fn run_scripted_turn(
        open: impl FnOnce(AgentAdapterHandle) -> SqliteStateStoreBundle,
    ) -> (Vec<String>, SessionProjection, TurnFinished, FakeRunRefs) {
        let adapter =
            AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl12-equiv-session"));
        let bundle = open(adapter);
        let registration = bundle.register("rtl12-equiv-worker");
        let refs = bundle.send_task(&registration, "Run an RTL12 equivalence turn");
        let turn_id = TurnId::new("turn-rtl12-equiv-1");
        let batch = ScriptedMockTurn::new("turn-rtl12-equiv-1")
            .message_delta("msg-1", "inspecting state")
            .tool_requested("tool-1", "capo.agent_status")
            .tool_completed("tool-1", "capo.agent_status", "agent is running")
            .message_completed("msg-2", "state inspected")
            .turn_completed("done-1")
            .normalized_events(&refs.external_session_ref);
        let finished = bundle.run_turn(&refs, &turn_id, &batch);
        let session = bundle
            .state()
            .session(&refs.session_id)
            .expect("session")
            .expect("session present");
        let kinds = session_event_kind_sequence(bundle.state(), &refs.session_id);
        (kinds, session, finished, refs)
    }

    let fake_root = temp_root();
    let fake = run_scripted_turn(|adapter| {
        SqliteStateStoreBundle::Fake(Box::new(
            FakeBoundaryController::open_with_adapter(
                ProjectId::new("project-capo"),
                &fake_root,
                adapter,
            )
            .expect("open fake controller"),
        ))
    });

    let real_root = temp_root();
    let real = run_scripted_turn(|adapter| {
        SqliteStateStoreBundle::Real(Box::new(
            RealBoundaryController::open_with_adapter(
                ProjectId::new("project-capo"),
                &real_root,
                adapter,
            )
            .expect("open real controller"),
        ))
    });

    // Equal event sequences (the causal session event-kind order). There are no
    // adapter-identity fields to factor out: both runs drive the identical
    // scripted adapter and session label, so the comparison is over the full
    // sequence. (The "modulo adapter-identity" framing only matters if the two
    // sides ever ran distinct adapters; here, with the shared core, they cannot.)
    assert_eq!(
        real.0, fake.0,
        "fake and real scripted-turn event sequences diverged"
    );
    // The projected read model and the loop's TurnFinished outcome also match.
    assert_eq!(real.1, fake.1, "session projection diverged");
    assert_eq!(real.2, fake.2, "TurnFinished outcome diverged");
    assert_eq!(real.3.session_id, fake.3.session_id);
    // Sanity: the sequence actually carries the scripted turn's domain events.
    assert!(
        fake.0.iter().any(|kind| kind == "session.summary_updated"),
        "scripted turn should record a summary update: {:?}",
        fake.0
    );
    assert!(
        fake.0.iter().any(|kind| kind == "evidence.recorded"),
        "scripted turn should record evidence on completion: {:?}",
        fake.0
    );
}

#[test]
fn resource_ceiling_classifies_the_first_breach_in_priority_order() {
    use std::time::Duration;

    // RTL7: the pure classifier the loop and the live arm both consult. Turns
    // are checked before wall-clock before token/cost, so the abort reason is
    // deterministic for a given usage.
    let ceiling = RunResourceCeiling::for_live_provider(2, Duration::from_secs(30), 1_000);

    // Within every bound: no breach.
    assert_eq!(
        ceiling.breach(RunResourceUsage {
            turns_taken: 2,
            wall_clock_elapsed: Duration::from_secs(30),
            token_cost: 1_000,
        }),
        None
    );
    // Over max turns: turns win the priority order even if other bounds are also
    // over.
    assert_eq!(
        ceiling.breach(RunResourceUsage {
            turns_taken: 3,
            wall_clock_elapsed: Duration::from_secs(99),
            token_cost: 9_999,
        }),
        Some(CeilingBreach::MaxTurns {
            limit: 2,
            observed: 3
        })
    );
    // Turns OK, wall-clock over: wall-clock is the breach.
    assert_eq!(
        ceiling.breach(RunResourceUsage {
            turns_taken: 1,
            wall_clock_elapsed: Duration::from_secs(31),
            token_cost: 10,
        }),
        Some(CeilingBreach::WallClock {
            limit: Duration::from_secs(30),
            observed: Duration::from_secs(31)
        })
    );
    // Turns + wall-clock OK, token/cost over: token/cost is the breach.
    assert_eq!(
        ceiling.breach(RunResourceUsage {
            turns_taken: 1,
            wall_clock_elapsed: Duration::from_secs(5),
            token_cost: 1_001,
        }),
        Some(CeilingBreach::TokenCost {
            limit: 1_000,
            observed: 1_001
        })
    );
    // The live-provider ceiling always bounds wall-clock and yields a >=1s
    // runtime timeout; the unbounded ceiling does not.
    assert!(ceiling.bounds_wall_clock());
    assert_eq!(ceiling.wall_clock_timeout_seconds(), Some(30));
    assert!(!RunResourceCeiling::unbounded().bounds_wall_clock());
    assert_eq!(
        RunResourceCeiling::unbounded().wall_clock_timeout_seconds(),
        None
    );
}

#[test]
fn run_that_exceeds_max_turns_aborts_with_run_aborted_event_and_projects_no_further_turn() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // RTL7 acceptance: a scripted run that exceeds max-turns aborts with a
    // `run.aborted` event and no further turns are projected.
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("ceiling-session")),
    )
    .expect("open controller");
    let registration = controller.register_agent("ceiling-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run under a max-turns=1 ceiling")
        .expect("send task");

    let ceiling = RunResourceCeiling::max_turns(1);

    // Turn 1 is within the ceiling: it projects and completes.
    let turn1 = TurnId::new("turn-ceiling-1");
    let batch1 = ScriptedMockTurn::new("turn-ceiling-1")
        .message_completed("msg-1", "first turn")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "ok")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);
    let outcome1 = controller
        .run_turn_within_ceiling(
            &refs,
            &turn1,
            &batch1,
            &ceiling,
            RunResourceUsage::default(),
            0,
        )
        .expect("turn 1");
    let finished1 = match &outcome1 {
        CeilingTurnOutcome::Completed(finished) => finished,
        CeilingTurnOutcome::Aborted(breach) => panic!("turn 1 must not abort: {breach:?}"),
    };
    assert_eq!(finished1.turn_id, turn1);
    assert_eq!(finished1.stop_reason, TurnStopReason::Completed);
    let event_count_after_turn1 = controller.state().event_count().expect("count");
    assert!(
        !controller
            .state()
            .recent_events_for_session(&refs.session_id, 64)
            .expect("events")
            .iter()
            .any(|event| event.kind == "run.aborted"),
        "the within-ceiling turn must not abort the run"
    );

    // Turn 2 would be the 2nd turn (over max_turns=1): the loop aborts BEFORE
    // projecting it.
    let turn2 = TurnId::new("turn-ceiling-2");
    let batch2 = ScriptedMockTurn::new("turn-ceiling-2")
        .message_completed("msg-2", "second turn")
        .turn_completed("done-2")
        .normalized_events(&refs.external_session_ref);
    let usage_after_turn1 = RunResourceUsage {
        turns_taken: 1,
        ..RunResourceUsage::default()
    };
    let outcome2 = controller
        .run_turn_within_ceiling(&refs, &turn2, &batch2, &ceiling, usage_after_turn1, 0)
        .expect("turn 2");
    assert_eq!(
        outcome2.breach(),
        Some(CeilingBreach::MaxTurns {
            limit: 1,
            observed: 2
        }),
        "turn 2 must abort against max_turns=1"
    );
    assert!(outcome2.finished().is_none());

    // A `run.aborted` event was recorded, keyed to the aborting turn, and the
    // run projection is now `aborted`.
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 64)
        .expect("events");
    let aborted = events
        .iter()
        .find(|event| event.kind == "run.aborted")
        .expect("run.aborted event recorded");
    assert_eq!(aborted.turn_id.as_deref(), Some("turn-ceiling-2"));
    assert!(aborted.payload_json.contains("max_turns_exceeded"));
    assert_eq!(
        controller
            .state()
            .run(&refs.run_id)
            .expect("run")
            .expect("run present")
            .status,
        "aborted"
    );

    // No further turn was projected: turn-ceiling-2's content never reached the
    // read models, and exactly one event (the abort) was appended after turn 1.
    assert!(
        !events.iter().any(|event| {
            event.turn_id.as_deref() == Some("turn-ceiling-2") && event.kind != "run.aborted"
        }),
        "the aborted turn must not project any of its batch"
    );
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(
        tools
            .iter()
            .all(|tool| tool.turn_id.as_deref() != Some("turn-ceiling-2")),
        "the aborted turn must not project a tool call"
    );
    assert_eq!(
        controller.state().event_count().expect("count"),
        event_count_after_turn1 + 1,
        "exactly one event (run.aborted) is appended for the over-ceiling turn"
    );
}

#[test]
fn aborted_run_stays_aborted_after_restart_replay_and_abort_is_idempotent() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    // RTL7: restart/replay proves the aborted run stays aborted after rebuild,
    // and re-recording the same breach is idempotent (the run aborts exactly
    // once).
    let state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("ceiling-replay-session")),
    )
    .expect("open controller");
    let registration = controller
        .register_agent("ceiling-replay-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Run under a max-turns=1 ceiling")
        .expect("send task");

    let ceiling = RunResourceCeiling::max_turns(1);
    let turn1 = TurnId::new("turn-replay-ceiling-1");
    let batch1 = ScriptedMockTurn::new("turn-replay-ceiling-1")
        .message_completed("msg-1", "first turn")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);
    controller
        .run_turn_within_ceiling(
            &refs,
            &turn1,
            &batch1,
            &ceiling,
            RunResourceUsage::default(),
            0,
        )
        .expect("turn 1");
    let turn2 = TurnId::new("turn-replay-ceiling-2");
    let batch2 = ScriptedMockTurn::new("turn-replay-ceiling-2")
        .message_completed("msg-2", "second turn")
        .turn_completed("done-2")
        .normalized_events(&refs.external_session_ref);
    let usage_after_turn1 = RunResourceUsage {
        turns_taken: 1,
        ..RunResourceUsage::default()
    };
    let aborted = controller
        .run_turn_within_ceiling(&refs, &turn2, &batch2, &ceiling, usage_after_turn1, 0)
        .expect("turn 2");
    assert!(aborted.breach().is_some());
    assert_eq!(
        controller
            .state()
            .run(&refs.run_id)
            .expect("run")
            .expect("present")
            .status,
        "aborted"
    );
    let event_count_before = controller.state().event_count().expect("count");

    // Restart: reopen from the same root and rebuild projections from the event
    // log alone. The run is still aborted.
    let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        reopened
            .run(&refs.run_id)
            .expect("run")
            .expect("present")
            .status,
        "aborted",
        "an aborted run stays aborted after restart/replay"
    );
    assert_eq!(
        reopened.event_count().expect("count"),
        event_count_before,
        "rebuild appends no events"
    );

    // Re-recording the same breach is idempotent: the abort event's idempotency
    // key dedups, so no new event is appended and the run stays aborted once.
    controller
        .abort_run_for_ceiling(
            &refs,
            &turn2,
            CeilingBreach::MaxTurns {
                limit: 1,
                observed: 2,
            },
        )
        .expect("re-abort");
    assert_eq!(
        controller.state().event_count().expect("count"),
        event_count_before,
        "re-recording the same breach appends nothing"
    );
}

#[test]
fn wall_clock_and_token_cost_breaches_abort_with_their_reason_code_and_terminal_projections() {
    use std::time::Duration;

    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    // RTL7: every ceiling dimension -- not just max_turns -- aborts with the
    // right reason code AND the coordinated terminal projection set (run/session
    // aborted, agent freed). Drives the two dimensions max_turns does not cover.
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("ceiling-dims-session")),
    )
    .expect("open controller");

    for (agent_name, turn_label, breach, expected_code) in [
        (
            "wall-clock-worker",
            "turn-wall-clock",
            CeilingBreach::WallClock {
                limit: Duration::from_secs(30),
                observed: Duration::from_secs(31),
            },
            "max_wall_clock_exceeded",
        ),
        (
            "token-cost-worker",
            "turn-token-cost",
            CeilingBreach::TokenCost {
                limit: 1_000,
                observed: 1_500,
            },
            "max_token_cost_exceeded",
        ),
    ] {
        let registration = controller.register_agent(agent_name).expect("agent");
        let refs = controller
            .send_task(&registration, "Run under a resource ceiling")
            .expect("send task");
        let turn = TurnId::new(turn_label);

        controller
            .abort_run_for_ceiling(&refs, &turn, breach)
            .expect("abort");

        // The run.aborted event carries the dimension's reason code, keyed to the
        // aborting turn.
        let events = controller
            .state()
            .recent_events_for_session(&refs.session_id, 64)
            .expect("events");
        let aborted = events
            .iter()
            .find(|event| event.kind == "run.aborted")
            .expect("run.aborted recorded");
        assert_eq!(aborted.turn_id.as_deref(), Some(turn_label));
        assert!(
            aborted.payload_json.contains(expected_code),
            "expected reason code {expected_code} in {}",
            aborted.payload_json
        );

        // The coordinated terminal projection set: run + session aborted, the
        // agent freed (available, no current session) -- the same shape every
        // other terminal stop leaves behind.
        assert_eq!(
            controller
                .state()
                .run(&refs.run_id)
                .expect("run")
                .expect("present")
                .status,
            "aborted"
        );
        assert_eq!(
            controller
                .state()
                .session(&refs.session_id)
                .expect("session")
                .expect("present")
                .status,
            "aborted"
        );
        let agent = controller
            .state()
            .agent(&refs.agent_id)
            .expect("agent")
            .expect("present");
        assert_eq!(agent.status, "available");
        assert!(agent.current_session_id.is_none());
    }
}

// --- ACI1: real tool dispatch wired into the loop -------------------------

/// ACI1: the real controller is constructed with the REAL Capo registry and
/// real runtime wrappers, never the test-only fake exposure.
#[test]
fn real_controller_tool_exposures_are_real_not_the_fake_default() {
    use capo_tools::RuntimeToolConfig;

    let controller =
        RealBoundaryController::open(ProjectId::new("project-capo"), temp_root()).expect("open");
    // The Capo registry is live by construction.
    assert!(controller.capo_tools_are_real());
    assert!(controller.capo_registry().is_some());
    // Runtime wrappers are not the fake either, once wired with a workspace.
    assert!(!controller.runtime_tools_are_real());
    let controller =
        controller.with_runtime_tools(RuntimeToolConfig::local_workspace(temp_root(), temp_root()));
    assert!(controller.runtime_tools_are_real());
}

/// ACI1: a real loop turn invoking a Capo-governed tool flows through
/// `authorize_and_invoke` (the real registry, not the fake summary shim) and
/// persists the canonical observed tool-result event sequence keyed to the turn.
#[test]
fn real_controller_turn_invokes_a_capo_tool_through_authorize_and_invoke() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest, ToolExposureResult};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-capo-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("aci1-worker").expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Inspect agent status through a real tool call",
        )
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci1-tool"),
        tool_call_id: ToolCallId::new("tool-aci1-agent-status"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: "capo.agent_status".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: CapoToolContext {
                    task_status: "task active".to_string(),
                    agent_status: "agent running".to_string(),
                    session_summary: "summary".to_string(),
                    workpad_excerpt: "section".to_string(),
                    evidence_note: "note".to_string(),
                    capability_scope: "state:read:agent".to_string(),
                },
            }),
        )
        .expect("dispatch capo tool");

    // The dispatch produced a real Capo result (allowed), not a fake observation.
    let ToolExposureResult::Capo(result) = &outcome.result else {
        panic!("expected a real Capo result");
    };
    assert_eq!(result.permission_decision.effect, "allow");
    assert_eq!(result.output, "agent running");
    assert_eq!(outcome.status, "completed");

    // The canonical real audit event sequence was persisted (in order).
    assert_eq!(
        outcome.observed_event_kinds,
        vec![
            "tool.call_requested",
            "permission.requested",
            "permission.decided",
            "capability.grant_used",
            "tool.invocation_started",
            "tool.output_artifact_recorded",
            "tool.output_observed",
            "tool.call_completed",
            "tool.result_delivered",
        ]
    );

    // The observed tool-result event and the completed projection are persisted,
    // keyed to this turn.
    let events = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-aci1-tool")
        .expect("turn events");
    assert!(events.iter().any(|event| {
        event.kind == "tool.output_observed" && event.payload_json.contains("capo.agent_status")
    }));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "tool.call_completed")
    );
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_call_id == scope.tool_call_id
            && tool.tool_name == "capo.agent_status"
            && tool.tool_origin == "capo"
            && tool.status == "completed"
            && tool.turn_id.as_deref() == Some("turn-aci1-tool")
    }));
}

/// ACI7: a real dispatched tool call persists queryable per-call provenance
/// (correlation_id, permission_decision_id, capability_grant_use_id) and
/// wall-clock timing (started_at/completed_at) on the `ToolCall` projection, and
/// the same provenance rebuilds identically on replay.
#[test]
fn real_controller_dispatch_persists_provenance_and_timing_that_replays_identically() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci7-prov-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("aci7-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Record per-call provenance for a tool call")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci7-prov"),
        tool_call_id: ToolCallId::new("tool-aci7-prov"),
    };
    controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: "capo.agent_status".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: CapoToolContext {
                    task_status: "task active".to_string(),
                    agent_status: "agent running".to_string(),
                    session_summary: "summary".to_string(),
                    workpad_excerpt: "section".to_string(),
                    evidence_note: "note".to_string(),
                    capability_scope: "state:read:agent".to_string(),
                },
            }),
        )
        .expect("dispatch capo tool");

    let read_provenance = || {
        controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tool calls")
            .into_iter()
            .find(|tool| tool.tool_call_id == scope.tool_call_id)
            .expect("dispatched tool call")
            .provenance
    };

    let before = read_provenance();
    // The correlation_id ties command -> turn -> tool (it carries the turn and
    // tool_call_id, the shared join key stamped on every event of the call).
    let correlation_id = before.correlation_id.clone().expect("correlation_id");
    assert!(correlation_id.contains("turn-aci7-prov"));
    assert!(correlation_id.contains(scope.tool_call_id.as_str()));
    // The permission-decision and capability-grant-use ids are pinned per call.
    assert!(
        before
            .permission_decision_id
            .as_deref()
            .is_some_and(|id| id.starts_with("decision-"))
    );
    assert!(
        before
            .capability_grant_use_id
            .as_deref()
            .is_some_and(|id| id.contains(scope.tool_call_id.as_str()))
    );
    // Wall-clock timing is captured around the invocation.
    let started = before.started_at.expect("started_at");
    let completed = before.completed_at.expect("completed_at");
    assert!(started > 0 && completed >= started);

    // A restart/replay rebuilds the IDENTICAL provenance and timing.
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    assert_eq!(
        read_provenance(),
        before,
        "provenance must replay identically"
    );
}

/// ACI8: a dispatched GO2 agent report is persisted as a DISTINCT
/// `tool.observation_recorded` projection tagged `source=agent_reported`
/// (carrying confidence), separate from observed runtime/adapter evidence, and
/// the same classification rebuilds identically on replay -- so completion is
/// never reachable by agent assertion alone.
#[test]
fn real_controller_dispatches_an_agent_report_persisted_as_agent_reported() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{AgentReportRequest, ToolExposureRequest, ToolExposureResult};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci8-report-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("aci8-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Report intent through a GO2 reporting tool")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci8-report"),
        tool_call_id: ToolCallId::new("tool-aci8-report-intent"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::AgentReport(AgentReportRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: "capo.complete_requirement".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                confidence: 80,
                body: serde_json::json!({"requirement_id": "REQ-1", "summary": "done"}),
                submission_id: Some("sub-aci8".to_string()),
            }),
        )
        .expect("dispatch agent report");

    let ToolExposureResult::AgentReport(record) = &outcome.result else {
        panic!("expected an agent-report result");
    };
    assert_eq!(record.source, "agent_reported");
    assert!(record.accepted);
    assert_eq!(outcome.status, "completed");

    // The distinct observation class is persisted: a `tool.observation_recorded`
    // row tagged `source=agent_reported`, carrying confidence, NOT a
    // `tool.output_observed` runtime-evidence event.
    let events = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-aci8-report")
        .expect("turn events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded"
                && event.payload_json.contains("agent_reported")),
        "report must persist a `tool.observation_recorded` event tagged agent_reported"
    );
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "tool.output_observed"),
        "an agent report must NOT persist a runtime `tool.output_observed` event"
    );

    let read_observation = || {
        controller
            .state()
            .tool_observations_for_session(&refs.session_id)
            .expect("tool observations")
            .into_iter()
            .find(|observation| observation.tool_call_id.as_ref() == Some(&scope.tool_call_id))
            .expect("agent report observation")
    };
    let before = read_observation();
    assert_eq!(
        before.source, "agent_reported",
        "the persisted observation must be classified agent_reported, not observed evidence"
    );
    assert_eq!(
        before.confidence, "80",
        "the report's confidence is carried"
    );
    assert_eq!(before.tool_name, "capo.complete_requirement");

    // A restart/replay rebuilds the IDENTICAL agent_reported classification.
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    assert_eq!(
        read_observation(),
        before,
        "the agent_reported observation must replay identically"
    );
}

/// ACI1: the runtime-wrapper path is equally real -- a `capo.file_read` turn
/// flows through `RuntimeToolWrappers::authorize_and_invoke`, reads the
/// workspace file, and records the output artifact.
#[test]
fn real_controller_turn_invokes_a_runtime_wrapper_through_authorize_and_invoke() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join("status.md"), "real read through the loop").expect("seed file");

    let scripted =
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-runtime-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(workspace, artifacts));
    let registration = controller
        .register_agent("aci1-runtime-worker")
        .expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Read a workspace file through a real tool call",
        )
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci1-runtime"),
        tool_call_id: ToolCallId::new("tool-aci1-file-read"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.file_read".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({"path": "status.md"}),
            }),
        )
        .expect("dispatch runtime tool");

    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a real runtime-wrapper result");
    };
    assert_eq!(result.status, "completed");
    assert_eq!(result.output_artifacts.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&result.output_artifacts[0].uri).expect("artifact"),
        "real read through the loop"
    );
    assert_eq!(outcome.tool_origin, "runtime");

    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(tools.iter().any(|tool| {
        tool.tool_call_id == scope.tool_call_id
            && tool.tool_name == "capo.file_read"
            && tool.status == "completed"
            && tool.turn_id.as_deref() == Some("turn-aci1-runtime")
    }));
}

/// ACI1 deny path: a denied Capo dispatch (read-only policy + a write tool)
/// returns `outcome.status == "denied"` AND drives the persisted projection to
/// "denied" -- it must NOT stick at "requested" (the bug: the deny audit kind
/// `tool.call_canceled` has no loop EventKind, so the projection was never
/// advanced past the initial "requested" write).
#[test]
fn real_controller_denied_capo_dispatch_persists_denied_projection() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        CapoToolContext, CapoToolRequest, PermissionPolicy, ToolExposureRequest, ToolExposureResult,
    };

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-deny-session"));
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        PermissionPolicy::static_read_only_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("aci1-deny-worker")
        .expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Attempt a write tool under a read-only policy",
        )
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci1-deny"),
        tool_call_id: ToolCallId::new("tool-aci1-evidence-record"),
    };
    // capo.evidence_record is a write/mutating tool; the read-only policy denies it.
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: "capo.evidence_record".to_string(),
                capability_profile_id: "static-read-only-local".to_string(),
                context: CapoToolContext {
                    task_status: "task active".to_string(),
                    agent_status: "agent running".to_string(),
                    session_summary: "summary".to_string(),
                    workpad_excerpt: "section".to_string(),
                    evidence_note: "note".to_string(),
                    capability_scope: "tool:invoke:capo.evidence_record".to_string(),
                },
            }),
        )
        .expect("dispatch denied capo tool");

    let ToolExposureResult::Capo(result) = &outcome.result else {
        panic!("expected a real Capo result");
    };
    assert_ne!(result.permission_decision.effect, "allow");
    assert_eq!(outcome.status, "denied");
    // No tool.call_completed event is persisted on the deny path.
    assert!(
        !outcome
            .observed_event_kinds
            .contains(&"tool.call_completed".to_string())
    );

    // The persisted projection reaches the TERMINAL denied status, not "requested",
    // and records no output artifact.
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let projection = tools
        .iter()
        .find(|tool| tool.tool_call_id == scope.tool_call_id)
        .expect("denied projection present");
    assert_eq!(projection.status, "denied");
    assert_eq!(projection.output_artifact_id, None);
    assert_eq!(projection.turn_id.as_deref(), Some("turn-aci1-deny"));
}

/// ACI1 failure path: a runtime dispatch that fails at execution (a
/// `capo.file_read` on a missing file under an allow policy) returns
/// `outcome.status == "failed"` and drives the persisted projection to "failed"
/// -- the failure audit kind `tool.call_failed` has no loop EventKind, so before
/// the fix the projection stuck at "requested".
#[test]
fn real_controller_failed_runtime_dispatch_persists_failed_projection() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-fail-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(workspace, artifacts));
    let registration = controller
        .register_agent("aci1-fail-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Read a workspace file that does not exist")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci1-fail"),
        tool_call_id: ToolCallId::new("tool-aci1-missing-read"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.file_read".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({"path": "does-not-exist.md"}),
            }),
        )
        .expect("dispatch failing runtime tool");

    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a real runtime-wrapper result");
    };
    assert_eq!(result.status, "failed");
    assert_eq!(outcome.status, "failed");
    // The terminal failure audit event was observed, and no completed event was.
    assert!(
        outcome
            .observed_event_kinds
            .contains(&"tool.output_observed".to_string())
    );
    assert!(
        !outcome
            .observed_event_kinds
            .contains(&"tool.call_completed".to_string())
    );

    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let projection = tools
        .iter()
        .find(|tool| tool.tool_call_id == scope.tool_call_id)
        .expect("failed projection present");
    assert_eq!(projection.status, "failed");
    assert_eq!(projection.output_artifact_id, None);
    assert_eq!(projection.turn_id.as_deref(), Some("turn-aci1-fail"));
}

/// ACI4 no-match path: an `capo.apply_patch` whose hunk no strategy can locate
/// returns the wrapper's finer-grained `no_match` status on the typed result,
/// but the controller MUST fold that onto the shared dispatch vocabulary so the
/// persisted projection is canonical (`failed`) -- a `no_match` must never
/// escape to downstream consumers (dashboards, safety-gates score_run,
/// goal-autonomy evidence) that only recognize `completed`/`failed`/`denied`.
/// Mirrors the `precondition_failed` fold the file_write guard relies on.
#[test]
fn real_controller_apply_patch_no_match_folds_onto_shared_failed_status() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    // Seed a file whose content the hunk's `search` block will not match.
    std::fs::write(workspace.join("lib.rs"), "fn one() {}\nfn two() {}\n").expect("seed file");

    let scripted =
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci4-no-match-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(workspace, artifacts));
    let registration = controller
        .register_agent("aci4-no-match-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Apply a patch whose hunk does not match")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci4-no-match"),
        tool_call_id: ToolCallId::new("tool-aci4-apply-patch-miss"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.apply_patch".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "path": "lib.rs",
                    "hunks": [{
                        "search": "completely\nunrelated\nblock\n",
                        "replace": "x\n"
                    }]
                }),
            }),
        )
        .expect("dispatch no-match apply_patch");

    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a real runtime-wrapper result");
    };
    // The wrapper carries the FINER no_match detail for the loop to reflect on.
    assert_eq!(result.status, "no_match");
    assert_eq!(result.typed_output["status"], "no_match");
    assert_eq!(result.typed_output["rejected_hunk_index"], 0);
    // No write/artifact, so it is NOT audited as a completed call.
    assert!(result.output_artifacts.is_empty());
    assert!(
        !outcome
            .observed_event_kinds
            .contains(&"tool.call_completed".to_string())
    );

    // But the persisted DISPATCH status is folded onto the shared vocabulary.
    assert_eq!(outcome.status, "failed");

    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let projection = tools
        .iter()
        .find(|tool| tool.tool_call_id == scope.tool_call_id)
        .expect("no-match projection present");
    // The terminal projection status is canonical, never the raw `no_match`.
    assert_eq!(projection.status, "failed");
    assert_eq!(projection.output_artifact_id, None);
    assert_eq!(projection.turn_id.as_deref(), Some("turn-aci4-no-match"));
}

/// ACI1 replay identity: one dispatched tool call must reconstruct to exactly
/// ONE observed tool ref, not three. The tool.* events of a single call share a
/// stamped item_id (the tool_call_id) so `reconstruct_turn_finished`'s dedup
/// collapses tool.call_requested/invocation_started/call_completed into a single
/// ref -- matching the loop's documented replay-identity invariant.
#[test]
fn real_controller_dispatched_tool_call_reconstructs_as_single_observed_ref() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-replay-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        temp_root(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("aci1-replay-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Inspect agent status once for replay")
        .expect("send task");

    let turn_id = TurnId::new("turn-aci1-replay");
    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: ToolCallId::new("tool-aci1-replay-status"),
    };
    controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: "capo.agent_status".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: CapoToolContext {
                    task_status: "task active".to_string(),
                    agent_status: "agent running".to_string(),
                    session_summary: "summary".to_string(),
                    workpad_excerpt: "section".to_string(),
                    evidence_note: "note".to_string(),
                    capability_scope: "state:read:agent".to_string(),
                },
            }),
        )
        .expect("dispatch capo tool");

    let finished = controller
        .core()
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct turn");
    // One real tool call -> exactly one observed tool ref (not three distinct
    // payload strings from tool.call_requested/invocation_started/call_completed).
    assert_eq!(
        finished.observed_tool_refs.len(),
        1,
        "expected a single observed tool ref per dispatched tool call, got {:?}",
        finished.observed_tool_refs
    );
    assert_eq!(
        finished.observed_tool_refs[0],
        scope.tool_call_id.to_string()
    );
}

/// Tiny test-only adapter over the two coexisting controllers so the parity
/// test can drive an identical scripted sequence on each without duplicating
/// the body. Both arms call the SAME public method names; the point of the test
/// is that the resulting persisted state is identical.
// Both controllers are large handles; box both arms so the enum stays small and
// balanced. ACI1 gave `RealBoundaryController` its own real tool exposures, so
// it grew past the fake handle and tripped `large_enum_variant`; boxing both
// keeps the lint happy (and mirrors the server's `ControllerRoute::Real`).
enum SqliteStateStoreBundle {
    Fake(Box<FakeBoundaryController>),
    Real(Box<RealBoundaryController>),
}

impl SqliteStateStoreBundle {
    fn register(&self, agent_name: &str) -> FakeAgentRegistration {
        match self {
            Self::Fake(c) => c.register_agent(agent_name).expect("register agent"),
            Self::Real(c) => c.register_agent(agent_name).expect("register agent"),
        }
    }

    fn send_task(&self, registration: &FakeAgentRegistration, goal: &str) -> FakeRunRefs {
        match self {
            Self::Fake(c) => c.send_task(registration, goal).expect("send task"),
            Self::Real(c) => c.send_task(registration, goal).expect("send task"),
        }
    }

    fn redirect(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        goal: &str,
    ) -> FakeReadModelObservation {
        match self {
            Self::Fake(c) => c.redirect(registration, refs, goal).expect("redirect"),
            Self::Real(c) => c.redirect(registration, refs, goal).expect("redirect"),
        }
    }

    fn interrupt(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> FakeReadModelObservation {
        match self {
            Self::Fake(c) => c.interrupt(registration, refs, reason).expect("interrupt"),
            Self::Real(c) => c.interrupt(registration, refs, reason).expect("interrupt"),
        }
    }

    fn stop(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> FakeReadModelObservation {
        match self {
            Self::Fake(c) => c.stop(registration, refs, reason).expect("stop"),
            Self::Real(c) => c.stop(registration, refs, reason).expect("stop"),
        }
    }

    fn run_turn(
        &self,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        batch: &[capo_adapters::NormalizedAdapterEvent],
    ) -> TurnFinished {
        match self {
            Self::Fake(c) => c.run_turn(refs, turn_id, batch).expect("run turn"),
            Self::Real(c) => c.run_turn(refs, turn_id, batch).expect("run turn"),
        }
    }

    fn state(&self) -> &SqliteStateStore {
        match self {
            Self::Fake(c) => c.state(),
            Self::Real(c) => c.state(),
        }
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
