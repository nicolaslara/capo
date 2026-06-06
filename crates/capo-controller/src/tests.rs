use std::time::{SystemTime, UNIX_EPOCH};

use capo_adapters::{
    AcpPermissionOption, AcpPermissionOptionKind, AcpPermissionOutcome, AdapterPermissionRequest,
};

use super::*;

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
    let root = temp_root();
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &root)
        .expect("open controller");
    controller.register_agent("worker").expect("register agent");

    let error = controller
        .plan_local_adapter_dispatch(
            "unknown",
            "worker",
            "Do work.",
            temp_root().to_path_buf(),
            temp_root().to_path_buf(),
        )
        .unwrap_err();

    assert!(error.contains("unsupported local adapter dispatch plan"));
}

#[test]
fn fake_boundaries_drive_controller_state_and_interrupt_from_read_models() {
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
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
    // DP5: the packet is now assembled from REAL FTS5-retrieved sources (the
    // goal event, the adapter summary artifact, and the workpad markdown
    // pointer), ranked against the goal, rather than four literal candidates.
    // The goal terms match every live source, so at least one item is included;
    // we assert the packet was built with included items + an explanation rather
    // than pinning brittle exact counts that now depend on FTS ranking.
    assert!(memory_event.payload_json.contains("\"included_count\":3"));
    assert!(memory_event.payload_json.contains("\"excluded_count\":0"));
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
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
    let tmp_state_root = temp_root();
    let store = SqliteStateStore::open(&tmp_state_root).expect("open state");
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
    // SLICE-A LEGIBILITY: the session summary for ASSISTANT prose now carries the
    // conductor/agent's REAL WORDS (capped) so the dashboard/thread readback is
    // legible, rather than a `content_hash=` label.
    assert_eq!(
        observation.session.latest_summary.as_deref(),
        Some("Codex fixture response."),
        "assistant-prose summary must be the legible real words"
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
    // SLICE-A LEGIBILITY: assistant PROSE is now carried inline (legible feed),
    // while TOOL content (`cargo test`) stays redacted to a ref/hash.
    let mut saw_prose = false;
    for event in &events {
        if event.payload_json.contains("Codex fixture response.") {
            saw_prose = true;
        }
        assert!(
            !event.payload_json.contains("cargo test"),
            "tool content must stay redacted (not inlined as prose)"
        );
        assert!(event.redaction_state != "contains_sensitive");
    }
    assert!(
        saw_prose,
        "assistant message PROSE must be carried inline so the feed is legible"
    );
}

#[test]
fn acp_live_transcript_events_flow_through_the_loop_ingestion_route() {
    // DP1: a DRIVEN `AcpLiveAdapter` transcript (off a scripted ACP transport, no
    // live process) feeds its normalized `session/update` events through the SAME
    // `apply_normalized_adapter_events_with_turn` ingestion route every other
    // provider uses -- proving the live ACP events are event-sourced into the read
    // models, not reduced to a summary string at the adapter boundary.
    use capo_adapters::{AcpAdapter, AcpLiveAdapter, ScriptedAcpTransport, ScriptedServerFrame};

    let root = temp_root();
    let controller =
        FakeBoundaryController::open(ProjectId::new("project-capo"), &root).expect("controller");
    let registration = controller
        .register_agent("acp-live-replay")
        .expect("register acp agent");
    let refs = controller
        .send_task(&registration, "Drive a live ACP transcript")
        .expect("send task");

    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            std::path::PathBuf::from("/tmp/capo-acp-ingest-ws"),
            std::path::PathBuf::from("/tmp/capo-acp-ingest-art"),
        ));
    let setup_plan = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &capo_tools::PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-ingest"),
    );
    let adapter = AcpLiveAdapter::new(
        "acp-agent",
        vec!["--stdio".to_string()],
        std::path::PathBuf::from("/tmp/capo-acp-ingest-ws"),
        std::path::PathBuf::from("/tmp/capo-acp-ingest-art"),
        setup_plan,
    );

    let transport = ScriptedAcpTransport::new()
        .on_request(
            "initialize",
            vec![ScriptedServerFrame::Response(serde_json::json!({
                "protocolVersion": 1
            }))],
        )
        .on_request(
            "session/new",
            vec![ScriptedServerFrame::Response(serde_json::json!({
                "sessionId": "acp-ingest-session-1"
            }))],
        )
        .on_request(
            "session/prompt",
            vec![
                ScriptedServerFrame::Update(serde_json::json!({
                    "sessionId": "acp-ingest-session-1",
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tool-ingest-1",
                        "title": "write file",
                        "status": "pending"
                    }
                })),
                ScriptedServerFrame::Update(serde_json::json!({
                    "sessionId": "acp-ingest-session-1",
                    "update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "tool-ingest-1",
                        "status": "completed",
                        "content": { "type": "text", "text": "done" }
                    }
                })),
                ScriptedServerFrame::Response(serde_json::json!({ "stopReason": "end_turn" })),
            ],
        );

    let transcript = adapter.drive(transport, "do the task").expect("drive acp");
    assert!(
        !transcript.events.is_empty(),
        "the driven transcript must carry normalized session/update events"
    );

    // The SAME ingestion route the loop uses for every other provider.
    let report = controller
        .apply_normalized_adapter_events(&refs, &transcript.events)
        .expect("apply acp-live transcript through the loop ingestion route");
    assert!(report.appended_event_count > 0);
    assert!(report.tool_event_count > 0);

    // The tool call observed off the ACP wire is event-sourced into the read model
    // (not discarded into a TurnOutput summary).
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(
        tools
            .iter()
            .any(|tool| tool.status == "completed" && tool.tool_origin == "adapter_native:acp"),
        "the ACP tool call must land as a completed adapter-native read-model row"
    );
}

#[test]
fn acp_live_turn_routes_permission_through_policy_and_ingests_events_via_loop_route() {
    // DP1 review fixes 1 + 3, end-to-end through the SINGLE controller seam:
    //
    // 1. SAFETY: the wire client is NOT the policy authority. `drive_acp_live_turn`
    //    installs the controller's `PermissionPolicy`-backed decider into the wire
    //    client, so an inbound `session/request_permission` is decided by
    //    `decide_adapter_permission` and PERSISTED (`permission.requested` ->
    //    `permission.decided`). A read-only policy DENIES the write scope, so the
    //    agent's offered `allow_once` is OVER-RULED -- the wire client writes back
    //    `cancelled`, never the offered allow option.
    //
    // 3. INGESTION: the per-event batch flows through
    //    `apply_normalized_adapter_events_with_turn`, landing the tool call as a
    //    read-model row -- not collapsed into a `TurnOutput` summary.
    use capo_adapters::{
        AcpAdapter, AcpLiveAdapter, AcpPermissionOutcome, ScriptedAcpTransport,
        ScriptedServerFrame, TurnRequest,
    };

    let root = temp_root();
    // A read-only static policy: it DENIES `filesystem:write:workspace`.
    let controller = FakeBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        &root,
        PermissionPolicy::static_read_only_local(),
    )
    .expect("controller");
    let registration = controller
        .register_agent("acp-live-policy")
        .expect("register acp agent");
    let refs = controller
        .send_task(&registration, "Drive a live ACP turn under a deny policy")
        .expect("send task");

    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            std::path::PathBuf::from("/tmp/capo-acp-policy-ws"),
            std::path::PathBuf::from("/tmp/capo-acp-policy-art"),
        ));
    let setup_plan = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &PermissionPolicy::static_read_only_local(),
        refs.session_id.clone(),
    );
    let adapter = AcpLiveAdapter::new(
        "acp-agent",
        vec!["--stdio".to_string()],
        std::path::PathBuf::from("/tmp/capo-acp-policy-ws"),
        std::path::PathBuf::from("/tmp/capo-acp-policy-art"),
        setup_plan,
    );

    let transport = ScriptedAcpTransport::new()
        .on_request(
            "initialize",
            vec![ScriptedServerFrame::Response(serde_json::json!({
                "protocolVersion": 1
            }))],
        )
        .on_request(
            "session/new",
            vec![ScriptedServerFrame::Response(serde_json::json!({
                "sessionId": "acp-policy-session-1"
            }))],
        )
        .on_request(
            "session/prompt",
            vec![
                ScriptedServerFrame::Update(serde_json::json!({
                    "sessionId": "acp-policy-session-1",
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "tool-policy-1",
                        "title": "write file",
                        "status": "pending"
                    }
                })),
                // The agent asks for permission to write; the policy denies it.
                ScriptedServerFrame::RequestPermission(serde_json::json!({
                    "sessionId": "acp-policy-session-1",
                    "toolCall": { "toolCallId": "tool-policy-1", "kind": "edit" },
                    "options": [
                        { "optionId": "opt-allow", "name": "Allow", "kind": "allow_once" }
                    ]
                })),
                ScriptedServerFrame::Update(serde_json::json!({
                    "sessionId": "acp-policy-session-1",
                    "update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "tool-policy-1",
                        "status": "completed",
                        "content": { "type": "text", "text": "done" }
                    }
                })),
                ScriptedServerFrame::Response(serde_json::json!({ "stopReason": "end_turn" })),
            ],
        );

    let outcome = controller
        .drive_acp_live_turn(
            &refs,
            &adapter,
            transport,
            &TurnRequest {
                turn_id: TurnId::new("turn-acp-policy"),
                agent_name: "acp-worker".to_string(),
                goal: "write a file".to_string(),
            },
            None,
        )
        .expect("drive acp live turn through the controller");

    // SAFETY: the policy DENY over-ruled the agent's offered allow. The wire client
    // wrote back `cancelled`, NOT the offered allow option.
    assert_eq!(outcome.transcript.permission_round_trips.len(), 1);
    let round_trip = &outcome.transcript.permission_round_trips[0];
    assert_eq!(
        round_trip.outcome,
        AcpPermissionOutcome::Cancelled,
        "a policy deny must over-rule the adapter-offered allow on the wire",
    );
    assert_eq!(round_trip.capo_decision, "deny");
    assert!(round_trip.must_not_proceed);
    assert!(
        !round_trip.permission_decision_id.is_empty(),
        "the controller persisted a real decision id (not the fail-closed default)",
    );

    // The lifecycle was PERSISTED through the controller seam.
    let perm_events: Vec<_> = controller
        .state()
        .events_after(0, 10_000)
        .expect("events")
        .into_iter()
        .filter(|event| {
            event.session_id.as_ref() == Some(&refs.session_id)
                && event.kind.starts_with("permission.")
        })
        .collect();
    assert!(
        perm_events.iter().any(|e| e.kind == "permission.requested"
            && e.payload_json.contains("acp-live-perm-turn-acp-policy")),
        "the ACP round-trip permission.requested persisted",
    );
    // The ACP round-trip's decided event (not the unrelated memory-packet decision
    // from task setup): keyed by our `request_ref`.
    let decided = perm_events
        .iter()
        .find(|e| {
            e.kind == "permission.decided"
                && e.payload_json.contains("acp-live-perm-turn-acp-policy")
        })
        .expect("ACP round-trip permission.decided persisted");
    assert!(decided.payload_json.contains("\"decision\":\"reject\""));
    assert!(
        !decided.payload_json.contains("\"option_id\":\"opt-allow\""),
        "the denied allow option must not be persisted as the chosen response",
    );

    // INGESTION: the per-event batch landed in the read models via the loop route.
    assert!(outcome.ingest.appended_event_count > 0);
    assert!(outcome.ingest.tool_event_count > 0);
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert!(
        tools
            .iter()
            .any(|tool| tool.status == "completed" && tool.tool_origin == "adapter_native:acp"),
        "the ACP tool call must land as a completed adapter-native read-model row",
    );
}

#[test]
fn acp_live_adapter_dispatches_through_the_agent_adapter_handle() {
    // DP1: `AcpLiveAdapter` is reachable from the single orchestration seam --
    // `AgentAdapterHandle::acp(..)` -- and reports as a real provider, so the loop
    // can dispatch to it (with the live gate off it fails closed to a blocked
    // turn rather than spawning a process).
    use capo_adapters::{
        AcpAdapter, AcpLiveAdapter, AdapterSessionRequest, AgentAdapter, AgentAdapterHandle,
        TurnRequest,
    };

    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            std::path::PathBuf::from("/tmp/capo-acp-handle-ws"),
            std::path::PathBuf::from("/tmp/capo-acp-handle-art"),
        ));
    let setup_plan = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &capo_tools::PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-handle"),
    );
    let handle = AgentAdapterHandle::acp(AcpLiveAdapter::new(
        "acp-agent",
        vec!["--stdio".to_string()],
        std::path::PathBuf::from("/tmp/capo-acp-handle-ws"),
        std::path::PathBuf::from("/tmp/capo-acp-handle-art"),
        setup_plan,
    ));

    assert!(
        handle.is_real(),
        "the ACP handle is a real provider binding"
    );
    assert_eq!(handle.binding().variant, "acp-live");

    let session = handle.open_session(AdapterSessionRequest {
        session_id: SessionId::new("session-acp-handle"),
        agent_name: "acp-worker".to_string(),
    });
    // Gate off -> fail closed to a blocked turn (no process spawn).
    let output = handle.send_turn(
        &session,
        TurnRequest {
            turn_id: TurnId::new("turn-acp-handle"),
            agent_name: "acp-worker".to_string(),
            goal: "do the task".to_string(),
        },
    );
    assert_eq!(output.status, "blocked");
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
    // SLICE-A LEGIBILITY: the redaction floor is now PROSE-vs-TOOL aware. The
    // assistant's own message prose ("Claude fixture response.") is carried
    // inline so the conductor chat / live feed are legible, but TOOL content
    // (the `cargo test` command and its `tests passed` result) stays redacted to
    // a ref/hash -- credential-shaped tool-payload redaction is unchanged.
    let mut saw_prose = false;
    for event in events {
        if event.payload_json.contains("Claude fixture response.") {
            saw_prose = true;
        }
        assert!(
            !event.payload_json.contains("cargo test"),
            "tool-call INPUT must stay redacted (not inlined as prose)"
        );
        assert!(
            !event.payload_json.contains("tests passed"),
            "tool-call RESULT must stay redacted (not inlined as prose)"
        );
        assert!(event.redaction_state != "contains_sensitive");
    }
    assert!(
        saw_prose,
        "assistant message PROSE must be carried inline so the chat/live feed are \
         legible (acceptance #1/#2)"
    );
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let stop_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let interrupt_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let stop_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let fail_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let int_controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

/// ST5 review fix: the thread read model's `item_text` must be proven against
/// the REAL persisted payload shapes the production append paths emit, not
/// fabricated `{"latest_summary":...}` payloads no path produces.
///
/// This drives the genuine controller append paths -- a scripted adapter turn
/// (the adapter-replay path, whose `session.summary_updated`/`tool.*` events
/// carry `adapter_event_payload_json`: `normalized_kind`/`tool_name`/`status`,
/// NOT prose) and a command-path interrupt (the `session_control` path, whose
/// `session.interrupted` event carries `{reason, adapter_summary}`) -- then
/// reads `session_thread` over the durable log and asserts the rendered item
/// text matches what those real payloads carry.
#[test]
fn session_thread_item_text_matches_real_controller_append_payloads() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("thread-real-session")),
    )
    .expect("open controller");
    let registration = controller.register_agent("loop-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Render a real multi-turn thread")
        .expect("send task");

    // Turn A: a real scripted adapter turn projected through the adapter-replay
    // append path (summary + tool + terminal completion), keyed to turn-real-a.
    let turn_a = TurnId::new("turn-real-a");
    let batch = ScriptedMockTurn::new("turn-real-a")
        .message_completed("msg-1", "state inspected")
        .tool_requested("tool-1", "capo.agent_status")
        .tool_completed("tool-1", "capo.agent_status", "agent is running")
        .turn_completed("done-1")
        .normalized_events(&refs.external_session_ref);
    controller
        .run_turn(&refs, &turn_a, &batch)
        .expect("run real adapter turn");

    // Turn B: a real command-path interrupt projected through the
    // session_control append path, keyed to its own turn.
    let turn_b = TurnId::new("turn-real-b");
    controller
        .interrupt_turn(&registration, &refs, &turn_b, "halting for review")
        .expect("interrupt turn");

    // Read the thread as the server's ReadThread does: a pure projection over
    // the durable, turn-keyed event log.
    let thread = controller
        .state()
        .session_thread(&refs.session_id, 0, 1024)
        .expect("session thread");
    // `send_task` opens the session's own active turn, so the thread carries
    // that plus the two turns this test drives; assert by id rather than count.
    assert!(
        thread.turns.len() >= 2,
        "at least the two real turns this test drove are present"
    );
    let turn_a_view = thread
        .turns
        .iter()
        .find(|turn| turn.turn_id == "turn-real-a")
        .expect("turn-real-a present");
    let turn_b_view = thread
        .turns
        .iter()
        .find(|turn| turn.turn_id == "turn-real-b")
        .expect("turn-real-b present");

    // SLICE-A LEGIBILITY: the adapter-replay `session.summary_updated` payload
    // for ASSISTANT prose now carries the REAL WORDS inline (`content`), so the
    // assistant output item renders the conductor's literal text -- legible chat
    // / live feed -- rather than the `normalized_kind (status)` label. (Tool
    // items below still render their composed `tool_name (status)` label: tool
    // payloads never carry prose.)
    let summary_item = turn_a_view
        .items
        .iter()
        .find(|item| item.kind == capo_state::ThreadItemKind::Output)
        .expect("an Output item from the real summary event");
    assert_eq!(summary_item.event_kind, "session.summary_updated");
    assert_eq!(
        summary_item.text.as_deref(),
        Some("state inspected"),
        "real adapter-replay summary payload renders the legible assistant prose"
    );

    // The tool items render the composed `tool_name (status)` label from the
    // real adapter-replay tool payloads -- the scripted turn projects a request
    // and a completion (plus a recorded observation), each a distinct tool item.
    let tool_labels: Vec<&str> = turn_a_view
        .items
        .iter()
        .filter(|item| item.kind == capo_state::ThreadItemKind::Tool)
        .filter_map(|item| item.text.as_deref())
        .collect();
    assert!(
        tool_labels.contains(&"capo.agent_status (requested)"),
        "real adapter-replay tool request payload renders the composed label, got {tool_labels:?}"
    );
    assert!(
        tool_labels.contains(&"capo.agent_status (completed)"),
        "real adapter-replay tool completion payload renders the composed label, got {tool_labels:?}"
    );

    // The real terminal completion event closes turn A.
    assert_eq!(turn_a_view.status, capo_state::ThreadTurnStatus::Completed);

    // The command-path `session.interrupted` payload is `{reason,
    // adapter_summary}`; `item_text` reads `adapter_summary` (the scripted
    // mock's closing summary), proving the prose path against the real shape.
    assert_eq!(
        turn_b_view.status,
        capo_state::ThreadTurnStatus::Interrupted
    );
    let interrupted_item = turn_b_view
        .items
        .iter()
        .find(|item| item.event_kind == "session.interrupted")
        .expect("a session.interrupted item");
    assert_eq!(
        interrupted_item.text.as_deref(),
        Some("Scripted mock interrupted session: halting for review"),
        "real session.interrupted payload renders adapter_summary"
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
/// the fake handle and the real handle over the SAME scripted adapter.
///
/// RTL5 + AI3: both handles persist through the one `append_event`/projection
/// path, so the LOOP-DRIVEN portion (the `run_turn` ingestion) yields identical
/// projections and an identical `TurnFinished`. What AI3 INTENTIONALLY diverges
/// is the `send_task` per-turn summary tool surface: the fake handle keeps the
/// legacy `ToolExposure::fake()` summary shim (a canned observation), while the
/// real handle dispatches the SAME `capo.session_summary` selection through the
/// REAL `dispatch_tool_call` seam (`authorize_and_invoke`). So this test asserts
/// the loop-driven read models match, AND that the real path's summary tool went
/// through the real seam (observed-evidence row + dispatch provenance) where the
/// fake path did not -- the AI3 "real tool dispatch, not the fake shim" proof.
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
        TurnFinished,
        FakeRunRefs,
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
            finished,
            refs,
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

    // The loop-driven session projection and TurnFinished outcome are identical:
    // both handles ingest the same scripted `run_turn` batch through the one
    // shared loop/projection path -- modulo the `updated_sequence` bookkeeping
    // (the AI3 real send_task dispatch emits one more event than the fake shim,
    // shifting the global sequence the final projection was stamped at; every
    // semantic field is identical).
    let mut real_session = real.0.clone();
    let mut fake_session = fake.0.clone();
    real_session.updated_sequence = 0;
    fake_session.updated_sequence = 0;
    assert_eq!(real_session, fake_session, "session projection diverged");
    assert_eq!(real.4, fake.4, "TurnFinished outcome diverged");

    // AI3 divergence: the per-turn summary tool (`tool-rtl5-worker`) goes through
    // the REAL dispatch seam on the real handle and the FAKE shim on the fake
    // handle. The shared loop tool (`tool-1`, ingested by `run_turn`) is identical
    // on both. So filter out the loop tool and assert the summary tool diverged
    // exactly as designed: the real handle persisted a `runtime_output` observed
    // evidence row + dispatch provenance for the summary call; the fake did not.
    let summary_call_id = real.5.run_id.to_string().replace("run-", "tool-");
    let real_summary_obs: Vec<_> = real
        .2
        .iter()
        .filter(|obs| obs.tool_call_id.as_ref().map(|id| id.as_str()) == Some(&summary_call_id))
        .collect();
    let fake_summary_obs: Vec<_> = fake
        .2
        .iter()
        .filter(|obs| obs.tool_call_id.as_ref().map(|id| id.as_str()) == Some(&summary_call_id))
        .collect();
    assert_eq!(
        real_summary_obs.len(),
        1,
        "the real send_task summary tool must persist one observed-evidence row through the real seam: {real_summary_obs:?}",
    );
    assert_eq!(
        real_summary_obs[0].source, "runtime_output",
        "the real send_task summary observation is observed evidence (the real authorize+invoke), not the fake shim",
    );
    assert!(
        fake_summary_obs.is_empty(),
        "the fake send_task summary tool uses the legacy shim and persists no observed-evidence row: {fake_summary_obs:?}",
    );
    // The real summary `ToolCall` carries dispatch provenance (a correlation id);
    // the fake one carries the default (empty) provenance -- the seam difference.
    let real_summary_call = real
        .1
        .iter()
        .find(|call| call.tool_call_id.as_str() == summary_call_id)
        .expect("real summary tool call");
    assert!(
        real_summary_call.provenance.correlation_id.is_some(),
        "the real send_task summary call carries dispatch provenance",
    );
    let fake_summary_call = fake
        .1
        .iter()
        .find(|call| call.tool_call_id.as_str() == summary_call_id)
        .expect("fake summary tool call");
    assert!(
        fake_summary_call.provenance.correlation_id.is_none(),
        "the fake send_task summary call carries no dispatch provenance (the shim path)",
    );
    // Both reach a completed summary tool call (the lifecycle is equivalent even
    // though the seam differs).
    assert_eq!(real_summary_call.status, "completed");
    assert_eq!(fake_summary_call.status, "completed");
    // Evidence parity: the loop-driven evidence row is identical on both; the
    // send_task evidence row differs only by the dispatch-issued artifact id, so
    // assert both recorded the same NUMBER of evidence rows.
    assert_eq!(real.3.len(), fake.3.len(), "evidence row count diverged",);
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
/// the terminal `(task, agent, session, run)` statuses and the causal SESSION-
/// LIFECYCLE event-kind sequence. Two routings are "equivalent" when their
/// fingerprints match exactly.
///
/// AI3: the event component is scoped to the session-LIFECYCLE markers
/// (`session.*`/`run.*`/`memory.*`/`evidence.*`), not the per-turn tool-dispatch
/// internals. The real handle now dispatches the `send_task` summary tool through
/// the real `authorize_and_invoke` seam (a different `tool.*`/`permission.*`
/// shape than the fake shim -- interleaved `permission.requested`/`decided`), so
/// the raw event count and the tool-dispatch sub-sequence intentionally diverge;
/// the lifecycle the suite gates on does not.
type LifecycleFingerprint = (String, String, String, String, Vec<String>);

/// The causal session event-kind sequence (sequence order), dropping the
/// per-request audit envelope whose idempotency key embeds the command id.
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

/// The session-LIFECYCLE event-kind sequence: the `session.*`/`run.*`/`memory.*`/
/// `evidence.*` markers that define where the lifecycle landed, excluding the
/// per-turn tool-dispatch internals (`tool.*`/`permission.*`/`capability.*`) that
/// AI3 routes through the real seam on the real handle and the legacy shim on the
/// fake handle.
fn session_lifecycle_event_kinds(state: &SqliteStateStore, session_id: &SessionId) -> Vec<String> {
    session_event_kind_sequence(state, session_id)
        .into_iter()
        .filter(|kind| {
            kind.starts_with("session.")
                || kind.starts_with("run.")
                || kind.starts_with("memory.")
                || kind.starts_with("evidence.")
        })
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
        session_lifecycle_event_kinds(state, &refs.session_id),
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

/// RTL12 + AI3 parity-equivalence: for a scripted LOOP turn, the fake and real
/// paths produce equivalent event sequences.
///
/// Both handles drive the SAME scripted multi-event turn through the RTL3 loop
/// (`run_turn`) over the same adapter and session label. The loop ingestion path
/// is shared, so the causal event-kind sequence FOR THE LOOP TURN, the stable
/// session projection, and the `TurnFinished` outcome must match. AI3 changed
/// the SEPARATE `send_task` per-turn summary tool surface (the real handle now
/// dispatches it through the real `authorize_and_invoke` seam, the fake keeps the
/// shim), so this test scopes the event-sequence comparison to the loop turn
/// (`turn-rtl12-equiv-1`) -- the portion that genuinely runs on the one shared
/// loop path -- rather than the whole session (whose send_task prefix now
/// intentionally diverges; the dedicated divergence proof is
/// `real_controller_read_models_match_fake_path_for_identical_scripted_output`).
#[test]
fn fake_and_real_paths_produce_equivalent_event_sequences_for_a_scripted_turn() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    /// The causal event-kind sequence for ONE turn (the shared loop path),
    /// dropping the per-request audit envelope, in sequence order.
    fn turn_event_kind_sequence(
        state: &SqliteStateStore,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Vec<String> {
        let mut events = state
            .events_for_session_turn(session_id, turn_id)
            .expect("turn events");
        events.sort_by_key(|event| event.sequence);
        events
            .into_iter()
            .filter(|event| event.kind != "server.request_handled")
            .map(|event| event.kind)
            .collect()
    }

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
        let kinds =
            turn_event_kind_sequence(bundle.state(), &refs.session_id, "turn-rtl12-equiv-1");
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

    // Equal event sequences for the shared LOOP turn (both runs drive the
    // identical scripted adapter, session label, and turn id through the one
    // shared loop ingestion path).
    assert_eq!(
        real.0, fake.0,
        "fake and real scripted LOOP-turn event sequences diverged"
    );
    // The projected read model and the loop's TurnFinished outcome also match,
    // modulo the `updated_sequence` bookkeeping: the AI3 real send_task dispatch
    // emits one more event than the fake shim, shifting the global sequence the
    // final projection was stamped at; every semantic field is identical.
    let mut real_session = real.1.clone();
    let mut fake_session = fake.1.clone();
    real_session.updated_sequence = 0;
    fake_session.updated_sequence = 0;
    assert_eq!(real_session, fake_session, "session projection diverged");
    assert_eq!(real.2, fake.2, "TurnFinished outcome diverged");
    assert_eq!(real.3.session_id, fake.3.session_id);
    // Sanity: the loop turn actually carries the scripted turn's domain events.
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    // The Capo registry is live by construction.
    assert!(controller.capo_tools_are_real());
    assert!(controller.capo_registry().is_some());
    // Runtime wrappers are not the fake either, once wired with a workspace.
    assert!(!controller.runtime_tools_are_real());
    let controller = controller.with_runtime_tools(RuntimeToolConfig::local_workspace(
        temp_root().to_path_buf(),
        temp_root().to_path_buf(),
    ));
    assert!(controller.runtime_tools_are_real());
}

// --- AI3: the production turn loop invokes the real dispatch seam ----------

/// AI3 verification: a real production `send_task` command turn (the
/// `RealBoundaryController::send_task_command` path the server routes
/// `SendTask` through) invokes its per-turn `capo.session_summary` tool THROUGH
/// the real `authorize_and_invoke` seam -- NOT the `ToolExposure::fake()` /
/// `self.tools.invoke` shim. It drives the PRODUCTION command path (a real
/// `CommandEnvelope`, not a bespoke `dispatch_tool_call` harness) and asserts
/// the persisted tool call carries the canonical observed audit sequence + the
/// `ToolCall`/`ToolObservation` projections the ACI1 dispatch tests assert,
/// keyed to the turn.
#[test]
fn production_send_task_command_dispatches_summary_tool_through_authorize_and_invoke() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_core::{CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin};

    let project_id = ProjectId::new("project-capo");
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("ai3-prod-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        project_id.clone(),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("ai3-prod-worker").expect("agent");

    // Drive the PRODUCTION command path: a real SendTask `CommandEnvelope`, the
    // exact shape the server hands `RealBoundaryController::send_task_command`.
    let command = CommandEnvelope::new(
        CommandId::new("cmd-ai3-send"),
        InputOrigin::Cli,
        "operator",
        project_id,
        CommandTarget::Agent(registration.agent_id.clone()),
        CommandIntent::SendTask,
    )
    .with_text("Inspect the session and summarize it");
    let command = CommandEnvelope {
        structured_args: vec![("agent".to_string(), "ai3-prod-worker".to_string())],
        ..command
    };
    let refs = controller
        .send_task_command(&command)
        .expect("production send_task command");

    // The per-turn summary tool is `capo.session_summary`, keyed to the turn's
    // synthetic refs (`turn-{agent}` / `tool-{agent}`) the send_task path stamps.
    let turn_id = format!("turn-{}", registration.agent_name);
    let summary_tool_call_id = format!("tool-{}", registration.agent_name);

    // The canonical REAL audit sequence (the ACI1 dispatch shape) was persisted
    // for the summary tool, keyed to this turn. SG1 wired the decide step's
    // lifecycle step 5 into this path, so the sequence now carries
    // `capability.grant_created` after `permission.decided` (the decision is
    // recorded, then the grant materialized, before the tool/runtime layer's
    // `capability.grant_used`/invocation proceeds).
    let turn_events = controller
        .state()
        .events_for_session_turn(&refs.session_id, &turn_id)
        .expect("turn events");
    let summary_event_kinds: Vec<String> = {
        let mut events: Vec<_> = turn_events
            .iter()
            .filter(|event| event.item_id.as_deref() == Some(summary_tool_call_id.as_str()))
            .collect();
        events.sort_by_key(|event| event.sequence);
        events.into_iter().map(|event| event.kind.clone()).collect()
    };
    assert_eq!(
        summary_event_kinds,
        vec![
            "tool.call_requested",
            "permission.requested",
            "permission.decided",
            "capability.grant_created",
            "capability.grant_used",
            "tool.invocation_started",
            "tool.output_artifact_recorded",
            "tool.output_observed",
            "tool.call_completed",
            "tool.result_delivered",
        ],
        "the production send_task summary tool must flow through authorize_and_invoke",
    );

    // The `ToolCall` projection is completed, real (`capo` origin), carries
    // dispatch provenance (the fake shim leaves provenance default), and is keyed
    // to the turn.
    let tool_calls = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let summary_call = tool_calls
        .iter()
        .find(|call| call.tool_call_id.as_str() == summary_tool_call_id)
        .expect("summary tool call projection");
    assert_eq!(summary_call.tool_name, "capo.session_summary");
    assert_eq!(summary_call.tool_origin, "capo");
    assert_eq!(summary_call.status, "completed");
    assert_eq!(summary_call.turn_id.as_deref(), Some(turn_id.as_str()));
    assert!(
        summary_call.provenance.correlation_id.is_some(),
        "the real dispatch seam stamps dispatch provenance the fake shim never does",
    );

    // The observed-evidence `ToolObservation` projection (the ACI9 row) is
    // present and tagged `runtime_output` -- the proof the real authorize+invoke
    // ran, not the fake summary shim (which records no observation row).
    let observations = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    let observed = observations
        .iter()
        .find(|obs| obs.tool_call_id.as_ref().map(|id| id.as_str()) == Some(&summary_tool_call_id))
        .expect("observed-evidence row for the dispatched summary tool");
    assert_eq!(observed.source, "runtime_output");
    assert_eq!(observed.tool_name, "capo.session_summary");
    assert_eq!(observed.observed_status, "completed");

    // Fail-closed proof that the fake shim was NOT the tool surface: the
    // controller's tool exposures are real by construction.
    assert!(controller.capo_tools_are_real());
}

/// ACI1: a real loop turn invoking a Capo-governed tool flows through
/// `authorize_and_invoke` (the real registry, not the fake summary shim) and
/// persists the canonical observed tool-result event sequence keyed to the turn.
#[test]
fn real_controller_turn_invokes_a_capo_tool_through_authorize_and_invoke() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest, ToolExposureResult};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-capo-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

    // The canonical real audit event sequence was persisted (in order). SG1's
    // decide step inserts `capability.grant_created` after `permission.decided`
    // (lifecycle step 5) before the tool layer's `capability.grant_used`.
    assert_eq!(
        outcome.observed_event_kinds,
        vec![
            "tool.call_requested",
            "permission.requested",
            "permission.decided",
            "capability.grant_created",
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

    // ACI9: the observed tool result is ALSO normalized into a `ToolObservation`
    // projection tagged `source=runtime_output` (observed evidence), so a query
    // over the observation read model surfaces observed evidence, not only agent
    // reports.
    let observations = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations");
    let observed = observations
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&scope.tool_call_id))
        .expect("observed runtime observation for the dispatched capo tool");
    assert_eq!(observed.source, "runtime_output");
    assert_eq!(observed.tool_name, "capo.agent_status");
    assert_eq!(observed.observed_status, "completed");
    assert_eq!(observed.instrumentation_level, "full");
    assert_eq!(
        observed.artifact_id.as_deref(),
        outcome.output_artifact_id.as_deref(),
        "the observed evidence row carries the output artifact id"
    );
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

/// ACI9: in ONE session, a dispatched OBSERVED tool and a dispatched agent
/// report normalize into the `ToolObservation` projection as TWO DISTINCT
/// classes -- observed evidence tagged `source=runtime_output` vs the
/// `source=agent_reported` claim -- and the distinction survives a
/// restart/replay rebuild. This is the load-bearing ACI9 invariant: observed
/// proof and an agent claim are co-queryable yet never indistinguishable, so
/// completion can never be reached by an agent assertion masquerading as
/// observed evidence.
#[test]
fn real_controller_dispatch_persists_observed_and_reported_distinctly_and_replays() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{AgentReportRequest, CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci9-mixed-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("aci9-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Observe state and report progress")
        .expect("send task");

    // 1) A dispatched OBSERVED Capo tool -> observed runtime evidence.
    let observed_call = ToolCallId::new("tool-aci9-observed");
    let observed_scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci9"),
        tool_call_id: observed_call.clone(),
    };
    controller
        .dispatch_tool_call(
            &observed_scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: observed_call.clone(),
                session_id: refs.session_id.clone(),
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
        .expect("dispatch observed capo tool");

    // 2) A dispatched GO2 agent report -> an `agent_reported` claim.
    let report_call = ToolCallId::new("tool-aci9-reported");
    let report_scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci9"),
        tool_call_id: report_call.clone(),
    };
    controller
        .dispatch_tool_call(
            &report_scope,
            ToolExposureRequest::AgentReport(AgentReportRequest {
                tool_call_id: report_call.clone(),
                session_id: refs.session_id.clone(),
                tool_id: "capo.complete_requirement".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                confidence: 80,
                body: serde_json::json!({"requirement_id": "REQ-1", "summary": "done"}),
                submission_id: Some("sub-aci9".to_string()),
            }),
        )
        .expect("dispatch agent report");

    let read_observations = || {
        controller
            .state()
            .tool_observations_for_session(&refs.session_id)
            .expect("tool observations")
    };
    let before = read_observations();

    let observed = before
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&observed_call))
        .expect("observed runtime observation");
    let reported = before
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&report_call))
        .expect("agent-reported observation");

    // The two are DISTINCT observation classes by source.
    assert_eq!(observed.source, "runtime_output");
    assert_eq!(reported.source, "agent_reported");
    assert_ne!(observed.source, reported.source);
    assert!(
        capo_tools::source_is_observed_evidence(&observed.source),
        "the runtime row is observed evidence"
    );
    assert!(
        !capo_tools::source_is_observed_evidence(&reported.source),
        "the agent report is NOT observed evidence -- a claim never masquerades as proof"
    );
    // The report carries the agent's self-declared confidence; observed evidence
    // does not (it is observed, not self-attested).
    assert_eq!(reported.confidence, "80");
    assert_eq!(observed.confidence, "observed");

    // The observed/reported separation survives a restart/replay rebuild.
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    assert_eq!(
        read_observations(),
        before,
        "observed vs reported classification must replay identically"
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
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

/// ACI7 (event-payload leak guard): redaction is enforced not only on the
/// artifacts on disk but on the PERSISTED EVENT payloads. A dispatched tool whose
/// INPUT and OUTPUT both carry a known secret must reference REDACTED artifacts
/// from its events -- the secret cleartext must NEVER appear inline in any
/// persisted event's `payload_json`.
///
/// `capo.file_write` is the strongest probe: the secret rides in the `content`
/// input (-> input artifact) AND is echoed into the unified-diff output (-> output
/// artifact), so a single dispatch exercises both redaction seams. We dispatch it,
/// then scan EVERY persisted event for the session (not just this turn) for the
/// secret cleartext. Both an operator-named pattern and an UNNAMED
/// credential-shaped token (caught only by the default scan) are planted, so the
/// guard fires whether or not the operator declared the secret. We also rebuild
/// projections and re-scan, so a replay can never reintroduce a leak.
#[test]
fn real_controller_dispatch_never_leaks_a_secret_into_event_payloads() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_runtime::RedactionRule;
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    // A secret the operator named as a redaction pattern AND an unnamed
    // credential-shaped token the default scan must catch on its own.
    let named_secret = "SUPERSECRET-DB-PASSWORD";
    let aws_key = "AKIAIOSFODNN7EXAMPLE";

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    // Seed an existing file so the write produces a non-trivial before->after diff
    // (the OUTPUT artifact) that echoes the new secret-bearing content.
    std::fs::write(workspace.join("config.env"), "name=ok\n").expect("seed file");

    let mut config =
        RuntimeToolConfig::local_workspace(workspace.to_path_buf(), artifacts.to_path_buf());
    config.redaction_rules.push(RedactionRule {
        pattern: named_secret.to_string(),
        replacement: "[REDACTED]".to_string(),
    });

    let scripted =
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci7-leak-scan-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(config);
    let registration = controller
        .register_agent("aci7-leak-scan-worker")
        .expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Write a secret-bearing file through a real tool call",
        )
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-aci7-leak-scan"),
        tool_call_id: ToolCallId::new("tool-aci7-secret-write"),
    };
    // The secret rides in BOTH the input `content` and (via the diff) the output.
    let secret_content = format!("DB_PASSWORD={named_secret}\nAWS_KEY={aws_key}\nname=ok\n");
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.file_write".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "path": "config.env",
                    "content": secret_content,
                }),
            }),
        )
        .expect("dispatch runtime tool");

    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a real runtime-wrapper result");
    };
    assert_eq!(result.status, "completed");
    assert_eq!(outcome.status, "completed");

    // Sanity: the artifacts on disk ARE redacted (the input payload and the diff
    // output), so the secret never reaches durable storage in cleartext. This is
    // the ACI7 artifact contract; the event-payload scan below is the new guard.
    let input_artifact = result.input_artifact.as_ref().expect("input artifact");
    assert_eq!(input_artifact.redaction_state, "redacted");
    let input_on_disk = std::fs::read_to_string(&input_artifact.uri).expect("input artifact");
    assert!(
        !input_on_disk.contains(named_secret) && !input_on_disk.contains(aws_key),
        "secret leaked into the INPUT artifact: {input_on_disk}"
    );
    let output_artifact = result
        .output_artifacts
        .first()
        .expect("output diff artifact");
    assert_eq!(output_artifact.redaction_state, "redacted");
    let output_on_disk = std::fs::read_to_string(&output_artifact.uri).expect("output artifact");
    assert!(
        !output_on_disk.contains(named_secret) && !output_on_disk.contains(aws_key),
        "secret leaked into the OUTPUT artifact: {output_on_disk}"
    );

    // The MAIN assertion: scan EVERY persisted event for this session and assert
    // the secret cleartext appears in NO event's payload. Events must reference
    // the redacted artifacts by id, never inline the raw content.
    let scan_events = || {
        controller
            .state()
            // A limit far above any plausible event count for this session, so the
            // scan covers the WHOLE persisted event store, not a recency window.
            .recent_events_for_session(&refs.session_id, 100_000)
            .expect("session events")
    };
    let assert_no_leak = |events: &[capo_state::EventRecord]| {
        // The dispatch must actually have persisted tool events -- otherwise the
        // scan would be vacuously green.
        assert!(
            events.iter().any(|event| event.kind.starts_with("tool.")),
            "expected persisted tool events for the dispatched call"
        );
        for event in events {
            assert!(
                !event.payload_json.contains(named_secret),
                "named secret leaked into event payload (kind={}, id={}): {}",
                event.kind,
                event.event_id,
                event.payload_json
            );
            assert!(
                !event.payload_json.contains(aws_key),
                "credential-shaped secret leaked into event payload (kind={}, id={}): {}",
                event.kind,
                event.event_id,
                event.payload_json
            );
        }
        // The output-artifact-recorded event must carry the redacted artifact REF,
        // proving the events point AT the redacted artifact rather than inlining
        // the content.
        let artifact_id = outcome
            .output_artifact_id
            .as_deref()
            .expect("output artifact id");
        assert!(
            events.iter().any(|event| {
                event.kind == "tool.output_artifact_recorded"
                    && event.payload_json.contains(artifact_id)
            }),
            "expected an output-artifact event referencing the redacted artifact id"
        );
    };

    assert_no_leak(&scan_events());

    // A restart/replay must not reintroduce a leak: rebuild projections from the
    // event log and re-scan. (The event payloads are immutable, but this pins the
    // invariant against any future replay-side normalization.)
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    assert_no_leak(&scan_events());
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
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

/// FP1 pass path: a PASSING `capo.test_run` (exit 0). The process-runner reports
/// the raw status `exited` for ANY terminated process -- it is not itself a
/// pass/fail discriminator. The dispatch MUST fold `exited` onto the shared
/// vocabulary using the wrapper's own `passed` signal, so a successful run
/// persists a `completed` ToolCall projection (never the raw `exited`, which
/// would make a non-zero exit indistinguishable from success). The raw `exited`
/// detail still survives on the observed-evidence observation row. Deterministic
/// via `/bin/sh`, no live provider.
#[test]
fn real_controller_passing_test_run_persists_completed_projection() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("fp1-pass-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
    let registration = controller.register_agent("fp1-pass-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run a passing test command")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-fp1-pass"),
        tool_call_id: ToolCallId::new("tool-fp1-test-pass"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.test_run".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "program": "/bin/sh",
                    "argv": ["-c", "echo 'mod::ok ... ok'; exit 0"],
                    "cwd": ".",
                }),
            }),
        )
        .expect("dispatch passing test_run");

    // The wrapper carries the raw runner status `exited` and `passed == true`.
    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a runtime test_run result");
    };
    assert_eq!(result.status, "exited");
    assert_eq!(result.typed_output["passed"], serde_json::json!(true));
    // The dispatch folds `exited` + passed -> `completed`.
    assert_eq!(outcome.status, "completed");
    assert!(
        outcome
            .observed_event_kinds
            .contains(&"tool.call_completed".to_string()),
        "a passing run reaches the completed audit event"
    );

    // The persisted ToolCall projection is `completed`, never the raw `exited`.
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let projection = tools
        .iter()
        .find(|tool| tool.tool_call_id == scope.tool_call_id)
        .expect("passing test_run projection present");
    assert_eq!(projection.status, "completed");
    assert_eq!(projection.tool_origin, "runtime");

    // The raw runner detail still survives on the observed-evidence row.
    let observed = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations")
        .into_iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&scope.tool_call_id))
        .expect("observed runtime observation");
    assert_eq!(observed.source, "runtime_output");
    assert_eq!(
        observed.observed_status, "exited",
        "the raw runner status is preserved on observed_evidence.observed_status"
    );
}

/// FP1 fail path: a FAILING `capo.shell_run` (non-zero exit). The runner maps a
/// non-zero exit to the raw status `failed` (and `passed == false`), which the
/// dispatch keeps in the shared vocabulary as `failed` -- distinct from the
/// passing run's `completed` above, so the pass/fail discriminator the
/// safety-gates `score_run` will consume is never dropped. Together with the
/// passing test it pins both ends of the `exited` fold: a passing run does NOT
/// collapse to the same bucket as a non-zero exit. Deterministic via `/bin/sh`,
/// no live provider.
#[test]
fn real_controller_failing_shell_run_persists_failed_projection() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("fp1-fail-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
    let registration = controller.register_agent("fp1-fail-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Run a failing shell command")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-fp1-fail"),
        tool_call_id: ToolCallId::new("tool-fp1-shell-fail"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                run_id: scope.run_id.clone(),
                tool_id: "capo.shell_run".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "program": "/bin/sh",
                    "argv": ["-c", "echo boom; exit 3"],
                    "cwd": ".",
                }),
            }),
        )
        .expect("dispatch failing shell_run");

    // A non-zero exit: the runner maps the raw status to `failed` (distinct from
    // the `exited` it reports for a success), and `passed == false`.
    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a runtime shell_run result");
    };
    assert_eq!(result.status, "failed");
    assert_eq!(result.typed_output["passed"], serde_json::json!(false));
    assert_eq!(result.typed_output["exit_status"], serde_json::json!(3));
    // The dispatch keeps the failure in the shared vocabulary as `failed`, NOT
    // `completed` -- the pass/fail discriminator survives.
    assert_eq!(outcome.status, "failed");

    // The persisted ToolCall projection is `failed`, distinguishable from the
    // passing-run `completed` above.
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    let projection = tools
        .iter()
        .find(|tool| tool.tool_call_id == scope.tool_call_id)
        .expect("failing shell_run projection present");
    assert_eq!(projection.status, "failed");

    // The raw runner detail still survives on the observed-evidence row.
    let observed = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations")
        .into_iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&scope.tool_call_id))
        .expect("observed runtime observation");
    assert_eq!(observed.source, "runtime_output");
    assert_eq!(
        observed.observed_status, "failed",
        "the raw runner status is preserved on observed_evidence.observed_status"
    );
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
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
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

/// ACI1 replay identity across a true restart: the single-observed-ref invariant
/// must survive reopening the store from disk. The live test above derives the
/// outcome from the controller that did the dispatch; this one dispatches ONE
/// tool call, then reconstructs `TurnFinished` from a FRESH controller opened
/// over the same on-disk state root (nothing in-memory carries over), and
/// asserts the dispatched call still collapses to EXACTLY ONE observed tool ref.
/// This pins the regression the FIXREPLAY remediation targets: the tool.* events
/// of one call (`tool.call_requested` / `tool.invocation_started` /
/// `tool.call_completed`) share the stamped item_id (the tool_call_id), so the
/// `reconstruct_turn_finished` dedup keyed on `persisted_turn_ref` reads them
/// back from the log as one ref, not three -- even after a restart.
#[test]
fn real_controller_dispatched_tool_call_reconstructs_single_observed_ref_after_restart() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let state_root = temp_root();
    let scripted =
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-replay-restart-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("aci1-replay-restart-worker")
        .expect("agent");
    let refs = controller
        .send_task(
            &registration,
            "Inspect agent status once for restart replay",
        )
        .expect("send task");

    let turn_id = TurnId::new("turn-aci1-replay-restart");
    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: turn_id.clone(),
        tool_call_id: ToolCallId::new("tool-aci1-replay-restart-status"),
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

    // The live reconstruction (the controller that did the dispatch).
    let live = controller
        .core()
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct turn live");
    assert_eq!(
        live.observed_tool_refs.len(),
        1,
        "live reconstruction must already be a single observed ref, got {:?}",
        live.observed_tool_refs
    );

    // RESTART: a fresh controller opened over the same on-disk state root, which
    // never saw the in-memory dispatch. Reconstruction here reads the persisted,
    // turn-keyed event log only -- the genuine replay-identity proof.
    let reopened = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-replay-restart-session")),
    )
    .expect("reopen controller from disk");
    reopened
        .state()
        .rebuild_projections()
        .expect("rebuild projections on restart");
    let replayed = reopened
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct turn after restart");

    // One dispatched tool call -> EXACTLY one observed tool ref after a restart,
    // identical to the live reconstruction (no over-count from the 3 tool.*
    // events of the single call).
    assert_eq!(
        replayed.observed_tool_refs.len(),
        1,
        "expected a single observed tool ref per dispatched tool call after restart, got {:?}",
        replayed.observed_tool_refs
    );
    assert_eq!(
        replayed.observed_tool_refs[0],
        scope.tool_call_id.to_string()
    );
    assert_eq!(
        replayed.observed_tool_refs, live.observed_tool_refs,
        "the single-observed-ref invariant must be identical live and after restart",
    );
}

/// ACI1 replay identity for the `send_task` turn-context tool shim: the synthetic
/// `capo.session_summary` call the memory-packet shim emits per turn must ALSO
/// collapse to exactly ONE observed tool ref. Its tool.* events
/// (`tool.call_requested` / `tool.invocation_started` / `tool.call_completed`)
/// previously carried distinct per-kind payloads and NO shared item_id, so
/// `reconstruct_turn_finished` over-counted one call as three refs (it fell
/// through to the payload_json fallback). Stamping the shared tool_call_id item
/// ref makes the dedup collapse them to one, identical before and after a
/// restart -- the same invariant the real `dispatch_tool_call` path honors.
#[test]
fn send_task_turn_context_tool_reconstructs_as_single_observed_ref() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    let state_root = temp_root();
    let scripted =
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-sendtask-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("aci1-sendtask-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "summarize the turn once")
        .expect("send task");

    // The `send_task` shim keys its turn-context tool call onto `turn-<agent>`.
    let turn_id = TurnId::new("turn-aci1-sendtask-worker");
    let live = controller
        .core()
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct turn live");
    assert_eq!(
        live.observed_tool_refs.len(),
        1,
        "the send_task turn-context tool call must be a single observed ref, got {:?}",
        live.observed_tool_refs
    );
    assert_eq!(live.observed_tool_refs[0], "tool-aci1-sendtask-worker");

    // The invariant survives a true restart: a fresh controller over the same
    // on-disk root reconstructs the identical single ref from the event log.
    let reopened = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci1-sendtask-session")),
    )
    .expect("reopen controller from disk");
    reopened
        .state()
        .rebuild_projections()
        .expect("rebuild projections on restart");
    let replayed = reopened
        .reconstruct_turn_finished(&refs, &turn_id)
        .expect("reconstruct turn after restart");
    assert_eq!(
        replayed.observed_tool_refs, live.observed_tool_refs,
        "the send_task single-observed-ref invariant must be identical after restart",
    );
}

// ---------------------------------------------------------------------------
// ACI11: full tools E2E gate through the real loop + restart/replay identity
// ---------------------------------------------------------------------------

/// ACI11 E2E gate: a single real session drives the whole ACI tool surface
/// through `RealBoundaryController::dispatch_tool_call` -- a `capo.file_read`,
/// a `capo.apply_patch` (with lint-on-edit), and a `capo.test_run` -- and then
/// emits a GO2 `capo.complete_subtask` agent report. The test asserts the four
/// load-bearing ACI11 invariants AT ONCE, with NO live provider:
///
/// 1. Observed evidence (the three runtime-wrapper calls) is persisted as
///    `source=runtime_output` observation rows, distinct from
/// 2. the agent's `agent_reported` completion CLAIM, so completion is never
///    reachable by agent assertion alone;
/// 3. per-call provenance (correlation/decision/grant ids + wall-clock timing)
///    is queryable on every dispatched tool call; and
/// 4. the entire projection set -- tool calls, observations, and the report --
///    rebuilds byte-identically after a restart (reopen the store, rebuild
///    projections from the log).
#[test]
fn real_controller_full_tools_e2e_persists_observed_and_reported_and_replays_identically() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        AgentReportRequest, RuntimeToolConfig, ToolExposureRequest, ToolExposureResult,
        WrapperToolRequest,
    };

    // A scratch workspace seeded with a file to read and a Rust file to patch.
    let state_root = temp_root();
    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join("notes.txt"), "alpha\nbravo\ncharlie\n").expect("seed read file");
    std::fs::write(workspace.join("edit.rs"), "fn main() {}\n").expect("seed edit file");

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci11-e2e-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts.clone(),
    ));
    let registration = controller
        .register_agent("aci11-e2e-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Read, patch, test, then report completion")
        .expect("send task");

    let turn = TurnId::new("turn-aci11-e2e");
    let scope = |tool_call_id: &str| ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: turn.clone(),
        tool_call_id: ToolCallId::new(tool_call_id),
    };

    // 1) capo.file_read -- observed evidence.
    let read_call = ToolCallId::new("tool-aci11-read");
    let read = controller
        .dispatch_tool_call(
            &scope("tool-aci11-read"),
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: read_call.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                tool_id: "capo.file_read".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({"path": "notes.txt"}),
            }),
        )
        .expect("dispatch file_read");
    assert_eq!(read.status, "completed");

    // 2) capo.apply_patch with lint-on-edit -- observed evidence; the well-formed
    //    Rust edit passes rustfmt --check.
    let patch_call = ToolCallId::new("tool-aci11-patch");
    let patch = controller
        .dispatch_tool_call(
            &scope("tool-aci11-patch"),
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: patch_call.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                tool_id: "capo.apply_patch".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "path": "edit.rs",
                    "hunks": [{
                        "search": "fn main() {}\n",
                        "replace": "fn main() {\n    let _x = 1;\n}\n",
                    }],
                }),
            }),
        )
        .expect("dispatch apply_patch");
    let ToolExposureResult::Runtime(patch_result) = &patch.result else {
        panic!("expected a runtime apply_patch result");
    };
    assert_eq!(
        patch_result.status, "completed",
        "summary: {}",
        patch_result.summary
    );
    assert_eq!(
        patch_result.narrow_output()["lint_status"],
        serde_json::json!("passed"),
        "lint-on-edit must run and pass on the well-formed Rust edit",
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("edit.rs")).expect("edited file"),
        "fn main() {\n    let _x = 1;\n}\n",
    );

    // 3) capo.test_run -- observed evidence; a deterministic /bin/sh fake.
    let test_call = ToolCallId::new("tool-aci11-test");
    let test = controller
        .dispatch_tool_call(
            &scope("tool-aci11-test"),
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: test_call.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                tool_id: "capo.test_run".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "program": "/bin/sh",
                    "argv": ["-c", "echo 'test mod::ok ... ok'; exit 0"],
                    "cwd": ".",
                }),
            }),
        )
        .expect("dispatch test_run");
    let ToolExposureResult::Runtime(test_result) = &test.result else {
        panic!("expected a runtime test_run result");
    };
    assert_eq!(
        test_result.narrow_output()["passed"],
        serde_json::json!(true)
    );
    // FP1: the runner reports `exited` for a passing test_run, but the dispatch
    // outcome folds that onto the shared vocabulary using the wrapper's own
    // `passed` signal -- a passing run is `completed`, never the raw `exited`.
    assert_eq!(test_result.status, "exited");
    assert_eq!(test.status, "completed");

    // 4) GO2 capo.complete_subtask -- an `agent_reported` completion CLAIM,
    //    NOT observed evidence.
    let report_call = ToolCallId::new("tool-aci11-report");
    let report = controller
        .dispatch_tool_call(
            &scope("tool-aci11-report"),
            ToolExposureRequest::AgentReport(AgentReportRequest {
                tool_call_id: report_call.clone(),
                session_id: refs.session_id.clone(),
                tool_id: "capo.complete_subtask".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                confidence: 90,
                body: serde_json::json!({"subtask_id": "ST-1", "summary": "read+patch+test done"}),
                submission_id: Some("sub-aci11".to_string()),
            }),
        )
        .expect("dispatch agent report");
    let ToolExposureResult::AgentReport(record) = &report.result else {
        panic!("expected an agent-report result");
    };
    assert_eq!(record.source, "agent_reported");
    assert!(record.accepted);

    // -- Snapshot the projections before the restart. --
    let read_tools = || {
        controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tool calls")
    };
    let read_observations = || {
        controller
            .state()
            .tool_observations_for_session(&refs.session_id)
            .expect("tool observations")
    };
    let tools_before = read_tools();
    let observations_before = read_observations();
    let event_count_before = controller.state().event_count().expect("event count");

    // The three runtime wrappers persisted as observed evidence (runtime_output);
    // the report persisted as the agent claim (agent_reported). Co-queryable yet
    // a distinct class: completion is never reachable by the claim alone.
    for observed_call in [&read_call, &patch_call, &test_call] {
        let observed = observations_before
            .iter()
            .find(|observation| observation.tool_call_id.as_ref() == Some(observed_call))
            .expect("observed runtime observation");
        assert_eq!(observed.source, "runtime_output");
        assert!(
            capo_tools::source_is_observed_evidence(&observed.source),
            "a runtime wrapper result is observed evidence"
        );
    }
    let reported = observations_before
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&report_call))
        .expect("agent-reported observation");
    assert_eq!(reported.source, "agent_reported");
    assert!(
        !capo_tools::source_is_observed_evidence(&reported.source),
        "an agent report is NOT observed evidence -- a claim never masquerades as proof"
    );
    assert_eq!(reported.confidence, "90");

    // Per-call provenance is queryable on every dispatched tool call.
    for call in [&read_call, &patch_call, &test_call, &report_call] {
        let tool = tools_before
            .iter()
            .find(|tool| &tool.tool_call_id == call)
            .unwrap_or_else(|| panic!("tool call {} present", call.as_str()));
        let provenance = &tool.provenance;
        let correlation = provenance
            .correlation_id
            .as_deref()
            .expect("correlation_id");
        assert!(correlation.contains("turn-aci11-e2e"));
        assert!(correlation.contains(call.as_str()));
        assert!(provenance.started_at.expect("started_at") > 0);
        assert!(
            provenance.completed_at.expect("completed_at")
                >= provenance.started_at.expect("started_at")
        );
    }

    // -- Restart: reopen the store and rebuild projections from the log. --
    let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        reopened
            .tool_calls_for_session(&refs.session_id)
            .expect("tool calls"),
        tools_before,
        "tool-call projections (incl. provenance/timing) must rebuild identically",
    );
    assert_eq!(
        reopened
            .tool_observations_for_session(&refs.session_id)
            .expect("tool observations"),
        observations_before,
        "observed-vs-reported observation projections must rebuild identically",
    );
    assert_eq!(
        reopened.event_count().expect("event count"),
        event_count_before,
        "replay introduces no new events",
    );
}

/// ACI11 live smoke (opt-in): one real `capo.shell_run` against a scratch
/// workspace through the SAME `dispatch_tool_call` substrate the deterministic
/// E2E gate uses. It is `#[ignore]` and additionally guarded by an explicit env
/// gate mirroring `CAPO_SERVER_RUN_CODEX_LIVE`, so it never runs for everyone
/// else. It is PAIRED with a deterministic assertion (the persisted observed
/// `runtime_output` observation + completed tool-call projection), so completion
/// is never operator-asserted alone. The always-on deterministic pairing is
/// `real_controller_full_tools_e2e_persists_observed_and_reported_and_replays_identically`.
#[test]
#[ignore = "live tool smoke: set CAPO_TOOLS_RUN_LIVE=1 to run it"]
fn live_shell_run_smoke_is_paired_with_a_deterministic_assertion() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{
        RuntimeToolConfig, ToolExposureRequest, ToolExposureResult, WrapperToolRequest,
    };

    if std::env::var("CAPO_TOOLS_RUN_LIVE").as_deref() != Ok("1") {
        eprintln!("skipping live shell_run smoke: set CAPO_TOOLS_RUN_LIVE=1 to run it");
        return;
    }

    let state_root = temp_root();
    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("aci11-live-session"));
    let controller = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &state_root,
        scripted,
    )
    .expect("open real controller")
    .with_runtime_tools(RuntimeToolConfig::local_workspace(
        workspace.to_path_buf(),
        artifacts.to_path_buf(),
    ));
    let registration = controller
        .register_agent("aci11-live-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Run a real shell command")
        .expect("send task");

    let call = ToolCallId::new("tool-aci11-live-shell");
    let outcome = controller
        .dispatch_tool_call(
            &ToolDispatchScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: TurnId::new("turn-aci11-live"),
                tool_call_id: call.clone(),
            },
            ToolExposureRequest::Runtime(WrapperToolRequest {
                tool_call_id: call.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                tool_id: "capo.shell_run".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                input: serde_json::json!({
                    "program": "/bin/sh",
                    "argv": ["-c", "echo capo-aci11-live"],
                    "cwd": ".",
                }),
            }),
        )
        .expect("dispatch live shell_run");

    // Deterministic pairing: the live run persists observed evidence, NOT an
    // operator attestation. (Secrets in the output are stripped by the wrapper
    // redaction policy before the artifact is written; see the redaction tests.)
    let ToolExposureResult::Runtime(result) = &outcome.result else {
        panic!("expected a runtime shell_run result");
    };
    assert_eq!(result.status, "exited");
    let observed = controller
        .state()
        .tool_observations_for_session(&refs.session_id)
        .expect("tool observations")
        .into_iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&call))
        .expect("observed live shell observation");
    assert_eq!(observed.source, "runtime_output");
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

fn temp_root() -> capo_tmptest::TempRoot {
    capo_tmptest::TempRoot::new("capo-controller")
}

// --- AI2: real-Codex chat backend, binding-respecting + fail-closed-fast ----

/// The codex-live chat gate reads two process-global env vars, so the two tests
/// that toggle them must not race each other (or any other env-touching test).
static CODEX_LIVE_CHAT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Write an executable `codex` STUB pinned by absolute path. The runtime spawns
/// with `env_clear()` (only HOME/PATH/TMPDIR/USER/LOGNAME/SHELL/LANG survive), so
/// the stub uses ONLY POSIX builtins (`read`/`printf`) and reads its fixed JSONL
/// from an absolute-path fixture -- no live provider is involved. Returns the
/// absolute path to the stub program.
#[cfg(unix)]
fn write_codex_stub(dir: &std::path::Path, fixture_jsonl: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).expect("stub dir");
    let fixture = dir.join("codex-output.jsonl");
    std::fs::write(&fixture, fixture_jsonl).expect("write fixture");
    let stub = dir.join("codex-stub.sh");
    // The shebang resolves `/bin/sh` by absolute path (kernel-level), so an
    // empty/clamped PATH does not matter. The body streams the absolute-path
    // fixture to stdout using only the `read` + `printf` shell builtins.
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{}'\n",
        fixture.display()
    );
    std::fs::write(&stub, script).expect("write stub");
    let mut perms = std::fs::metadata(&stub)
        .expect("stub metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub
}

#[cfg(unix)]
#[test]
fn codex_bound_chat_drives_the_real_adapter_through_a_codex_stub_with_gate_open() {
    use capo_adapters::{AgentAdapterHandle, CodexLiveAdapter};

    let _guard = CODEX_LIVE_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // The deterministic `codex` stub emits a fixed JSONL turn (an agent_message
    // item + a turn.completed), pinned by absolute path. No live provider runs.
    let stub_dir = temp_root();
    let fixture = "{\"type\":\"thread.started\",\"thread_id\":\"codex-stub-thread\"}\n\
{\"type\":\"item.completed\",\"item\":{\"id\":\"item-1\",\"type\":\"agent_message\",\"text\":\"CODEX_STUB_CHAT_SUMMARY\"}}\n\
{\"type\":\"turn.completed\"}\n";
    let stub = write_codex_stub(&stub_dir, fixture);

    let workspace = temp_root();
    let artifacts = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");

    // Bind the agent to a REAL Codex chat handle (not fake/scripted), pinned to
    // the stub program and a short bounded timeout.
    let codex_handle = AgentAdapterHandle::codex(
        CodexLiveAdapter::new(workspace.clone(), artifacts.clone())
            .with_codex_program_override(stub.display().to_string())
            .with_timeout_seconds(30),
    );
    assert!(
        codex_handle.is_real(),
        "codex handle must be a real binding"
    );

    // Gate OPEN: both live-provider opt-ins set.
    // SAFETY: serialized by `CODEX_LIVE_CHAT_ENV_LOCK`.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_CODEX_LIVE", "1");
    }

    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        codex_handle,
    )
    .expect("open controller with codex handle");
    let registration = controller
        .register_agent("codex-chat-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Summarize the current workpad")
        .expect("codex-bound chat send_task succeeds with the gate open");

    // SAFETY: serialized.
    unsafe {
        std::env::remove_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT");
        std::env::remove_var("CAPO_SERVER_RUN_CODEX_LIVE");
    }

    // The chat turn produced the STUB's output through the `AgentAdapter` trait:
    // the session summary is the parsed Codex `agent_message` text from the stub,
    // proving the real adapter ran (the open-session ref is the adapter's session
    // template; the turn's observed summary is the load-bearing assertion).
    let observation = controller.observe(&refs).expect("observe");
    assert_eq!(
        observation.session.latest_summary.as_deref(),
        Some("CODEX_STUB_CHAT_SUMMARY"),
        "chat summary must be the real stub output, not a fake-adapter summary"
    );
    assert_ne!(
        observation.session.latest_summary.as_deref(),
        Some("Fake adapter processed goal for codex-chat-worker: Summarize the current workpad")
    );
    // The Codex chat adapter's open-session ref names the real binding, not the
    // fake-adapter template -- so chat for a Codex-bound agent is NOT the fake
    // adapter masquerading.
    assert_eq!(
        refs.external_session_ref,
        "codex-live-chat-session-codex-chat-worker"
    );
    assert_ne!(
        refs.external_session_ref,
        "fake-adapter-session-codex-chat-worker"
    );
}

#[test]
fn codex_bound_chat_fails_closed_fast_when_gate_is_off() {
    use capo_adapters::{AgentAdapterHandle, CodexLiveAdapter};

    let _guard = CODEX_LIVE_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Gate OFF: ensure neither opt-in is set.
    // SAFETY: serialized by `CODEX_LIVE_CHAT_ENV_LOCK`.
    unsafe {
        std::env::remove_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT");
        std::env::remove_var("CAPO_SERVER_RUN_CODEX_LIVE");
    }

    // A real Codex handle pointed at a NON-EXISTENT absolute program: the
    // fail-closed-fast path must never spawn it, so the bogus path is never run.
    let codex_handle = AgentAdapterHandle::codex(
        CodexLiveAdapter::new(temp_root().to_path_buf(), temp_root().to_path_buf())
            .with_codex_program_override("/nonexistent/codex-should-never-spawn".to_string())
            .with_timeout_seconds(1),
    );
    let tmp_state_root = temp_root();
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        codex_handle,
    )
    .expect("open controller with codex handle");
    let registration = controller
        .register_agent("codex-chat-worker")
        .expect("agent");

    // The chat turn returns an IMMEDIATE typed error (not a hang, not a fake
    // summary). A wall-clock budget proves "fast": the fail-closed decision
    // happens before any spawn/wait, so this returns well under a second.
    let started = std::time::Instant::now();
    let error = controller
        .send_task(&registration, "Summarize the current workpad")
        .expect_err("codex-bound chat must fail closed when the gate is off");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "fail-closed chat must return fast, not block: took {:?}",
        started.elapsed()
    );
    match error {
        StateError::CodexLiveChat(detail) => {
            assert!(
                detail.contains("fail-closed")
                    && detail.contains("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT"),
                "typed fail-closed error should name the missing opt-in: {detail}"
            );
        }
        other => panic!("expected StateError::CodexLiveChat, got {other:?}"),
    }
}

// --- SG1: PermissionPolicy enforcement wired into the real decide step -----
//
// The real loop's tool-dispatch decide step
// (`RealBoundaryController::dispatch_tool_call`) runs `PermissionPolicy::decide`
// through `authorize_and_invoke` BEFORE any tool invocation or workspace write,
// follows the documented lifecycle (append `permission.requested`, evaluate,
// append `permission.decided`, and on a non-observational decision append
// `capability.grant_created`), records `decision_source`/`persistence`/
// `explanation` on the decision, and surfaces a `deny` as a typed decide outcome
// (with a structured, agent-readable refusal) that blocks the invocation.

/// SG1 allow path: an allowed request emits the requested -> decided ->
/// grant-created sequence (lifecycle steps 2,4,5), records the full decision
/// fields on the decided + grant-created events, materializes a durable grant
/// projection, and reports a typed allow decide outcome -- and the tool then
/// proceeds (grant_used + invocation + completion).
#[test]
fn sg1_allowed_request_emits_requested_decided_grant_created_sequence() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg1-allow-session"));
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        // The TrustedLocal policy remains the real controller default; it is
        // reachable through the real loop (not the fake test-only policy).
        PermissionPolicy::allow_trusted_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("sg1-allow-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Inspect agent status under the decide step")
        .expect("send task");

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-sg1-allow"),
        tool_call_id: ToolCallId::new("tool-sg1-allow"),
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
        .expect("dispatch allowed capo tool");

    // Typed decide outcome: allowed, grant created, no refusal.
    assert!(outcome.decide.allowed);
    assert_eq!(outcome.decide.effect, "allow");
    assert!(outcome.decide.grant_created);
    assert_eq!(outcome.decide.refusal, None);
    assert_eq!(
        outcome.decide.decision_source,
        "allow_trusted_local_profile"
    );
    assert_eq!(outcome.decide.persistence, "until_session_end");
    assert!(!outcome.decide.explanation.is_empty());

    // The documented lifecycle sequence: requested -> decided -> grant-created
    // appear in order, BEFORE the tool layer's grant_used + invocation.
    let decided = outcome
        .observed_event_kinds
        .iter()
        .position(|kind| kind == "permission.decided")
        .expect("permission.decided present");
    let grant_created = outcome
        .observed_event_kinds
        .iter()
        .position(|kind| kind == "capability.grant_created")
        .expect("capability.grant_created present");
    let grant_used = outcome
        .observed_event_kinds
        .iter()
        .position(|kind| kind == "capability.grant_used")
        .expect("capability.grant_used present");
    let requested = outcome
        .observed_event_kinds
        .iter()
        .position(|kind| kind == "permission.requested")
        .expect("permission.requested present");
    assert!(requested < decided, "requested precedes decided");
    assert!(decided < grant_created, "decided precedes grant-created");
    assert!(
        grant_created < grant_used,
        "grant-created precedes grant_used"
    );

    // The decided event payload records decision_source/persistence/explanation
    // (the audit trail is complete even when allowed).
    let turn_events = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-sg1-allow")
        .expect("turn events");
    let decided_event = turn_events
        .iter()
        .find(|event| event.kind == "permission.decided")
        .expect("decided event persisted");
    assert!(decided_event.payload_json.contains("\"effect\":\"allow\""));
    assert!(
        decided_event
            .payload_json
            .contains("\"decision_source\":\"allow_trusted_local_profile\"")
    );
    assert!(
        decided_event
            .payload_json
            .contains("\"persistence\":\"until_session_end\"")
    );
    assert!(decided_event.payload_json.contains("\"explanation\":"));

    // A durable grant projection was created and is read-backable.
    let grant_created_event = turn_events
        .iter()
        .find(|event| event.kind == "capability.grant_created")
        .expect("grant-created event persisted");
    assert!(
        grant_created_event
            .payload_json
            .contains(&outcome.decide.capability_grant_id)
    );
    let grants = controller.state().capability_grants().expect("grants");
    let grant = grants
        .iter()
        .find(|grant| grant.capability_grant_id == outcome.decide.capability_grant_id)
        .expect("durable grant projection present");
    assert_eq!(grant.effect, "allow");
    assert_eq!(grant.decision_source, "allow_trusted_local_profile");
    assert_eq!(grant.persistence, "until_session_end");

    // The tool then proceeded (the decide step gated the call but allowed it).
    assert_eq!(outcome.status, "completed");
    assert!(
        outcome
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "tool.call_completed")
    );
}

/// SG1 deny path: a denied request blocks the invocation -- NO tool runs (no
/// `capability.grant_used`, no `tool.invocation_started`, no
/// `tool.call_completed`) -- and surfaces a typed `deny` decide outcome with a
/// structured, agent-readable refusal (not a raw error string) plus the full
/// decision fields recorded on the decided event.
#[test]
fn sg1_denied_request_blocks_invocation_with_structured_refusal() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, PermissionPolicy, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg1-deny-session"));
    // The Static read-only policy is reachable through the real loop and denies a
    // write/mutating tool.
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("sg1-deny-worker").expect("agent");
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
        turn_id: TurnId::new("turn-sg1-deny"),
        tool_call_id: ToolCallId::new("tool-sg1-deny"),
    };
    // capo.evidence_record is an ACI write/mutating tool; the read-only policy denies it.
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

    // Typed deny decide outcome with a structured, agent-readable refusal.
    assert!(!outcome.decide.allowed);
    assert_eq!(outcome.decide.effect, "deny");
    assert_eq!(outcome.status, "denied");
    let refusal = outcome
        .decide
        .refusal
        .as_ref()
        .expect("a denied write maps to a structured refusal");
    assert_eq!(refusal.tool_name, "capo.evidence_record");
    assert_eq!(refusal.decision_source, "static_policy:read-only-local");
    assert!(!refusal.reason.is_empty());
    // The agent-readable message is a structured line, not a raw error string.
    let message = refusal.agent_message();
    assert!(message.contains("capo.evidence_record"));
    assert!(message.contains("static_policy:read-only-local"));

    // The invocation was BLOCKED: no tool ran. None of the grant-use / invocation
    // / completion events were emitted.
    for blocked_kind in [
        "capability.grant_used",
        "tool.invocation_started",
        "tool.output_observed",
        "tool.call_completed",
        "tool.result_delivered",
    ] {
        assert!(
            !outcome
                .observed_event_kinds
                .iter()
                .any(|kind| kind == blocked_kind),
            "denied dispatch must not emit {blocked_kind}",
        );
    }

    // The decide step still recorded the decision (requested + decided) so the
    // audit trail is complete. The static policy denies with `persistence="once"`
    // (a `reject_once`), which per the ACP option-mapping table
    // (`capability-permissions.md:387`) records the rejection for THIS request and
    // creates NO grant -- only a `reject_always` (durable) deny materializes a
    // standing deny grant. So no `capability.grant_created` is emitted here.
    assert!(
        outcome
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "permission.requested")
    );
    assert!(
        outcome
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "permission.decided")
    );
    assert!(!outcome.decide.grant_created);
    assert!(
        !outcome
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "capability.grant_created"),
        "a reject_once deny records the rejection but creates no durable deny grant",
    );

    let turn_events = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-sg1-deny")
        .expect("turn events");
    // No tool invocation event was persisted for the denied call.
    assert!(
        !turn_events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );
    let decided_event = turn_events
        .iter()
        .find(|event| event.kind == "permission.decided")
        .expect("decided event persisted");
    assert!(decided_event.payload_json.contains("\"effect\":\"deny\""));
    assert!(
        decided_event
            .payload_json
            .contains("\"decision_source\":\"static_policy:read-only-local\"")
    );
    assert!(decided_event.payload_json.contains("\"explanation\":"));

    // The persisted tool-call projection reached the terminal denied status (it
    // did not stick at "requested"), and no output artifact was produced.
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

    // A `reject_once` deny creates NO durable grant: the grant store has no row
    // for this decision's grant id (the rejection is recorded on the
    // permission.decided event, not as a standing deny rule). A durable deny grant
    // is reserved for a future `reject_always` policy.
    let grants = controller.state().capability_grants().expect("grants");
    assert!(
        !grants
            .iter()
            .any(|grant| { grant.capability_grant_id == outcome.decide.capability_grant_id })
    );
}

/// SG1 (single canonical decide / loop consumes the typed refusal): the REAL turn
/// LOOP -- `RealBoundaryController::send_task`, the production chat/send-task path
/// -- no longer runs its own second upfront `permission_policy.decide`. Its ONLY
/// decide is the dispatch's, and the loop CONSUMES the typed
/// `PermissionDecideOutcome`/`ToolRefusal` that dispatch returns.
///
/// This proves the loop SURFACES the typed refusal (not merely that the call was
/// blocked): a standing durable `reject_always` deny grant (the SG3 read-back deny,
/// reachable in production) blocks the per-turn `capo.session_summary` dispatch, and
/// `send_task` reflects the refusal's structured `agent_message()` back onto the
/// blocked session read model -- the session `latest_blocker` IS the agent-readable
/// refusal line, not a raw error string -- and skips the downstream
/// memory/artifact/evidence steps. Under a PERMISSIVE policy (TrustedLocal) the only
/// thing that can deny here is the single canonical (dispatch) decide, so a block
/// here cannot be the dead upfront gate.
#[test]
fn sg1_real_loop_send_task_surfaces_typed_refusal_when_dispatch_denies() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    let agent_name = "sg1-loop-deny-worker";
    let session_id = format!("session-{agent_name}");
    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg1-loop-deny"));
    // TrustedLocal ALLOWS `capo.session_summary` (the permissive default), so the
    // upfront-decide gate this change removed would NOT have blocked it. The only
    // authority that can deny is the dispatch's single canonical decide via the SG3
    // standing-deny read-back seeded below.
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::allow_trusted_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent(agent_name).expect("agent");

    // The exact scope_json the per-turn `capo.session_summary` dispatch decides over
    // (the tool's own required scopes) -- so the seeded standing deny matches the
    // dispatch read-back's (subject + scope) key.
    let scope_json = controller
        .capo_registry()
        .expect("capo registry")
        .describe_tool("capo.session_summary")
        .expect("session_summary defined")
        .required_scopes_json;

    // Seed a standing durable DENY grant for the loop's subject + the session-summary
    // scope. The dispatch's grant read-back (SG3) enforces this as a `reject_always`
    // standing denial, so the single canonical decide denies the per-turn tool.
    sg3_seed_grant(
        &controller,
        capo_state::CapabilityGrantProjection {
            capability_grant_id: "grant-sg1-loop-standing-deny".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json,
            effect: "deny".to_string(),
            subject_json: format!("{{\"session_id\":\"{session_id}\"}}"),
            decision_source: "reject_always".to_string(),
            persistence: "until_revoked".to_string(),
            explanation: "operator standing denial of the per-turn summary tool".to_string(),
            created_at: Some("1700000000000".to_string()),
            expires_at: None,
            revoked_at: None,
            updated_sequence: 0,
        },
        "sg1-loop-standing-deny",
    );

    // Drive the REAL loop. send_task returns Ok with BLOCKED state (mirroring the
    // legacy deny path), not an error -- the deny is a reflected decide outcome.
    let refs = controller
        .send_task(&registration, "Inspect the session under a standing deny")
        .expect("send task returns blocked state, not an error");
    assert_eq!(refs.session_id.as_str(), session_id);
    let observation = controller.observe(&refs).expect("observe");

    // The loop SURFACED the typed refusal: the session blocker is the structured,
    // agent-readable refusal line (`ToolRefusal::agent_message()`), NOT a raw error.
    assert_eq!(observation.task.capo_execution_status, "blocked");
    assert_eq!(observation.agent.status, "paused");
    assert_eq!(observation.session.status, "waiting_for_permission");
    let blocker = observation
        .session
        .latest_blocker
        .as_deref()
        .expect("the blocked session carries the reflected refusal");
    // The agent-readable refusal shape: names the refused tool and the deciding
    // authority -- the structured line `ToolRefusal::agent_message()` produces.
    assert!(
        blocker.starts_with("Permission denied for tool `capo.session_summary`"),
        "the session blocker is the typed refusal's agent message, got: {blocker}",
    );
    assert!(
        blocker.contains("reject_always"),
        "the reflected refusal names the deciding authority (the standing deny), got: {blocker}",
    );

    // The dispatch's decide recorded the lifecycle (requested + decided) AND blocked
    // the tool: the tool-call projection reached terminal `denied`, and NO tool ran.
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tool calls");
    assert_eq!(tools.len(), 1, "exactly the one denied per-turn dispatch");
    assert_eq!(tools[0].status, "denied");
    assert!(tools[0].output_artifact_id.is_none());

    let turn_events = controller
        .state()
        .events_for_session_turn(&refs.session_id, &format!("turn-{agent_name}"))
        .expect("session turn events");
    assert!(
        turn_events
            .iter()
            .any(|event| event.kind == "permission.decided"
                && event.payload_json.contains("\"effect\":\"deny\"")),
        "the single canonical decide recorded the deny on the log",
    );
    for blocked_kind in [
        "capability.grant_used",
        "tool.invocation_started",
        "tool.output_observed",
        "tool.call_completed",
        "tool.result_delivered",
    ] {
        assert!(
            !turn_events.iter().any(|event| event.kind == blocked_kind),
            "a denied per-turn dispatch must not emit {blocked_kind}",
        );
    }

    // The early-return from the typed refusal skipped the downstream steps, exactly
    // like the legacy deny path: no memory packet, no evidence.
    assert!(
        controller
            .state()
            .memory_packets_for_session(&refs.session_id)
            .expect("memory packets")
            .is_empty(),
    );
    assert!(
        controller
            .state()
            .evidence_for_session(&refs.session_id)
            .expect("evidence")
            .is_empty(),
    );
}

// --- SG2: AgentAdapter permission round-trip + ACP option mapping (fixtures) -
//
// A fake/scripted adapter raises an `AdapterPermissionRequest` carrying the ACP
// `PermissionOption[]`, the controller decides it through `PermissionPolicy` +
// the `capability-permissions.md` ACP option-mapping table, and the chosen
// outcome (the selected `optionId` or `cancelled`) is returned to the adapter
// using the provider-neutral adapter types. The ACP option list + chosen option
// id are persisted as `adapter_options`/`adapter_response` on the decision
// record. Fixture/option-mapping only -- NO live ACP JSON-RPC wire.

/// Build the controller + a scripted ACP-shaped adapter that raises one scripted
/// permission round-trip, and return (controller, refs, scope) for a round-trip.
fn sg2_round_trip_setup(
    label: &str,
    policy: PermissionPolicy,
    request: AdapterPermissionRequest,
) -> (
    RealBoundaryController,
    RealRunRefs,
    crate::PermissionRoundTripScope,
    AdapterPermissionRequest,
    capo_tmptest::TempRoot,
) {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};

    let request_ref = format!("perm-{label}");
    let scripted = AgentAdapterHandle::scripted_mock(
        ScriptedMockAgent::acp_shaped(format!("sg2-{label}-session"))
            .with_permission_request(&request_ref, request.clone()),
    );
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        policy,
        scripted.clone(),
    )
    .expect("open real controller");
    let registration = controller
        .register_agent(&format!("sg2-{label}-worker"))
        .expect("agent");
    let refs = controller
        .send_task(
            &registration,
            &format!("Drive an ACP permission round-trip ({label})"),
        )
        .expect("send task");

    // The adapter raises the scripted permission request through the
    // provider-neutral `AgentAdapter` boundary (not a `Fake*` struct).
    let raised = scripted
        .scripted_permission_request(&request_ref)
        .expect("adapter raises scripted permission request");
    assert_eq!(raised, request);

    let scope = crate::PermissionRoundTripScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new(format!("turn-sg2-{label}")),
        request_ref,
    };
    (controller, refs, scope, raised, tmp_state_root)
}

fn sg2_options(kinds: &[AcpPermissionOptionKind]) -> Vec<AcpPermissionOption> {
    kinds
        .iter()
        .map(|kind| {
            AcpPermissionOption::new(format!("opt-{}", kind.as_str()), kind.as_str(), *kind)
        })
        .collect()
}

fn sg2_decided_event(
    controller: &RealBoundaryController,
    refs: &RealRunRefs,
    turn_id: &str,
) -> capo_state::EventRecord {
    controller
        .state()
        .events_for_session_turn(&refs.session_id, turn_id)
        .expect("turn events")
        .into_iter()
        .find(|event| event.kind == "permission.decided")
        .expect("permission.decided persisted")
}

/// SG2 allow_once: maps to an allow once/turn-scoped grant and returns the
/// matching ACP `optionId`. The lifecycle (requested -> decided -> grant-created)
/// is persisted, and the decided event records adapter_options/adapter_response.
#[test]
fn sg2_allow_once_round_trip_allows_turn_scoped_and_returns_option_id() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[
            AcpPermissionOptionKind::AllowOnce,
            AcpPermissionOptionKind::AllowAlways,
            AcpPermissionOptionKind::RejectOnce,
        ]),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "allow-once",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    // The chosen outcome returned to the adapter: the allow_once optionId.
    assert!(response.allowed());
    assert_eq!(response.capo_decision, "allow");
    assert_eq!(response.outcome.option_id(), Some("opt-allow_once"));
    assert_eq!(response.capo_persistence.as_deref(), Some("until_turn_end"));
    assert!(!response.adapter_error);
    // An honored allow is the ONLY path where the adapter may proceed.
    assert!(!response.must_not_proceed);
    assert!(response.may_proceed());
    let grant_id = response
        .capability_grant_id
        .as_ref()
        .expect("allow_once creates a turn-scoped grant");

    // The decided event records the chosen option id and the offered option list.
    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-allow-once");
    assert!(decided.payload_json.contains("\"decision\":\"allow\""));
    assert!(
        decided
            .payload_json
            .contains("\"option_id\":\"opt-allow_once\"")
    );
    assert!(decided.payload_json.contains("\"outcome\":\"selected\""));
    assert!(decided.payload_json.contains("opt-allow_always"));
    assert!(
        decided
            .payload_json
            .contains("\"persistence\":\"until_turn_end\"")
    );

    // A durable grant projection was created and is read-backable.
    let grants = controller.state().capability_grants().expect("grants");
    let grant = grants
        .iter()
        .find(|grant| &grant.capability_grant_id == grant_id)
        .expect("turn-scoped grant projection");
    assert_eq!(grant.effect, "allow");
    assert_eq!(grant.persistence, "until_turn_end");
}

/// SG2 allow_always (alone, under TrustedLocal): chosen but DOWNSCOPED to
/// `until_session_end` (never a durable remembered grant without profile opt-in),
/// returning the allow_always optionId.
#[test]
fn sg2_allow_always_round_trip_downscopes_to_session_end() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[
            AcpPermissionOptionKind::AllowAlways,
            AcpPermissionOptionKind::RejectAlways,
        ]),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "allow-always",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    assert!(response.allowed());
    assert_eq!(response.outcome.option_id(), Some("opt-allow_always"));
    // TrustedLocal downscope: NOT `until_revoked`.
    assert_eq!(
        response.capo_persistence.as_deref(),
        Some("until_session_end")
    );
    let grant_id = response
        .capability_grant_id
        .as_ref()
        .expect("allow_always creates a session-scoped grant");
    let grants = controller.state().capability_grants().expect("grants");
    let grant = grants
        .iter()
        .find(|grant| &grant.capability_grant_id == grant_id)
        .expect("session-scoped grant projection");
    assert_eq!(grant.persistence, "until_session_end");

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-allow-always");
    assert!(
        decided
            .payload_json
            .contains("\"option_id\":\"opt-allow_always\"")
    );
    assert!(
        decided
            .payload_json
            .contains("\"persistence\":\"until_session_end\"")
    );
}

/// SG2 reject_once: rejects with the correct returned `optionId`, records a Capo
/// reject, and creates NO grant (transient rejection).
#[test]
fn sg2_reject_once_round_trip_rejects_with_no_grant() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::RejectOnce]),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "reject-once",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    assert!(!response.allowed());
    assert_eq!(response.capo_decision, "deny");
    assert_eq!(response.outcome.option_id(), Some("opt-reject_once"));
    assert!(!response.adapter_error);
    // A `reject_once` is transient: no durable grant for THIS round-trip (the
    // grant store may already hold the per-turn summary tool's allow grant).
    assert_eq!(response.capability_grant_id, None);
    let grants = controller.state().capability_grants().expect("grants");
    assert!(
        !grants
            .iter()
            .any(|grant| grant.capability_grant_id.contains("round-trip")),
        "a reject_once round-trip creates no durable grant",
    );

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-reject-once");
    assert!(decided.payload_json.contains("\"decision\":\"reject\""));
    assert!(
        decided
            .payload_json
            .contains("\"option_id\":\"opt-reject_once\"")
    );
}

/// SG2 reject_always: rejects with the correct returned `optionId` AND creates a
/// scoped durable deny grant (`effect = deny`, `until_revoked`).
#[test]
fn sg2_reject_always_round_trip_creates_durable_deny_grant() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::RejectAlways]),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "reject-always",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    assert!(!response.allowed());
    assert_eq!(response.capo_decision, "deny");
    assert_eq!(response.outcome.option_id(), Some("opt-reject_always"));
    assert_eq!(response.capo_persistence.as_deref(), Some("until_revoked"));
    let grant_id = response
        .capability_grant_id
        .as_ref()
        .expect("reject_always creates a durable deny grant");
    let grants = controller.state().capability_grants().expect("grants");
    let grant = grants
        .iter()
        .find(|grant| &grant.capability_grant_id == grant_id)
        .expect("durable deny grant projection");
    assert_eq!(grant.effect, "deny");
    assert_eq!(grant.persistence, "until_revoked");

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-reject-always");
    assert!(decided.payload_json.contains("\"decision\":\"reject\""));
    assert!(
        decided
            .payload_json
            .contains("\"option_id\":\"opt-reject_always\"")
    );
}

/// SG2 cancellation: an explicit operator cancel returns the ACP `cancelled`
/// outcome and records `permission.decided` with `decision = cancel`. No grant.
#[test]
fn sg2_cancellation_round_trip_returns_cancelled_and_records_cancel() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, refs, scope, raised, _state) =
        sg2_round_trip_setup("cancel", PermissionPolicy::allow_trusted_local(), request);

    let response = controller
        .cancel_adapter_permission(
            &scope,
            &raised,
            crate::PermissionCancellation::OperatorCancelled,
        )
        .expect("cancel round-trip");

    assert_eq!(response.capo_decision, "cancel");
    assert_eq!(response.outcome, AcpPermissionOutcome::Cancelled);
    assert_eq!(response.outcome.kind(), "cancelled");
    assert_eq!(response.capability_grant_id, None);
    // An operator cancel is NOT an adapter error.
    assert!(!response.adapter_error);
    let grants = controller.state().capability_grants().expect("grants");
    assert!(
        !grants
            .iter()
            .any(|grant| grant.capability_grant_id.contains("round-trip")),
        "a canceled round-trip creates no durable grant",
    );

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-cancel");
    assert!(decided.payload_json.contains("\"decision\":\"cancel\""));
    assert!(decided.payload_json.contains("\"outcome\":\"cancelled\""));
}

/// SG2 no-selectable-option: an empty ACP option list is an adapter error.
/// Records `permission.decided` with `cancel`, returns `cancelled`, and flags the
/// adapter request as failed (`adapter_error`) rather than inventing an outcome.
#[test]
fn sg2_no_selectable_option_is_adapter_error_cancel() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        Vec::new(),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "no-option",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    assert_eq!(response.capo_decision, "cancel");
    assert_eq!(response.outcome, AcpPermissionOutcome::Cancelled);
    // The adapter request must be FAILED, not satisfied with an invented outcome.
    assert!(response.adapter_error);
    assert_eq!(response.capability_grant_id, None);

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-no-option");
    assert!(decided.payload_json.contains("\"decision\":\"cancel\""));
    assert!(decided.payload_json.contains("no_selectable_option"));
}

/// SG2 policy authority: an adapter offering an allow option CANNOT over-rule a
/// policy that denies the scope. The Static read-only policy denies a write
/// scope, so the round-trip reflects a Capo reject even though `allow_once` was
/// offered -- no allow, no grant.
#[test]
fn sg2_policy_deny_overrules_adapter_allow_option() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        // A write scope the read-only static policy does not include.
        "filesystem:write:workspace",
        "read-only-local",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, refs, scope, raised, _state) = sg2_round_trip_setup(
        "policy-deny",
        PermissionPolicy::static_read_only_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    // The adapter offered allow_once, but the policy denies the scope: deny wins.
    assert!(!response.allowed());
    assert_eq!(response.capo_decision, "deny");
    assert_eq!(response.capability_grant_id, None);
    // SAFETY-CRITICAL: the wire outcome returned to the adapter must NOT be the
    // offered allow option's `selected{opt-allow_once}` -- an ACP adapter reads
    // that as "permitted, proceed" and would run the exact call the policy denied.
    // No reject option was offered here, so the outcome is `cancelled`, and the
    // explicit `must_not_proceed` flag halts the adapter unambiguously.
    assert_ne!(
        response.outcome,
        AcpPermissionOutcome::Selected {
            option_id: "opt-allow_once".to_string()
        },
        "a policy-denied allow option must NOT return that allow option's selected outcome",
    );
    assert_eq!(response.outcome, AcpPermissionOutcome::Cancelled);
    assert!(
        response.must_not_proceed,
        "a policy deny must signal must_not_proceed so the adapter cannot proceed",
    );
    assert!(!response.may_proceed());
    let grants = controller.state().capability_grants().expect("grants");
    assert!(
        !grants
            .iter()
            .any(|grant| grant.capability_grant_id.contains("round-trip")),
        "a policy-denied round-trip creates no durable grant",
    );

    let decided = sg2_decided_event(&controller, &refs, "turn-sg2-policy-deny");
    assert!(decided.payload_json.contains("\"decision\":\"reject\""));
    // The persisted adapter_response must match the wire outcome (cancelled), not
    // the contradictory `selected{opt-allow_once}` the raw mapping carried.
    assert!(decided.payload_json.contains("\"outcome\":\"cancelled\""));
    assert!(
        !decided
            .payload_json
            .contains("\"option_id\":\"opt-allow_once\""),
        "the decided record must not persist the allow option id as the chosen response",
    );
    assert!(
        decided
            .payload_json
            .contains("\"decision_source\":\"static_policy:read-only-local\"")
    );
}

/// SG2 policy authority (reject option offered): when the policy denies the scope
/// but the adapter offered BOTH an allow and a reject option, the over-rule
/// returns the REJECT option's `optionId` (a reject outcome the adapter cannot
/// misread as proceed), still with `must_not_proceed` set and no grant.
#[test]
fn sg2_policy_deny_returns_reject_option_when_offered() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "read-only-local",
        sg2_options(&[
            AcpPermissionOptionKind::AllowOnce,
            AcpPermissionOptionKind::RejectOnce,
        ]),
    );
    let (controller, _refs, scope, raised, _state) = sg2_round_trip_setup(
        "policy-deny-reject",
        PermissionPolicy::static_read_only_local(),
        request,
    );

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");

    assert!(!response.allowed());
    assert_eq!(response.capo_decision, "deny");
    // The wire outcome is the offered reject option's id, NOT the allow option's.
    assert_eq!(response.outcome.option_id(), Some("opt-reject_once"));
    assert!(response.must_not_proceed);
    // A policy over-rule of an allow option still materializes no grant: the
    // reject option id is only the wire signal, not a durable reject decision.
    assert_eq!(response.capability_grant_id, None);
}

/// SG2 loop-driven round-trip (closing leg): the loop PULLS the raised request
/// from the bound adapter, the controller decides it, and the response is
/// DELIVERED back to the adapter -- and the adapter proceeds on an allow. This
/// proves the full raise -> decide -> deliver round-trip is driven THROUGH the
/// loop, not assembled by the test harness, and that the closing leg exists.
#[test]
fn sg2_loop_drives_round_trip_allow_and_adapter_proceeds() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, _refs, scope, _raised, _state) = sg2_round_trip_setup(
        "loop-allow",
        PermissionPolicy::allow_trusted_local(),
        request,
    );

    // The LOOP pulls the raised request, decides it, and delivers it back.
    let outcome = controller
        .run_adapter_permission_round_trip(&scope)
        .expect("loop drives round-trip")
        .expect("adapter raised a request for this request_ref");

    assert!(outcome.response.allowed());
    assert_eq!(outcome.response.outcome.option_id(), Some("opt-allow_once"));
    // Closing leg: the adapter received the response and would proceed.
    assert!(outcome.delivery.proceeded);
    assert!(!outcome.delivery.adapter_error);
}

/// SG2 loop-driven round-trip (closing leg, deny): the loop delivers a
/// policy-deny-of-an-allow-option response back to the adapter, and the adapter
/// HALTS (does not proceed) -- so the over-rule is honored end-to-end, not just
/// in the returned struct.
#[test]
fn sg2_loop_drives_round_trip_policy_deny_halts_adapter() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "read-only-local",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, _refs, scope, _raised, _state) = sg2_round_trip_setup(
        "loop-policy-deny",
        PermissionPolicy::static_read_only_local(),
        request,
    );

    let outcome = controller
        .run_adapter_permission_round_trip(&scope)
        .expect("loop drives round-trip")
        .expect("adapter raised a request");

    assert!(!outcome.response.allowed());
    assert!(outcome.response.must_not_proceed);
    // Closing leg: the adapter must NOT proceed with the denied tool call.
    assert!(
        !outcome.delivery.proceeded,
        "the adapter must halt on a policy-denied allow option",
    );
}

/// SG2 loop-driven round-trip: when the adapter raised NO request for the
/// `request_ref`, the loop hook is a no-op (`None`), never inventing a decision.
#[test]
fn sg2_loop_round_trip_absent_request_is_noop() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, _refs, mut scope, _raised, _state) = sg2_round_trip_setup(
        "loop-absent",
        PermissionPolicy::allow_trusted_local(),
        request,
    );
    // Point the loop at a request_ref the adapter never scripted.
    scope.request_ref = "perm-loop-absent-unscripted".to_string();

    let outcome = controller
        .run_adapter_permission_round_trip(&scope)
        .expect("loop drives round-trip");
    assert!(outcome.is_none(), "no raised request means no decision");
}

/// SG2 round-trip lifecycle + restart/replay: the requested -> decided ->
/// grant-created sequence is persisted in order, and the durable grant rebuilds
/// identically from the event log (honoring the SG0 invariant).
#[test]
fn sg2_round_trip_lifecycle_rebuilds_from_event_log() {
    let request = AdapterPermissionRequest::new(
        "capo.file_write",
        "filesystem:write:workspace",
        "trusted-local-dev",
        sg2_options(&[AcpPermissionOptionKind::AllowOnce]),
    );
    let (controller, refs, scope, raised, _state) =
        sg2_round_trip_setup("replay", PermissionPolicy::allow_trusted_local(), request);

    let response = controller
        .decide_adapter_permission(&scope, &raised)
        .expect("decide round-trip");
    let grant_id = response.capability_grant_id.clone().expect("grant id");

    let turn_events = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-sg2-replay")
        .expect("turn events");
    let position = |kind: &str| {
        turn_events
            .iter()
            .position(|event| event.kind == kind)
            .unwrap_or_else(|| panic!("{kind} present"))
    };
    let requested = position("permission.requested");
    let decided = position("permission.decided");
    let grant_created = position("capability.grant_created");
    assert!(requested < decided, "requested precedes decided");
    assert!(decided < grant_created, "decided precedes grant-created");

    // Rebuild the projections from the event log; the grant reconstructs
    // identically.
    let before = controller.state().capability_grants().expect("grants");
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    let after = controller.state().capability_grants().expect("grants");
    assert_eq!(before, after, "grant projection rebuilds identically");
    assert!(
        after
            .iter()
            .any(|grant| grant.capability_grant_id == grant_id),
        "the round-trip grant survives a projection rebuild",
    );
}

// --- SG3: grant read-back + revoke/expire ----------------------------------

/// Seed a durable grant projection directly through the state store, mirroring
/// the `capability.grant_created` lifecycle event the real loop appends. Returns
/// the controller already holding the seeded grant.
fn sg3_seed_grant(
    controller: &RealBoundaryController,
    grant: capo_state::CapabilityGrantProjection,
    event_suffix: &str,
) {
    controller
        .state()
        .append_event(
            capo_state::NewEvent::new(
                format!("event-sg3-grant-{event_suffix}"),
                capo_state::EventKind::CapabilityGrantCreated,
                "test",
            ),
            &[capo_state::ProjectionRecord::CapabilityGrant(grant)],
        )
        .expect("seed grant");
}

/// SG4: seed a durable ALLOW grant for the `session-sg4` subject (the real
/// re-admission surface for a critical scope -- a reviewed durable grant, not the
/// test-only `allow_trusted_local_with_grants` constructor).
fn sg4_seed_critical_grant(
    controller: &RealBoundaryController,
    grant_id: &str,
    scope_json: &str,
    event_suffix: &str,
) {
    controller
        .state()
        .append_event(
            capo_state::NewEvent::new(
                format!("event-sg4-grant-{event_suffix}"),
                capo_state::EventKind::CapabilityGrantCreated,
                "test",
            ),
            &[capo_state::ProjectionRecord::CapabilityGrant(
                capo_state::CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: scope_json.to_string(),
                    effect: "allow".to_string(),
                    subject_json: "{\"session_id\":\"session-sg4\"}".to_string(),
                    decision_source: "user".to_string(),
                    persistence: "until_revoked".to_string(),
                    explanation: "operator-reviewed critical-scope grant".to_string(),
                    created_at: Some("1700000000000".to_string()),
                    expires_at: None,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("seed sg4 critical grant");
}

fn sg3_grant(
    grant_id: &str,
    scope_json: &str,
    expires_at: Option<&str>,
) -> capo_state::CapabilityGrantProjection {
    capo_state::CapabilityGrantProjection {
        capability_grant_id: grant_id.to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: scope_json.to_string(),
        effect: "allow".to_string(),
        subject_json: "{\"session_id\":\"session-sg3\"}".to_string(),
        decision_source: "allow_trusted_local_profile".to_string(),
        persistence: "until_revoked".to_string(),
        explanation: "seeded allow grant".to_string(),
        created_at: Some("1700000000000".to_string()),
        expires_at: expires_at.map(str::to_string),
        revoked_at: None,
        updated_sequence: 0,
    }
}

/// SG3 read-back: a valid durable allow grant authorizes a later request even
/// when the policy itself would deny the scope (grants are not write-only).
#[test]
fn sg3_grant_read_back_authorizes_a_valid_durable_grant() {
    // A read-only-local STATIC policy denies `filesystem:write:workspace`...
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
    )
    .expect("open controller");
    let scope_json = "[\"filesystem:write:workspace\"]".to_string();

    // Without any grant, the decide step falls through to the policy and denies.
    let before = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
        })
        .expect("decide without grant");
    assert!(!before.allowed);
    assert_eq!(before.source, crate::GrantReadBackSource::Policy);
    assert!(before.authorizing_grant_id.is_none());

    // ...but with a valid durable allow grant for that exact scope, read-back
    // authorizes the request via the grant, not the policy.
    sg3_seed_grant(
        &controller,
        sg3_grant("grant-sg3-readback", &scope_json, None),
        "readback",
    );
    let after = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json,
        })
        .expect("decide with grant");
    assert!(after.allowed, "a valid grant authorizes the request");
    assert_eq!(after.source, crate::GrantReadBackSource::DurableGrant);
    assert_eq!(
        after.authorizing_grant_id.as_deref(),
        Some("grant-sg3-readback")
    );
    // The policy still records a deny (the grant, not the policy, authorized).
    assert_eq!(after.policy_decision.effect, "deny");
}

/// SG3 revoke: revoking a grant then re-requesting the same scope is denied
/// (the revoked grant reads as absent), while the original
/// `capability.grant_created`/`capability.grant_used` events remain unchanged and
/// a `capability.grant_revoked` event with the reason is appended.
#[test]
fn sg3_revoke_then_re_request_is_denied_and_old_events_preserved() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
    )
    .expect("open controller");
    let registration = controller.register_agent("sg3-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Drive an SG3 revoke flow")
        .expect("send task");
    let scope_json = "[\"filesystem:write:workspace\"]".to_string();

    // Seed an allow grant for the scope and a grant-used event against it.
    sg3_seed_grant(
        &controller,
        sg3_grant("grant-sg3-revoke", &scope_json, None),
        "revoke",
    );
    controller
        .state()
        .append_event(
            capo_state::NewEvent::new(
                "event-sg3-grant-used",
                capo_state::EventKind::CapabilityGrantUsed,
                "test",
            ),
            &[],
        )
        .expect("seed grant-used");

    // Read-back authorizes while the grant is valid.
    let granted = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
        })
        .expect("decide with grant");
    assert!(granted.allowed);
    assert_eq!(granted.source, crate::GrantReadBackSource::DurableGrant);

    let events_before = controller.state().event_count().expect("event count");

    // Revoke the grant with a reason.
    let revoke_scope = crate::GrantRevocationScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-sg3-revoke"),
    };
    let revocation = controller
        .revoke_capability_grant(&revoke_scope, "grant-sg3-revoke", "stricter policy")
        .expect("revoke grant");
    assert_eq!(revocation.capability_grant_id, "grant-sg3-revoke");
    assert_eq!(revocation.reason, "stricter policy");

    // Re-requesting the same scope after revoke is denied: the revoked grant
    // reads as absent, so read-back falls through to the denying policy.
    let after = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json,
        })
        .expect("decide after revoke");
    assert!(!after.allowed, "a revoked grant no longer authorizes");
    assert_eq!(after.source, crate::GrantReadBackSource::Policy);
    assert!(after.authorizing_grant_id.is_none());

    // The grant projection carries revoked_at; the durable store reads it back.
    let grant = controller
        .state()
        .capability_grant_by_id("grant-sg3-revoke")
        .expect("grant by id")
        .expect("grant present");
    assert!(grant.is_revoked());
    assert_eq!(
        grant.revoked_at.as_deref(),
        Some(revocation.revoked_at.as_str())
    );

    // The original grant-created and grant-used events are preserved unchanged;
    // revocation only ADDS a `capability.grant_revoked` event.
    let events_after = controller.state().event_count().expect("event count");
    assert_eq!(
        events_after,
        events_before + 1,
        "revoke adds exactly one event"
    );
    let revoked_event = controller
        .state()
        .events_for_session_turn(&refs.session_id, "turn-sg3-revoke")
        .expect("turn events")
        .into_iter()
        .find(|event| event.kind == "capability.grant_revoked")
        .expect("grant_revoked event present");
    assert!(
        revoked_event
            .payload_json
            .contains("\"reason\":\"stricter policy\"")
    );
    assert!(revoked_event.payload_json.contains("grant-sg3-revoke"));

    // A rebuild from the log reconstructs the revoked state identically (the old
    // created/used events plus the revoke event yield the same revoked grant).
    let before_rebuild = controller.state().capability_grants().expect("grants");
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    let after_rebuild = controller.state().capability_grants().expect("grants");
    assert_eq!(before_rebuild, after_rebuild);
    assert!(
        after_rebuild
            .iter()
            .any(|grant| { grant.capability_grant_id == "grant-sg3-revoke" && grant.is_revoked() })
    );
}

/// SG3 expiry: a grant past its `expires_at` does not authorize even though it was
/// never explicitly revoked (expiry is a denial input in decide).
#[test]
fn sg3_expired_grant_does_not_authorize() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
    )
    .expect("open controller");
    let scope_json = "[\"filesystem:write:workspace\"]".to_string();

    // Seed a grant whose `expires_at` is already in the past relative to the
    // wall clock (epoch-millis 1, far before now).
    sg3_seed_grant(
        &controller,
        sg3_grant("grant-sg3-expired", &scope_json, Some("1")),
        "expired",
    );

    let decision = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json,
        })
        .expect("decide with expired grant");
    assert!(
        !decision.allowed,
        "an expired grant does not authorize, even without an explicit revoke",
    );
    assert_eq!(decision.source, crate::GrantReadBackSource::Policy);
    assert!(decision.authorizing_grant_id.is_none());
    // The grant exists in the store but is past its expiry.
    let grant = controller
        .state()
        .capability_grant_by_id("grant-sg3-expired")
        .expect("grant by id")
        .expect("grant present");
    assert!(
        !grant.is_revoked(),
        "the grant was never explicitly revoked"
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis()
        .to_string();
    assert!(grant.is_expired(&now));
}

/// SG3 review-fix (CRITICAL: read-back is the LIVE gate). A valid durable allow
/// grant authorizes a real agent-driven tool call THROUGH the single live decide
/// path (`dispatch_tool_call`), even when the configured policy would deny it.
/// This proves read-back is wired into the loop's gate, not a parallel test-only
/// API: the tool runs, the audit flows, and the decide outcome names the durable
/// grant as the authority.
#[test]
fn sg3_live_dispatch_durable_grant_authorizes_a_policy_denied_write() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg3-live-session"));
    // The read-only-local STATIC policy DENIES `capo.evidence_record` (a write).
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("sg3-live-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Drive a live SG3 read-back dispatch")
        .expect("send task");

    let request = CapoToolRequest {
        tool_call_id: ToolCallId::new("tool-sg3-live-allow"),
        session_id: refs.session_id.clone(),
        tool_id: "capo.evidence_record".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        context: CapoToolContext {
            task_status: "task active".to_string(),
            agent_status: "agent running".to_string(),
            session_summary: "summary".to_string(),
            workpad_excerpt: "section".to_string(),
            evidence_note: "note".to_string(),
            capability_scope: "state:write:evidence".to_string(),
        },
    };
    let exposure_request = ToolExposureRequest::Capo(request.clone());
    // Derive the exact scope the live gate keys on, so the seeded grant matches
    // what the dispatch path will read back (no brittle hand-built scope string).
    let scope_json = controller
        .capo_registry()
        .expect("capo registry")
        .describe_tool("capo.evidence_record")
        .expect("evidence_record defined")
        .required_scopes_json;

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-sg3-live-allow"),
        tool_call_id: request.tool_call_id.clone(),
    };

    // WITHOUT a grant, the live gate consults the policy and the write is BLOCKED.
    let denied = controller
        .dispatch_tool_call(&scope, exposure_request.clone())
        .expect("dispatch without grant");
    assert!(
        !denied.decide.allowed,
        "policy denies the write with no grant"
    );
    assert_eq!(denied.status, "denied");
    assert_eq!(
        denied.decide.read_back_source,
        crate::GrantReadBackSource::Policy
    );
    assert!(
        !denied
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "tool.call_completed"),
        "the denied write must not run"
    );

    // Seed a valid durable allow grant for the SAME subject (session +
    // capability profile) and scope.
    sg3_seed_grant(
        &controller,
        capo_state::CapabilityGrantProjection {
            capability_grant_id: "grant-sg3-live-allow".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
            effect: "allow".to_string(),
            subject_json: format!("{{\"session_id\":\"{}\"}}", refs.session_id),
            decision_source: "allow_trusted_local_profile".to_string(),
            persistence: "until_revoked".to_string(),
            explanation: "operator-approved durable write grant".to_string(),
            created_at: Some("1700000000000".to_string()),
            expires_at: None,
            revoked_at: None,
            updated_sequence: 0,
        },
        "live-allow",
    );

    // Re-dispatch the SAME tool call through the live gate: the durable grant
    // authorizes it even though the configured policy still denies the scope, the
    // tool RUNS, and the decide outcome names the durable grant.
    let scope = ToolDispatchScope {
        turn_id: TurnId::new("turn-sg3-live-allow-2"),
        tool_call_id: ToolCallId::new("tool-sg3-live-allow-2"),
        ..scope
    };
    let request = CapoToolRequest {
        tool_call_id: scope.tool_call_id.clone(),
        ..request
    };
    let allowed = controller
        .dispatch_tool_call(&scope, ToolExposureRequest::Capo(request))
        .expect("dispatch with durable grant");
    assert!(
        allowed.decide.allowed,
        "a valid durable grant authorizes the live call"
    );
    assert_eq!(allowed.status, "completed");
    assert_eq!(
        allowed.decide.read_back_source,
        crate::GrantReadBackSource::DurableGrant,
        "the live gate's authority is the durable grant, not the policy"
    );
    assert_eq!(allowed.decide.capability_grant_id, "grant-sg3-live-allow");
    assert!(
        allowed
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "tool.call_completed"),
        "the authorized tool actually runs through the live path"
    );
}

/// SG3 review-fix (SECURITY: standing deny grants participate in the live gate).
/// A durable `deny` grant (a `reject_always` standing denial) BLOCKS a real tool
/// call through `dispatch_tool_call` even when the configured policy would allow
/// it. Without this, a previously `reject_always`-denied scope would be silently
/// re-authorized under a permissive policy.
#[test]
fn sg3_live_dispatch_durable_deny_grant_blocks_a_policy_allowed_call() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg3-deny-session"));
    // The TrustedLocal policy ALLOWS this read tool (it is the permissive default).
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::allow_trusted_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller.register_agent("sg3-deny-worker").expect("agent");
    let refs = controller
        .send_task(&registration, "Drive a live SG3 deny-grant dispatch")
        .expect("send task");

    let scope_json = controller
        .capo_registry()
        .expect("capo registry")
        .describe_tool("capo.agent_status")
        .expect("agent_status defined")
        .required_scopes_json;

    // Seed a standing durable DENY grant for the subject + scope.
    sg3_seed_grant(
        &controller,
        capo_state::CapabilityGrantProjection {
            capability_grant_id: "grant-sg3-standing-deny".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
            effect: "deny".to_string(),
            subject_json: format!("{{\"session_id\":\"{}\"}}", refs.session_id),
            decision_source: "reject_always".to_string(),
            persistence: "until_revoked".to_string(),
            explanation: "operator standing denial".to_string(),
            created_at: Some("1700000000000".to_string()),
            expires_at: None,
            revoked_at: None,
            updated_sequence: 0,
        },
        "standing-deny",
    );

    let scope = ToolDispatchScope {
        task_id: refs.task_id.clone(),
        agent_id: refs.agent_id.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        turn_id: TurnId::new("turn-sg3-standing-deny"),
        tool_call_id: ToolCallId::new("tool-sg3-standing-deny"),
    };
    let outcome = controller
        .dispatch_tool_call(
            &scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: refs.session_id.clone(),
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
        .expect("dispatch under standing deny");

    assert!(
        !outcome.decide.allowed,
        "the standing deny grant blocks even a policy-allowed call"
    );
    assert_eq!(outcome.status, "denied");
    assert_eq!(
        outcome.decide.read_back_source,
        crate::GrantReadBackSource::DurableDenyGrant
    );
    assert!(
        !outcome
            .observed_event_kinds
            .iter()
            .any(|kind| kind == "tool.call_completed"),
        "no tool runs when a standing deny grant matches"
    );
}

/// SG4 review-fix (the critical-scope exclusion is enforced through the SAME live
/// decide step the loop runs, and re-admission is the REAL durable-grant read-back
/// path -- not the unused `allow_trusted_local_with_grants` constructor).
///
/// Under the DEFAULT `allow_trusted_local()` controller (no special opt-in), every
/// doc-enumerated critical scope decides DENY through `decide_with_grant_read_back`
/// (the controller decide step that `dispatch_tool_call`'s read-back gate funnels
/// into via `self.permission_policy.decide`). Then seeding a durable ALLOW grant
/// for that exact subject+scope re-admits the SAME critical-scope request through
/// read-back, while the configured policy still records a deny -- proving the
/// production re-admit mechanism (durable grants), not the test-only constructor,
/// is what re-opens a critical scope.
#[test]
fn sg4_default_trusted_local_controller_denies_each_critical_scope_until_durable_grant() {
    // The full `trusted-local-dev` v0 exclusion list (mirrors capo-tools'
    // SG4_CRITICAL_SCOPES); kept in sync by the per-scope deny assertion below.
    const SG4_CRITICAL_SCOPES: &[&str] = &[
        "filesystem:write:path",
        "network:connect:internet",
        "network:expose:public",
        "network:connect:private_tunnel",
        "secret:read:credential_material",
        "secret:write:credential_material",
        "voice:read:raw_transcript",
        "memory:export:project",
        "memory:sync:external",
        "browser:control:remote_page",
        "shell:execute:path",
    ];

    // DEFAULT controller: the permissive TrustedLocal policy, no critical opt-in.
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::allow_trusted_local(),
    )
    .expect("open default trusted-local controller");

    for (index, critical) in SG4_CRITICAL_SCOPES.iter().enumerate() {
        let scope_json = format!("[\"{critical}\"]");

        // Without a grant, the live decide step DENIES the critical scope even
        // under the permissive default policy: TrustedLocal no longer blanket-
        // allows it, and there is no durable grant, so read-back falls through to
        // the policy and the policy itself denies.
        let denied = controller
            .decide_with_grant_read_back(PermissionRequest {
                session_id: SessionId::new("session-sg4"),
                capability_profile_id: "trusted-local-dev".to_string(),
                scope_json: scope_json.clone(),
            })
            .expect("decide critical scope without grant");
        assert!(
            !denied.allowed,
            "default TrustedLocal must DENY critical scope `{critical}` with no grant"
        );
        assert_eq!(
            denied.source,
            crate::GrantReadBackSource::Policy,
            "with no grant the authority is the configured policy for `{critical}`"
        );
        assert!(
            denied.authorizing_grant_id.is_none(),
            "no grant authorizes `{critical}`"
        );
        assert_eq!(
            denied.policy_decision.effect, "deny",
            "the TrustedLocal policy itself denies critical scope `{critical}`"
        );
        assert!(
            denied.policy_decision.explanation.contains(critical),
            "the deny explanation names `{critical}`, got: {}",
            denied.policy_decision.explanation
        );

        // Seed a durable ALLOW grant for the SAME subject (session + profile) and
        // scope -- the REAL re-admission surface used in the running loop.
        let grant_id = format!("grant-sg4-critical-{index}");
        sg4_seed_critical_grant(
            &controller,
            &grant_id,
            &scope_json,
            &format!("crit-{index}"),
        );

        // The SAME critical-scope request now ALLOWS, authorized by the durable
        // grant (read-back), even though the configured policy STILL records a deny.
        let readmitted = controller
            .decide_with_grant_read_back(PermissionRequest {
                session_id: SessionId::new("session-sg4"),
                capability_profile_id: "trusted-local-dev".to_string(),
                scope_json,
            })
            .expect("decide critical scope with durable grant");
        assert!(
            readmitted.allowed,
            "a durable allow grant re-admits critical scope `{critical}` through read-back"
        );
        assert_eq!(
            readmitted.source,
            crate::GrantReadBackSource::DurableGrant,
            "the durable grant, not the policy, is the authority for `{critical}`"
        );
        assert_eq!(
            readmitted.authorizing_grant_id.as_deref(),
            Some(grant_id.as_str())
        );
        assert_eq!(
            readmitted.policy_decision.effect, "deny",
            "the configured TrustedLocal policy still denies `{critical}`; only the grant re-admits"
        );
    }
}

/// SG3 review-fix (SECURITY/CORRECTNESS: read-back is subject-scoped). A grant
/// minted for session A does NOT authorize an identical-scope request from
/// session B -- read-back matches the grant's subject (session id + capability
/// profile), not the scope string alone, so it never broadens authorization
/// across sessions/agents.
#[test]
fn sg3_grant_read_back_is_subject_scoped_across_sessions() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::static_read_only_local(),
    )
    .expect("open controller");
    let scope_json = "[\"filesystem:write:workspace\"]".to_string();

    // Seed an allow grant whose subject is session A.
    sg3_seed_grant(
        &controller,
        sg3_grant("grant-sg3-session-a", &scope_json, None),
        "session-a",
    );

    // Session A (the grant's subject) is authorized by read-back.
    let session_a = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
        })
        .expect("decide for session A");
    assert!(session_a.allowed, "the grant's own session is authorized");
    assert_eq!(session_a.source, crate::GrantReadBackSource::DurableGrant);

    // Session B asks for the SAME scope: read-back does NOT authorize it (the
    // grant's subject is session A), so it falls through to the denying policy.
    let session_b = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-other"),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: scope_json.clone(),
        })
        .expect("decide for session B");
    assert!(
        !session_b.allowed,
        "a grant for session A must not authorize session B for the same scope"
    );
    assert_eq!(session_b.source, crate::GrantReadBackSource::Policy);
    assert!(session_b.authorizing_grant_id.is_none());

    // A different capability profile for the SAME session is also not authorized.
    let other_profile = controller
        .decide_with_grant_read_back(PermissionRequest {
            session_id: SessionId::new("session-sg3"),
            capability_profile_id: "some-other-profile".to_string(),
            scope_json,
        })
        .expect("decide for other profile");
    assert!(
        !other_profile.allowed,
        "a grant for one profile must not authorize a different profile"
    );
    assert_eq!(other_profile.source, crate::GrantReadBackSource::Policy);
}

/// SG3 review-fix (CORRECTNESS: revocation is sticky). After a grant is revoked,
/// a re-request that re-derives the SAME deterministic grant id under a permissive
/// policy does NOT silently un-revoke it: the durable row keeps its `revoked_at`,
/// so read-back still treats it as absent. This is what makes the revoked-then-
/// re-requested-under-permissive-policy case (untestable while read-back was
/// inert) safe now that read-back is the live gate.
#[test]
fn sg3_re_grant_after_revoke_does_not_un_revoke_under_permissive_policy() {
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
    use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest};

    let scripted = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("sg3-resticky"));
    // TrustedLocal allows the tool, so a re-request WOULD re-create the grant if
    // the create path did not preserve the prior revocation.
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open_with_permission_policy_and_adapter(
        ProjectId::new("project-capo"),
        tmp_state_root.to_path_buf(),
        PermissionPolicy::allow_trusted_local(),
        scripted,
    )
    .expect("open real controller");
    let registration = controller
        .register_agent("sg3-resticky-worker")
        .expect("agent");
    let refs = controller
        .send_task(&registration, "Drive a re-grant-after-revoke flow")
        .expect("send task");

    // First dispatch creates a durable grant (TrustedLocal => until_session_end).
    // capo.agent_status is a read tool TrustedLocal allows.
    let make_request = |tool_call_id: &str| CapoToolRequest {
        tool_call_id: ToolCallId::new(tool_call_id),
        session_id: refs.session_id.clone(),
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
    };
    let first = controller
        .dispatch_tool_call(
            &ToolDispatchScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: TurnId::new("turn-sg3-resticky-1"),
                tool_call_id: ToolCallId::new("tool-sg3-resticky-1"),
            },
            ToolExposureRequest::Capo(make_request("tool-sg3-resticky-1")),
        )
        .expect("first dispatch");
    assert!(first.decide.grant_created, "first allow creates a grant");
    let grant_id = first.decide.capability_grant_id.clone();

    // Revoke it.
    controller
        .revoke_capability_grant(
            &crate::GrantRevocationScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: TurnId::new("turn-sg3-resticky-revoke"),
            },
            &grant_id,
            "operator revoked",
        )
        .expect("revoke");
    let revoked_at = controller
        .state()
        .capability_grant_by_id(&grant_id)
        .expect("grant by id")
        .expect("grant present")
        .revoked_at
        .expect("revoked_at stamped");

    // Re-request the SAME tool under the still-permissive policy. The decision
    // re-derives the SAME deterministic grant id; the create path MUST NOT clear
    // the prior `revoked_at`.
    controller
        .dispatch_tool_call(
            &ToolDispatchScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: TurnId::new("turn-sg3-resticky-2"),
                tool_call_id: ToolCallId::new("tool-sg3-resticky-2"),
            },
            ToolExposureRequest::Capo(make_request("tool-sg3-resticky-2")),
        )
        .expect("second dispatch");

    let after = controller
        .state()
        .capability_grant_by_id(&grant_id)
        .expect("grant by id")
        .expect("grant present");
    assert!(
        after.is_revoked(),
        "a re-request after revoke must not silently un-revoke the grant"
    );
    assert_eq!(
        after.revoked_at.as_deref(),
        Some(revoked_at.as_str()),
        "the prior revocation timestamp is preserved verbatim"
    );
}

// ----- SG5: single-writer workspace lock / session-scoped write lease --------

/// Build a workspace-lease scope for a given session/run over a shared workspace
/// root, so two scopes built with the SAME `workspace_root` contend for the same
/// single-writer lease.
fn sg5_lease_scope(session: &str, run: &str, workspace_root: &str) -> crate::WorkspaceLeaseScope {
    crate::WorkspaceLeaseScope {
        task_id: TaskId::new("task-sg5"),
        agent_id: AgentId::new(format!("agent-{session}")),
        session_id: SessionId::new(session.to_string()),
        run_id: RunId::new(run.to_string()),
        turn_id: TurnId::new(format!("turn-{session}")),
        workspace_root: workspace_root.to_string(),
    }
}

/// SG5 contention: one holder takes the write lease, a second session's write is
/// REJECTED with a typed conflict (never interleaved), the holder releases, and
/// the second session's write then succeeds.
#[test]
fn sg5_workspace_lock_rejects_second_writer_until_holder_releases() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let workspace = "/workspace/capo";
    let holder = sg5_lease_scope("session-holder", "run-holder", workspace);
    let challenger = sg5_lease_scope("session-challenger", "run-challenger", workspace);

    // Holder acquires the lease.
    let acquired = controller
        .acquire_workspace_write_lease(&holder)
        .expect("acquire");
    assert!(acquired.may_write(), "holder acquires the free lease");
    assert!(matches!(
        acquired,
        crate::WorkspaceWriteLeaseOutcome::Acquired { .. }
    ));

    // A second session's write is rejected with a typed conflict, not queued.
    let denied = controller
        .gate_workspace_write(&challenger, true)
        .expect("gate challenger write");
    assert!(!denied.allowed(), "second concurrent writer is rejected");
    let conflict = denied.conflict().expect("conflict surfaced");
    assert_eq!(
        conflict.held_by_session_id,
        SessionId::new("session-holder")
    );
    assert!(
        conflict.agent_message().contains("single-writer"),
        "conflict carries an agent-readable single-writer message"
    );

    // The challenger acquiring directly also conflicts (no silent interleave).
    let direct = controller
        .acquire_workspace_write_lease(&challenger)
        .expect("challenger acquire");
    assert!(direct.conflict().is_some(), "direct acquire also conflicts");

    // The holder releases the lease with a reason.
    let released = controller
        .release_workspace_write_lease(&holder, "turn complete")
        .expect("release");
    assert!(released.may_write());

    // Now the second session's write succeeds.
    let now_allowed = controller
        .gate_workspace_write(&challenger, true)
        .expect("gate after release");
    assert!(
        now_allowed.allowed(),
        "after release the second writer succeeds"
    );
    let holder_now = controller
        .workspace_lease_holder(&challenger)
        .expect("holder lookup")
        .expect("lease held");
    assert_eq!(
        holder_now.holder_session_id,
        SessionId::new("session-challenger"),
        "the challenger now holds the lease"
    );
}

/// SG5: read-only tools/reads are NOT blocked by the write lease. While the
/// holder owns the write lease, another session's READ is allowed and the
/// holder's own read is allowed too.
#[test]
fn sg5_reads_are_not_blocked_by_the_write_lease() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let workspace = "/workspace/capo";
    let holder = sg5_lease_scope("session-holder", "run-holder", workspace);
    let reader = sg5_lease_scope("session-reader", "run-reader", workspace);

    controller
        .acquire_workspace_write_lease(&holder)
        .expect("acquire");

    // Another session's read passes through even though the holder owns the lease.
    let other_read = controller
        .gate_workspace_write(&reader, false)
        .expect("gate other read");
    assert_eq!(other_read, crate::WorkspaceWriteGate::ReadAllowed);
    assert!(other_read.allowed());

    // The holder's own read is allowed too.
    let holder_read = controller
        .gate_workspace_write(&holder, false)
        .expect("gate holder read");
    assert_eq!(holder_read, crate::WorkspaceWriteGate::ReadAllowed);

    // The reader gating a read never took the lease away from the holder.
    let still_held = controller
        .workspace_lease_holder(&holder)
        .expect("holder lookup")
        .expect("still held");
    assert_eq!(
        still_held.holder_session_id,
        SessionId::new("session-holder")
    );
}

/// SG5 restart/replay: the lease state (held, then released, then re-acquired by
/// another session) rebuilds IDENTICALLY from the event log -- a rebuild yields
/// the same single-writer holder, and the lock still rejects a stale contender.
#[test]
fn sg5_lease_state_rebuilds_from_the_event_log() {
    let root = temp_root();
    let workspace = "/workspace/capo";
    let holder = sg5_lease_scope("session-holder", "run-holder", workspace);
    let challenger = sg5_lease_scope("session-challenger", "run-challenger", workspace);

    {
        let controller =
            RealBoundaryController::open(ProjectId::new("project-capo"), &root).expect("open");
        // Holder acquires, releases; challenger then acquires and keeps the lease.
        controller
            .acquire_workspace_write_lease(&holder)
            .expect("acquire");
        controller
            .release_workspace_write_lease(&holder, "turn complete")
            .expect("release");
        controller
            .acquire_workspace_write_lease(&challenger)
            .expect("challenger acquire");
    }

    // Reopen over the same state root and rebuild projections purely from events.
    let reopened =
        RealBoundaryController::open(ProjectId::new("project-capo"), &root).expect("reopen");
    let before_rebuild = reopened
        .state()
        .workspace_leases(&ProjectId::new("project-capo"))
        .expect("leases before");
    reopened
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    let after_rebuild = reopened
        .state()
        .workspace_leases(&ProjectId::new("project-capo"))
        .expect("leases after");
    assert_eq!(
        before_rebuild, after_rebuild,
        "lease projection rebuilds identically from the event log"
    );

    // The rebuilt lease names the challenger as the single live holder.
    let live: Vec<_> = after_rebuild.iter().filter(|l| l.is_held()).collect();
    assert_eq!(live.len(), 1, "exactly one live lease after rebuild");
    assert_eq!(
        live[0].holder_session_id,
        SessionId::new("session-challenger")
    );

    // The lock still enforces single-writer after the rebuild: the holder that
    // released earlier is now a stale contender and is rejected.
    let stale = reopened
        .gate_workspace_write(&holder, true)
        .expect("gate stale writer");
    assert!(
        !stale.allowed(),
        "after rebuild the lock still rejects a second writer"
    );
    assert_eq!(
        stale.conflict().expect("conflict").held_by_session_id,
        SessionId::new("session-challenger")
    );
}

/// SG5: re-acquiring the lease you already hold is idempotent (no new event, no
/// conflict), so a holder writing twice in a turn does not spuriously deny
/// itself.
#[test]
fn sg5_holder_re_acquire_is_idempotent() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let holder = sg5_lease_scope("session-holder", "run-holder", "/workspace/capo");

    let first = controller
        .acquire_workspace_write_lease(&holder)
        .expect("first acquire");
    assert!(matches!(
        first,
        crate::WorkspaceWriteLeaseOutcome::Acquired { .. }
    ));
    let events_after_first = controller.state().event_count().expect("count");

    let second = controller
        .acquire_workspace_write_lease(&holder)
        .expect("second acquire");
    assert!(matches!(
        second,
        crate::WorkspaceWriteLeaseOutcome::AlreadyHeldBySelf { .. }
    ));
    assert!(second.may_write(), "the holder may still write");
    assert_eq!(
        controller.state().event_count().expect("count"),
        events_after_first,
        "re-acquiring an already-held lease emits no new event"
    );
}

/// SG5 review-fix regression: a session releasing its OWN lease and then
/// re-acquiring it must actually re-hold the lease and append a NEW event.
///
/// This is the same-session acquire -> release -> re-acquire path. Before the
/// fix it was silently deduped by the event idempotency layer (the acquire
/// event id was deterministic per (lease, session)), so the re-acquire returned
/// `Acquired` while the projection stayed `released` -- a phantom acquire that
/// let a second session also "acquire" the workspace, breaking single-writer.
#[test]
fn sg5_same_session_reacquire_after_release_re_holds() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let holder = sg5_lease_scope("session-holder", "run-holder", "/workspace/capo");

    controller
        .acquire_workspace_write_lease(&holder)
        .expect("first acquire");
    controller
        .release_workspace_write_lease(&holder, "turn complete")
        .expect("release");
    let events_after_release = controller.state().event_count().expect("count");

    // Re-acquire with the SAME session after the release.
    let reacquired = controller
        .acquire_workspace_write_lease(&holder)
        .expect("re-acquire");
    assert!(
        matches!(
            reacquired,
            crate::WorkspaceWriteLeaseOutcome::Acquired { .. }
        ),
        "the re-acquire takes the freed lease afresh"
    );

    // The lease projection now reads HELD by the re-acquiring session...
    let held = controller
        .workspace_lease_holder(&holder)
        .expect("holder lookup")
        .expect("lease re-held after re-acquire");
    assert_eq!(held.holder_session_id, SessionId::new("session-holder"));

    // ...and a NEW `workspace.lease_acquired` event was appended (not deduped).
    assert!(
        controller.state().event_count().expect("count") > events_after_release,
        "the re-acquire appends a new event rather than being silently deduped"
    );

    // A second session is now correctly blocked, proving single-writer holds.
    let challenger = sg5_lease_scope("session-challenger", "run-challenger", "/workspace/capo");
    let denied = controller
        .gate_workspace_write(&challenger, true)
        .expect("gate challenger");
    assert!(
        !denied.allowed(),
        "after a same-session re-acquire a second writer is still rejected"
    );
}

/// SG5 review-fix: releasing a lease held by a DIFFERENT session is rejected
/// with a typed conflict and leaves the original holder in place (a session
/// cannot steal the single-writer lock by releasing someone else's lease).
#[test]
fn sg5_cross_session_release_conflicts_and_leaves_holder() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let holder = sg5_lease_scope("session-holder", "run-holder", "/workspace/capo");
    let other = sg5_lease_scope("session-other", "run-other", "/workspace/capo");

    controller
        .acquire_workspace_write_lease(&holder)
        .expect("acquire");
    let events_after_acquire = controller.state().event_count().expect("count");

    // A different session trying to release the holder's lease conflicts.
    let release = controller
        .release_workspace_write_lease(&other, "not my lease")
        .expect("cross-session release");
    let conflict = release
        .conflict()
        .expect("cross-session release is a conflict");
    assert_eq!(
        conflict.held_by_session_id,
        SessionId::new("session-holder")
    );

    // No event was appended, and the original holder still holds the lease.
    assert_eq!(
        controller.state().event_count().expect("count"),
        events_after_acquire,
        "a rejected cross-session release emits no event"
    );
    let still_held = controller
        .workspace_lease_holder(&holder)
        .expect("holder lookup")
        .expect("still held by original holder");
    assert_eq!(
        still_held.holder_session_id,
        SessionId::new("session-holder")
    );
}

/// SG5 review-fix: releasing a free/never-acquired lease is a no-op that emits
/// no event.
#[test]
fn sg5_release_of_free_lease_is_a_no_op() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    let scope = sg5_lease_scope("session-holder", "run-holder", "/workspace/capo");
    let events_before = controller.state().event_count().expect("count");

    let release = controller
        .release_workspace_write_lease(&scope, "nothing held")
        .expect("release of free lease");
    // Modeled as `AlreadyHeldBySelf` (no conflict, nothing to release).
    assert!(
        release.conflict().is_none(),
        "no-op release is not a conflict"
    );
    assert_eq!(
        controller.state().event_count().expect("count"),
        events_before,
        "releasing a free/never-acquired lease emits no event"
    );
    assert!(
        controller
            .workspace_lease_holder(&scope)
            .expect("holder lookup")
            .is_none(),
        "the lease is still free after a no-op release"
    );
}

/// SG5 review-fix: two genuinely DISTINCT workspace roots get INDEPENDENT
/// leases (a write under root B is not blocked by a held lease on root A), and
/// the SAME root spelled differently (trailing slash, `.`/`..`) shares ONE
/// lease. This proves the lease key is keyed on a collision-free encoding of the
/// normalized path, not the lossy slug that collapsed `/srv/a/b` and `/srv/ab`.
#[test]
fn sg5_distinct_workspace_roots_get_independent_leases() {
    let tmp_state_root = temp_root();
    let controller = RealBoundaryController::open(ProjectId::new("project-capo"), &tmp_state_root)
        .expect("open");
    // Roots that the old separator-stripping slug collapsed to one key.
    let root_a = sg5_lease_scope("session-a", "run-a", "/srv/a/b");
    let root_b = sg5_lease_scope("session-b", "run-b", "/srv/ab");

    controller
        .acquire_workspace_write_lease(&root_a)
        .expect("acquire A");
    // A write under the unrelated root B is NOT blocked by A's held lease.
    let gate_b = controller
        .gate_workspace_write(&root_b, true)
        .expect("gate B");
    assert!(
        gate_b.allowed(),
        "a held lease on /srv/a/b must not block a writer on /srv/ab"
    );
    assert!(
        controller
            .workspace_lease_holder(&root_b)
            .expect("holder B")
            .is_some(),
        "root B now holds its own independent lease"
    );

    // The SAME root spelled differently shares ONE lease: a different session
    // requesting `/srv/a/b/` (trailing slash) and `/srv/x/../a/b` contends with
    // the holder of `/srv/a/b`.
    for alias in ["/srv/a/b/", "/srv/x/../a/b", "/srv/./a/b"] {
        let aliased = sg5_lease_scope("session-alias", "run-alias", alias);
        let conflict = controller
            .gate_workspace_write(&aliased, true)
            .expect("gate alias");
        assert!(
            !conflict.allowed(),
            "alias {alias} normalizes to the same lease and must conflict with the holder"
        );
        assert_eq!(
            conflict.conflict().expect("conflict").held_by_session_id,
            SessionId::new("session-a"),
            "alias {alias} conflicts with the /srv/a/b holder"
        );
    }
}

// ---------------------------------------------------------------------------
// SG9: liveness-aware restart recovery
// ---------------------------------------------------------------------------

/// SG9 helper: seed an active-looking (`running`) in-flight run for `session`/
/// `run`, plus a `run.started` in-flight marker (the `runtime.start_requested`
/// shape the live path persists before waiting) carrying `external_pid`,
/// `boot_id`, and `runtime_process_ref`. This is exactly what
/// `inflight_runs_for_project` reads on restart.
fn sg9_seed_inflight_run(
    controller: &FakeBoundaryController,
    project_id: &ProjectId,
    session: &str,
    run: &str,
    external_pid: Option<u32>,
    boot_id: Option<&str>,
    runtime_process_ref: Option<&str>,
) {
    let session_id = SessionId::new(session.to_string());
    let run_id = RunId::new(run.to_string());
    let agent_id = AgentId::new(format!("agent-{session}"));

    // The session row (the inflight query joins runs to sessions on project_id).
    controller
        .state()
        .append_event(
            NewEvent {
                event_id: format!("event-sg9-session-{session}"),
                kind: EventKind::SessionStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some(format!("sg9-session:{session}")),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Session(SessionProjection {
                session_id: session_id.clone(),
                project_id: project_id.clone(),
                task_id: None,
                agent_id: agent_id.clone(),
                title: format!("SG9 {session}"),
                status: "running".to_string(),
                current_goal: "recoverable run".to_string(),
                latest_summary: None,
                latest_confidence: None,
                latest_blocker: None,
                external_session_ref: None,
                updated_sequence: 0,
            })],
        )
        .expect("seed session");

    // The in-flight marker: a `run.started` carrying the persisted pid/boot/ref,
    // leaving the run `running` (the state a crash interrupts).
    let payload = serde_json::json!({
        "status": "running",
        "external_pid": external_pid,
        "boot_id": boot_id,
        "runtime_process_ref": runtime_process_ref,
        "marker": "start_requested_inflight",
    })
    .to_string();
    controller
        .state()
        .append_event(
            NewEvent {
                event_id: format!("event-sg9-run-started-{run}"),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(agent_id),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some(format!("turn-{run}")),
                item_id: runtime_process_ref.map(ToString::to_string),
                payload_json: payload,
                idempotency_key: Some(format!("sg9-run-started:{run}")),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id,
                session_id,
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("seed run.started inflight marker");
}

/// SG9 helper: collect the recovery event kinds emitted for a single run, in
/// order.
fn sg9_recovery_event_kinds(controller: &FakeBoundaryController, session: &str) -> Vec<String> {
    controller
        .state()
        .recent_events_for_session(&SessionId::new(session.to_string()), 64)
        .expect("events")
        .into_iter()
        .filter(|event| event.actor == "capo-recovery")
        .map(|event| event.kind)
        .collect()
}

/// SG9: a run that terminated while Capo was down (no live process) classifies as
/// `Exited` -- a terminal `run.exited` then `run.recovered` -- NOT the blunt
/// `exited_unknown` the old path stamped on every live-looking run.
#[test]
fn sg9_gone_run_classifies_exited_not_exited_unknown() {
    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");
    // A run with NO persisted pid (a deterministic/mock run that crashed before
    // spawning) has nothing live to reattach -> Exited.
    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-gone",
        "run-gone",
        None,
        None,
        None,
    );

    let recovered = controller
        .recover_inflight_runs("recovery-sg9-gone")
        .expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].status, "recovered",
        "a gone run is reconciled to the terminal `recovered` status (matching the \
         durable run.exited -> run.recovered sequence), never `exited_unknown`"
    );

    let kinds = sg9_recovery_event_kinds(&controller, "session-gone");
    assert_eq!(
        kinds,
        vec!["run.exited".to_string(), "run.recovered".to_string()],
        "exited classification emits run.exited then run.recovered (no exited_unknown)"
    );
    // The run is reconciled (terminal recovery metadata = `recovered`), and is no
    // longer marked with the blunt `exited_unknown` status the old path stamped.
    let final_status = controller
        .state()
        .run(&RunId::new("run-gone"))
        .expect("run")
        .expect("present")
        .status;
    assert_eq!(
        final_status, "recovered",
        "the run is reconciled, not exited_unknown"
    );
    assert_ne!(
        final_status, "exited_unknown",
        "SG9 never stamps the blunt exited_unknown status"
    );
}

/// SG9: a still-alive run WITH an attachable runtime handle classifies as
/// `Reattached` -- a single `run.recovered` that reattaches in place WITHOUT
/// killing the live process (distinct from the RTL10 reaper, which kills).
#[cfg(unix)]
#[test]
fn sg9_alive_run_with_handle_reattaches_in_place_without_killing() {
    use capo_runtime::{LocalProcessConfig, LocalProcessRequest, LocalProcessRunner};
    use std::collections::HashMap;

    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");

    // Spawn a real, still-running process group to stand in for the live run.
    let workspace = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    let marker = workspace.join("reattach-survivor.txt");
    let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
        workspace.clone(),
        workspace.join("artifacts"),
    ));
    let running = runner
        .spawn_process(LocalProcessRequest {
            run_id: RunId::new("run-alive"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec![
                "-c".to_string(),
                format!("(sleep 2; printf survived > {}) &", marker.display()),
            ],
            cwd: workspace.to_path_buf(),
            env: HashMap::new(),
        })
        .expect("spawn live group");
    let pid = running.process.external_pid.expect("pid recorded");
    let boot = running.process.boot_id.clone();
    // The backgrounded descendant keeps the process group alive; the parent shell
    // exits on its own. We do not own the `Child` here (the runtime does), so we
    // just give the parent a moment to exit while the group stays live.
    drop(running);
    std::thread::sleep(std::time::Duration::from_millis(150));

    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-alive",
        "run-alive",
        Some(pid),
        boot.as_deref(),
        Some("fake-runtime-process-codex"),
    );

    let recovered = controller
        .recover_inflight_runs("recovery-sg9-alive")
        .expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].status, "recovered",
        "a still-alive attachable run is recovered (reattached) in place"
    );
    assert!(
        recovered[0].recovery_of_run_id.is_none(),
        "a reattach keeps the SAME run -- it is NOT a relaunch with recovery_of_run_id"
    );

    let kinds = sg9_recovery_event_kinds(&controller, "session-alive");
    assert_eq!(
        kinds,
        vec!["run.recovered".to_string()],
        "reattach emits ONLY run.recovered -- no run.exited, the process keeps running"
    );

    // Reattach-vs-relaunch encoding: the run.recovered payload pins the reattach
    // (reattached:true + the reattached observation kind). A regression that
    // dropped this flag or stamped a recovery_of_run_id would be caught here.
    let recovered_event = controller
        .state()
        .recent_events_for_session(&SessionId::new("session-alive"), 64)
        .expect("events")
        .into_iter()
        .find(|event| event.actor == "capo-recovery" && event.kind == "run.recovered")
        .expect("run.recovered event");
    assert!(
        recovered_event.payload_json.contains("\"reattached\":true"),
        "the reattach payload must carry reattached:true: {}",
        recovered_event.payload_json
    );
    assert!(
        recovered_event
            .payload_json
            .contains("\"recovery_observation_kind\":\"reattached\""),
        "the reattach payload records the reattached observation kind: {}",
        recovered_event.payload_json
    );

    // The reattach did NOT signal the live group: its descendant survives.
    std::thread::sleep(std::time::Duration::from_millis(2200));
    assert!(
        marker.exists(),
        "reattach must leave the live process running (no kill)"
    );
}

/// SG9: a still-alive run with NO attachable handle classifies as `Orphaned` --
/// `run.orphaned` then terminal `run.exited` then `run.recovered`.
#[cfg(unix)]
#[test]
fn sg9_alive_run_without_handle_classifies_orphaned() {
    use capo_runtime::{LocalProcessConfig, LocalProcessRequest, LocalProcessRunner};
    use std::collections::HashMap;

    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");

    let workspace = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
        workspace.clone(),
        workspace.join("artifacts"),
    ));
    let running = runner
        .spawn_process(LocalProcessRequest {
            run_id: RunId::new("run-orphan"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "(sleep 3) &".to_string()],
            cwd: workspace.to_path_buf(),
            env: HashMap::new(),
        })
        .expect("spawn live group");
    let pid = running.process.external_pid.expect("pid recorded");
    let boot = running.process.boot_id.clone();
    drop(running);
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Live pid, but NO runtime_process_ref persisted -> unowned orphan.
    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-orphan",
        "run-orphan",
        Some(pid),
        boot.as_deref(),
        None,
    );

    let recovered = controller
        .recover_inflight_runs("recovery-sg9-orphan")
        .expect("recover");
    assert_eq!(recovered.len(), 1);
    // The orphan emits run.orphaned -> run.exited -> run.recovered, so the
    // reconciled run ends `recovered`. The RETURN value must agree with the
    // durable (last-write-wins) projection, not the transient `orphaned` status.
    assert_eq!(
        recovered[0].status, "recovered",
        "the orphan is reconciled to the terminal `recovered` status"
    );
    assert!(
        recovered[0].recovery_of_run_id.is_none(),
        "an orphan reconciles the SAME run in place -- not a relaunch with recovery_of_run_id"
    );

    let kinds = sg9_recovery_event_kinds(&controller, "session-orphan");
    assert_eq!(
        kinds,
        vec![
            "run.orphaned".to_string(),
            "run.exited".to_string(),
            "run.recovered".to_string(),
        ],
        "an unowned live orphan is recorded orphaned, then exited, then recovered"
    );

    // The durable projection ends `recovered`, and rebuilding from the event log
    // reproduces the SAME terminal status (replay parity for the orphan path).
    let durable = controller
        .state()
        .run(&RunId::new("run-orphan"))
        .expect("run")
        .expect("present");
    assert_eq!(
        durable.status, "recovered",
        "the durable run projection ends `recovered`, agreeing with the return value"
    );
    controller.state().rebuild_projections().expect("rebuild");
    assert_eq!(
        controller
            .state()
            .run(&RunId::new("run-orphan"))
            .expect("run")
            .expect("present")
            .status,
        "recovered",
        "the orphan status rebuilds identically from the event log"
    );

    // Reattach-vs-relaunch encoding: the orphan's run.recovered does NOT claim a
    // reattach (the live process was NOT reattached -- it was recorded orphaned).
    let recovered_event = controller
        .state()
        .recent_events_for_session(&SessionId::new("session-orphan"), 64)
        .expect("events")
        .into_iter()
        .find(|event| event.actor == "capo-recovery" && event.kind == "run.recovered")
        .expect("run.recovered event");
    assert!(
        recovered_event
            .payload_json
            .contains("\"reattached\":false"),
        "an orphan must NOT be flagged reattached: {}",
        recovered_event.payload_json
    );
    assert!(
        recovered_event
            .payload_json
            .contains("\"recovery_observation_kind\":\"orphaned\""),
        "the orphan recovery records the orphaned observation kind: {}",
        recovered_event.payload_json
    );

    // Reap the live group so the test process tree does not leak.
    LocalProcessRunner::reap_orphan_process_group(pid, boot.as_deref());
}

/// SG9: restart recovery is IDEMPOTENT -- a repeated restart that observes the
/// same runtime state appends NO second recovery event (keyed on
/// `(run_id, recovery_observation_kind, observed_runtime_state_hash)`,
/// intentionally excluding the recovery attempt id).
#[test]
fn sg9_repeated_recovery_is_idempotent() {
    let project = ProjectId::new("project-capo");
    let state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &state_root).expect("open controller");
    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-idem",
        "run-idem",
        None,
        None,
        None,
    );

    // Build the SAME observation a restart would, and feed it to the state-layer
    // recovery TWICE with DIFFERENT attempt ids: the idempotency key
    // `(run_id, observation_kind, observed_runtime_state_hash)` excludes the
    // attempt id, so the second pass over the same observation appends nothing.
    let observation = RunRecoveryObservation {
        run_id: RunId::new("run-idem"),
        session_id: SessionId::new("session-idem"),
        previous_status: "running".to_string(),
        kind: RunRecoveryKind::Exited,
        external_pid: None,
        runtime_process_ref: None,
        observed_runtime_state_hash: "fnv1a64:deadbeefdeadbeef".to_string(),
    };

    let first = controller
        .state()
        .recover_inflight_runs(
            &project,
            "recovery-attempt-1",
            std::slice::from_ref(&observation),
        )
        .expect("first recover");
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].status, "recovered",
        "the reconciled run ends `recovered` (matching the durable run.exited -> \
         run.recovered sequence)"
    );
    let after_first = sg9_recovery_event_kinds(&controller, "session-idem");
    assert_eq!(
        after_first,
        vec!["run.exited".to_string(), "run.recovered".to_string()]
    );
    let watermark_after_first = controller.state().last_sequence().expect("seq");

    // A second restart with a DIFFERENT attempt id observes the same runtime
    // state -> no new events.
    controller
        .state()
        .recover_inflight_runs(&project, "recovery-attempt-2", &[observation])
        .expect("second recover");
    let after_second = sg9_recovery_event_kinds(&controller, "session-idem");
    assert_eq!(
        after_second, after_first,
        "a repeated restart appends no second recovery event"
    );
    assert_eq!(
        controller.state().last_sequence().expect("seq"),
        watermark_after_first,
        "the event log did not grow on the repeated recovery"
    );

    // The controller-level sweep is also naturally idempotent: after recovery the
    // run is no longer active-looking, so a second sweep reconciles nothing.
    let sweep = controller
        .recover_inflight_runs("recovery-attempt-3")
        .expect("third recover");
    assert!(
        sweep.is_empty(),
        "a reconciled run is no longer active-looking, so a repeat sweep is a no-op"
    );

    // And it rebuilds identically from the event log (replay parity).
    controller.state().rebuild_projections().expect("rebuild");
    assert_eq!(
        controller
            .state()
            .run(&RunId::new("run-idem"))
            .expect("run")
            .expect("present")
            .status,
        "recovered",
        "reconciled run status rebuilds identically from the event log"
    );
}

/// SG9: a single-writer workspace lease (SG5) held by a DEAD run is RECLAIMED
/// during recovery, so a dead holder no longer blocks the next writer; a lease
/// held by a still-alive (reattached) run is left untouched.
#[test]
fn sg9_recovery_reclaims_stale_lease_from_dead_holder() {
    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        RealBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");
    let workspace = "/workspace/capo";

    // A run takes the write lease, then crashes (it is gone on restart).
    let dead_holder = sg9_lease_scope("session-dead", "run-dead", workspace);
    let acquired = controller
        .acquire_workspace_write_lease(&dead_holder)
        .expect("acquire");
    assert!(acquired.may_write());

    // The dead run's lease is reclaimed.
    let reclaimed = controller
        .reclaim_stale_workspace_leases(&[RunId::new("run-dead")], "dead holder")
        .expect("reclaim");
    assert_eq!(reclaimed.len(), 1, "the dead holder's lease is reclaimed");
    assert!(
        controller
            .workspace_lease_holder(&dead_holder)
            .expect("holder lookup")
            .is_none(),
        "the reclaimed lease is now free"
    );

    // A NEW writer can now take the lease.
    let next = sg9_lease_scope("session-next", "run-next", workspace);
    let now = controller
        .acquire_workspace_write_lease(&next)
        .expect("next acquire");
    assert!(now.may_write(), "the next writer acquires the freed lease");

    // A lease whose holder is NOT in the dead set is left untouched.
    let reclaimed_again = controller
        .reclaim_stale_workspace_leases(&[RunId::new("run-dead")], "dead holder")
        .expect("reclaim again");
    assert!(
        reclaimed_again.is_empty(),
        "a live holder's lease is not reclaimed, and the reclaim is idempotent"
    );
    assert_eq!(
        controller
            .workspace_lease_holder(&next)
            .expect("holder")
            .expect("held")
            .holder_session_id,
        SessionId::new("session-next"),
        "the live next-writer keeps its lease"
    );
}

/// SG9 regression: the SAME workspace lease, reclaimed for dead holder A,
/// re-acquired by holder B, and reclaimed again, must end FREE -- the reclaim
/// idempotency key discriminates on the dead holder's run id, so the second
/// reclaim is NOT swallowed by `append_event`'s dedupe (which would otherwise
/// strand the single-writer lock permanently).
#[test]
fn sg9_re_reclaim_of_a_re_acquired_lease_frees_the_lock() {
    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        RealBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");
    let workspace = "/workspace/capo";

    // Holder A acquires the lease, then crashes; recovery reclaims it.
    let holder_a = sg9_lease_scope("session-a", "run-a", workspace);
    assert!(
        controller
            .acquire_workspace_write_lease(&holder_a)
            .expect("A acquire")
            .may_write()
    );
    let reclaimed_a = controller
        .reclaim_stale_workspace_leases(&[RunId::new("run-a")], "A crashed")
        .expect("reclaim A");
    assert_eq!(reclaimed_a.len(), 1, "A's lease is reclaimed");

    // Holder B acquires the SAME workspace (same lease row, reused in place), then
    // crashes too.
    let holder_b = sg9_lease_scope("session-b", "run-b", workspace);
    assert!(
        controller
            .acquire_workspace_write_lease(&holder_b)
            .expect("B acquire")
            .may_write(),
        "B can acquire the lease A freed"
    );

    // Recovery reclaims B's lease. With a key keyed only on the (reused)
    // workspace_lease_id this would collide with A's reclaim and append_event would
    // early-return -- leaving B's lease HELD forever. Keyed on the dead holder's
    // run id, B's reclaim is a distinct event and the lease is actually released.
    let reclaimed_b = controller
        .reclaim_stale_workspace_leases(&[RunId::new("run-b")], "B crashed")
        .expect("reclaim B");
    assert_eq!(
        reclaimed_b.len(),
        1,
        "B's lease must be reclaimed even though the same lease was reclaimed for A"
    );
    assert!(
        controller
            .workspace_lease_holder(&holder_b)
            .expect("holder lookup")
            .is_none(),
        "the re-acquired-then-re-died lease must end FREE, not stranded held"
    );

    // A fresh writer can take the freed lease.
    let holder_c = sg9_lease_scope("session-c", "run-c", workspace);
    assert!(
        controller
            .acquire_workspace_write_lease(&holder_c)
            .expect("C acquire")
            .may_write(),
        "the next writer acquires the freed lease"
    );

    // Re-reclaiming the SAME dead holder B is still idempotent (no new event).
    let reclaimed_b_again = controller
        .reclaim_stale_workspace_leases(&[RunId::new("run-b")], "B crashed")
        .expect("reclaim B again");
    assert!(
        reclaimed_b_again.is_empty(),
        "a repeated pass over the same dead holder reclaims nothing further"
    );
}

/// SG9: full recovery sweep wires lease reclaim into the run classification --
/// a gone run's lease is reclaimed by `recover_inflight_runs`.
#[test]
fn sg9_recover_inflight_runs_reclaims_lease_of_gone_run() {
    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");

    // A gone in-flight run that also held the workspace lease.
    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-locked",
        "run-locked",
        None,
        None,
        None,
    );
    let holder = sg9_lease_scope("session-locked", "run-locked", "/workspace/capo");
    controller
        .acquire_workspace_write_lease(&holder)
        .expect("acquire");
    assert!(
        controller
            .workspace_lease_holder(&holder)
            .expect("holder")
            .is_some(),
        "the run holds the lease before recovery"
    );

    controller
        .recover_inflight_runs("recovery-sg9-locked")
        .expect("recover");

    assert!(
        controller
            .workspace_lease_holder(&holder)
            .expect("holder")
            .is_none(),
        "recovery reclaimed the dead run's stale lease"
    );
}

/// SG9: an ORPHANED run is still ALIVE (its process group keeps running), so the
/// recovery sweep must NOT reclaim its single-writer workspace lease -- freeing it
/// would let a new writer race the live orphan and break the SG5 single-writer
/// invariant. Only a confirmed-terminal (exited) run's lease is reclaimed.
#[cfg(unix)]
#[test]
fn sg9_recover_inflight_runs_keeps_lease_of_live_orphan() {
    use capo_runtime::{LocalProcessConfig, LocalProcessRequest, LocalProcessRunner};
    use std::collections::HashMap;

    let project = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project.clone(), &tmp_state_root).expect("open controller");

    // Spawn a real, still-running process group (a live orphan with NO attachable
    // runtime_process_ref).
    let workspace = temp_root();
    std::fs::create_dir_all(&workspace).expect("workspace");
    let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
        workspace.clone(),
        workspace.join("artifacts"),
    ));
    let running = runner
        .spawn_process(LocalProcessRequest {
            run_id: RunId::new("run-live-orphan"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "(sleep 3) &".to_string()],
            cwd: workspace.to_path_buf(),
            env: HashMap::new(),
        })
        .expect("spawn live group");
    let pid = running.process.external_pid.expect("pid recorded");
    let boot = running.process.boot_id.clone();
    drop(running);
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Live pid, no runtime_process_ref -> classified Orphaned, but still alive.
    sg9_seed_inflight_run(
        &controller,
        &project,
        "session-live-orphan",
        "run-live-orphan",
        Some(pid),
        boot.as_deref(),
        None,
    );
    let holder = sg9_lease_scope("session-live-orphan", "run-live-orphan", "/workspace/capo");
    controller
        .acquire_workspace_write_lease(&holder)
        .expect("acquire");

    controller
        .recover_inflight_runs("recovery-sg9-live-orphan")
        .expect("recover");

    // The live orphan KEEPS its lease: it is still running, so reclaiming would be
    // unsafe.
    assert!(
        controller
            .workspace_lease_holder(&holder)
            .expect("holder")
            .is_some(),
        "a still-alive orphan must keep its single-writer lease (not reclaimed)"
    );

    // Reap the live group so the test process tree does not leak.
    LocalProcessRunner::reap_orphan_process_group(pid, boot.as_deref());
}

fn sg9_lease_scope(session: &str, run: &str, workspace_root: &str) -> crate::WorkspaceLeaseScope {
    crate::WorkspaceLeaseScope {
        task_id: TaskId::new("task-sg9"),
        agent_id: AgentId::new(format!("agent-{session}")),
        session_id: SessionId::new(session.to_string()),
        run_id: RunId::new(run.to_string()),
        turn_id: TurnId::new(format!("turn-{session}")),
        workspace_root: workspace_root.to_string(),
    }
}

/// SG10: the deterministic, hermetic safety suite.
///
/// SG1-SG9 each landed focused per-feature tests. SG10 is the consolidated
/// acceptance suite the SG0 invariant requires: every state-changing safety
/// behavior has at least one deterministic assertion (event/wire snapshot, exit
/// status, or replay), and there are NO live providers anywhere -- every test
/// uses the in-process controller, scripted/fake adapters, scripted shell
/// commands, and seeded/durable event-sourced state, so the suite is fully
/// reproducible.
///
/// The suite has two layers:
///
/// 1. One test per enumerated safety behavior (the SG10 acceptance list): denied
///    request, granted request, revoked grant denied on re-request, expired grant
///    denied, critical-scope denial under TrustedLocal, verification pass,
///    verification fail, workspace-lock contention, and checkpoint rollback
///    restoring prior state.
/// 2. A consolidated restart/replay test proving grant lifecycle
///    (created/revoked/expired), lock leases, checkpoint refs, score outcomes, and
///    recovery classifications ALL rebuild identically from the event log after a
///    store reopen + `rebuild_projections`.
#[cfg(test)]
mod sg10 {
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_runtime::LocalProcessConfig;

    use super::*;
    use crate::{
        AcceptanceCriterion, CheckpointScope, GrantReadBackSource, GrantRevocationScope,
        RunScoreOutcome, RunScoreScope, VerificationCommand, VerificationKind, VerificationScope,
        WorkspaceLeaseScope, WorkspaceWriteGate,
    };

    const PROJECT: &str = "project-capo";
    const PROFILE: &str = "trusted-local-dev";
    const WRITE_SCOPE: &str = "[\"filesystem:write:workspace\"]";

    fn project() -> ProjectId {
        ProjectId::new(PROJECT)
    }

    /// A static-policy controller bundled with the temp dir its store lives in,
    /// so the on-disk DB survives for the whole test. Derefs to the controller.
    struct StaticController {
        controller: FakeBoundaryController,
        _state: capo_tmptest::TempRoot,
    }

    impl std::ops::Deref for StaticController {
        type Target = FakeBoundaryController;

        fn deref(&self) -> &FakeBoundaryController {
            &self.controller
        }
    }

    /// A controller over the read-only-local STATIC policy, which DENIES a
    /// workspace write by default -- the policy used to prove deny / grant
    /// authorization / revoke / expiry without relying on the permissive default.
    ///
    /// Drives the [`FakeBoundaryController`] orchestration core directly: it is the
    /// explicit deterministic test seam (the `Real*` handle is a thin production
    /// wrapper over this same core, persisting through the identical
    /// `append_event`/projection path), and it exposes every SG1-SG9 safety method
    /// (`decide_with_grant_read_back`, `revoke_capability_grant`, the lock,
    /// checkpoint, `run_verification`, `score_run`, `recover_inflight_runs`) on one
    /// handle.
    fn static_controller() -> StaticController {
        let state = temp_root();
        let controller = FakeBoundaryController::open_with_permission_policy(
            project(),
            &state,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("open static controller");
        StaticController {
            controller,
            _state: state,
        }
    }

    fn write_request(session: &str) -> PermissionRequest {
        PermissionRequest {
            session_id: SessionId::new(session.to_string()),
            capability_profile_id: PROFILE.to_string(),
            scope_json: WRITE_SCOPE.to_string(),
        }
    }

    /// Seed a durable ALLOW grant for the write scope, subject-scoped to `session`,
    /// optionally with an `expires_at` (epoch-millis as a string).
    fn seed_write_grant(
        controller: &FakeBoundaryController,
        grant_id: &str,
        session: &str,
        expires_at: Option<&str>,
    ) {
        controller
            .state()
            .append_event(
                capo_state::NewEvent::new(
                    format!("event-sg10-grant-{grant_id}"),
                    capo_state::EventKind::CapabilityGrantCreated,
                    "test",
                ),
                &[capo_state::ProjectionRecord::CapabilityGrant(
                    capo_state::CapabilityGrantProjection {
                        capability_grant_id: grant_id.to_string(),
                        capability_profile_id: PROFILE.to_string(),
                        scope_json: WRITE_SCOPE.to_string(),
                        effect: "allow".to_string(),
                        subject_json: format!("{{\"session_id\":\"{session}\"}}"),
                        decision_source: "user".to_string(),
                        persistence: "until_revoked".to_string(),
                        explanation: "operator-reviewed grant".to_string(),
                        created_at: Some("1700000000000".to_string()),
                        expires_at: expires_at.map(str::to_string),
                        revoked_at: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("seed grant");
    }

    fn now_millis_string() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis()
            .to_string()
    }

    // ---- enumerated behavior: denied request ----------------------------------

    /// A request the policy denies (and that no grant authorizes) is DENIED: the
    /// decide step surfaces a typed deny via the grant read-back gate, naming the
    /// policy (not a grant) as the authority.
    #[test]
    fn sg10_denied_request_is_blocked() {
        let controller = static_controller();
        let decision = controller
            .decide_with_grant_read_back(write_request("session-denied"))
            .expect("decide");
        assert!(
            !decision.allowed,
            "an un-granted policy-denied write is denied"
        );
        assert_eq!(decision.source, GrantReadBackSource::Policy);
        assert!(decision.authorizing_grant_id.is_none());
        assert_eq!(decision.policy_decision.effect, "deny");
    }

    // ---- enumerated behavior: granted request ---------------------------------

    /// A valid durable ALLOW grant authorizes the SAME request the policy would
    /// deny -- grants are read back as authorization, not write-only.
    #[test]
    fn sg10_granted_request_is_authorized_via_read_back() {
        let controller = static_controller();
        seed_write_grant(&controller, "grant-sg10-allow", "session-granted", None);
        let decision = controller
            .decide_with_grant_read_back(write_request("session-granted"))
            .expect("decide");
        assert!(
            decision.allowed,
            "a valid durable grant authorizes the request"
        );
        assert_eq!(decision.source, GrantReadBackSource::DurableGrant);
        assert_eq!(
            decision.authorizing_grant_id.as_deref(),
            Some("grant-sg10-allow")
        );
        // The grant, not the policy, authorized: the policy still records a deny.
        assert_eq!(decision.policy_decision.effect, "deny");
    }

    // ---- enumerated behavior: revoked grant denied on re-request --------------

    /// After a grant is revoked, re-requesting the same scope is DENIED (the
    /// revoked grant reads as absent), while the original grant-created event is
    /// preserved and exactly one `capability.grant_revoked` event is added.
    #[test]
    fn sg10_revoked_grant_denied_on_re_request() {
        let controller = static_controller();
        let registration = controller.register_agent("sg10-revoke").expect("agent");
        let refs = controller
            .send_task(&registration, "Drive an SG10 revoke")
            .expect("send task");
        seed_write_grant(&controller, "grant-sg10-revoke", "session-revoke", None);

        // Authorized while valid.
        let granted = controller
            .decide_with_grant_read_back(write_request("session-revoke"))
            .expect("decide");
        assert!(granted.allowed);
        let events_before = controller.state().event_count().expect("count");

        // Revoke with a reason.
        let revoke_scope = GrantRevocationScope {
            task_id: refs.task_id.clone(),
            agent_id: refs.agent_id.clone(),
            session_id: refs.session_id.clone(),
            run_id: refs.run_id.clone(),
            turn_id: TurnId::new("turn-sg10-revoke"),
        };
        controller
            .revoke_capability_grant(&revoke_scope, "grant-sg10-revoke", "policy tightened")
            .expect("revoke");

        // Re-request is now denied.
        let after = controller
            .decide_with_grant_read_back(write_request("session-revoke"))
            .expect("decide after revoke");
        assert!(!after.allowed, "a revoked grant no longer authorizes");
        assert_eq!(after.source, GrantReadBackSource::Policy);

        // Old created event preserved; revoke ADDS exactly one event.
        let events_after = controller.state().event_count().expect("count");
        assert_eq!(events_after, events_before + 1, "revoke adds one event");
        let grant = controller
            .state()
            .capability_grant_by_id("grant-sg10-revoke")
            .expect("grant by id")
            .expect("present");
        assert!(grant.is_revoked());
    }

    // ---- enumerated behavior: expired grant denied ----------------------------

    /// A grant past its `expires_at` does NOT authorize, even though it was never
    /// explicitly revoked (expiry is a denial input in decide).
    #[test]
    fn sg10_expired_grant_denied() {
        let controller = static_controller();
        // `expires_at = 1` is far in the past relative to the wall clock.
        seed_write_grant(
            &controller,
            "grant-sg10-expired",
            "session-expired",
            Some("1"),
        );
        let decision = controller
            .decide_with_grant_read_back(write_request("session-expired"))
            .expect("decide");
        assert!(!decision.allowed, "an expired grant does not authorize");
        assert_eq!(decision.source, GrantReadBackSource::Policy);
        let grant = controller
            .state()
            .capability_grant_by_id("grant-sg10-expired")
            .expect("grant by id")
            .expect("present");
        assert!(
            !grant.is_revoked(),
            "the grant was never explicitly revoked"
        );
        assert!(grant.is_expired(&now_millis_string()));
    }

    // ---- enumerated behavior: critical-scope denial under TrustedLocal --------

    /// Under the DEFAULT TrustedLocal policy (the controller default), an
    /// un-granted CRITICAL scope is DENIED, while an ordinary non-critical
    /// workspace write still ALLOWS -- the SG4 critical-scope fix, asserted at the
    /// policy boundary used by the loop.
    #[test]
    fn sg10_critical_scope_denied_under_trusted_local() {
        let policy = PermissionPolicy::allow_trusted_local();

        // Each enumerated critical scope is denied.
        for scope in [
            "filesystem:write:path",
            "network:connect:internet",
            "network:expose:public",
            "secret:read:credential_material",
            "shell:execute:path",
        ] {
            let decision = policy.decide(PermissionRequest {
                session_id: SessionId::new("session-critical"),
                capability_profile_id: PROFILE.to_string(),
                scope_json: format!("[\"{scope}\"]"),
            });
            assert_eq!(
                decision.effect, "deny",
                "TrustedLocal must deny un-granted critical scope `{scope}`"
            );
            assert_eq!(decision.decision_source, "allow_trusted_local_profile");
        }

        // A non-critical workspace write still allows (audit-only allow intact).
        let allowed = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-critical"),
            capability_profile_id: PROFILE.to_string(),
            scope_json: WRITE_SCOPE.to_string(),
        });
        assert_eq!(
            allowed.effect, "allow",
            "ordinary workspace write still allows under TrustedLocal"
        );
    }

    // ---- enumerated behavior: verification pass / fail ------------------------

    fn verification_scope() -> VerificationScope {
        VerificationScope {
            task_id: TaskId::new("task-sg10"),
            agent_id: AgentId::new("agent-sg10"),
            session_id: SessionId::new("session-sg10-verify"),
            run_id: RunId::new("run-sg10-verify"),
            turn_id: TurnId::new("turn-sg10-verify"),
        }
    }

    fn shell_command(kind: VerificationKind, script: &str, cwd: &Path) -> VerificationCommand {
        VerificationCommand::new(
            kind,
            "/bin/sh",
            vec!["-c".to_string(), script.to_string()],
            cwd.to_path_buf(),
        )
    }

    /// A scripted command that exits 0 is classified PASSED, keyed off the real
    /// exit status, recorded as OBSERVED evidence.
    #[test]
    fn sg10_verification_pass_from_exit_status() {
        let tmp_state_root = temp_root();
        let controller =
            FakeBoundaryController::open(project(), &tmp_state_root).expect("controller");
        let workspace = temp_root();
        std::fs::create_dir_all(&workspace).expect("workspace");
        let artifacts = temp_root();
        let outcome = controller
            .run_verification(
                &verification_scope(),
                LocalProcessConfig::for_test(workspace.clone(), artifacts.to_path_buf()),
                &shell_command(VerificationKind::Test, "printf ok; exit 0", &workspace),
            )
            .expect("run verification");
        assert!(outcome.passed, "exit 0 classifies passed");
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.evidence_kind, "test");
    }

    /// A scripted command that exits non-zero is classified FAILED, keyed off the
    /// real exit status.
    #[test]
    fn sg10_verification_fail_from_exit_status() {
        let tmp_state_root = temp_root();
        let controller =
            FakeBoundaryController::open(project(), &tmp_state_root).expect("controller");
        let workspace = temp_root();
        std::fs::create_dir_all(&workspace).expect("workspace");
        let artifacts = temp_root();
        let outcome = controller
            .run_verification(
                &verification_scope(),
                LocalProcessConfig::for_test(workspace.clone(), artifacts.to_path_buf()),
                &shell_command(
                    VerificationKind::Check,
                    "printf boom >&2; exit 7",
                    &workspace,
                ),
            )
            .expect("run verification");
        assert!(!outcome.passed, "a non-zero exit classifies failed");
        assert_eq!(outcome.exit_code, Some(7));
    }

    // ---- enumerated behavior: workspace-lock contention -----------------------

    fn lease_scope(session: &str, run: &str, workspace_root: &str) -> WorkspaceLeaseScope {
        WorkspaceLeaseScope {
            task_id: TaskId::new("task-sg10"),
            agent_id: AgentId::new(format!("agent-{session}")),
            session_id: SessionId::new(session.to_string()),
            run_id: RunId::new(run.to_string()),
            turn_id: TurnId::new(format!("turn-{session}")),
            workspace_root: workspace_root.to_string(),
        }
    }

    /// Single-writer contention: one holder acquires the lease, a second writer is
    /// REJECTED with a typed conflict (never interleaved), and only after the
    /// holder releases does the second writer succeed.
    #[test]
    fn sg10_workspace_lock_contention_rejects_second_writer() {
        let tmp_state_root = temp_root();
        let controller =
            FakeBoundaryController::open(project(), &tmp_state_root).expect("controller");
        let workspace = "/w/sg10-lock";
        let holder = lease_scope("session-holder", "run-holder", workspace);
        let contender = lease_scope("session-contender", "run-contender", workspace);

        // Holder takes the write lease.
        match controller
            .gate_workspace_write(&holder, true)
            .expect("gate holder write")
        {
            WorkspaceWriteGate::WriteAllowed { .. } => {}
            other => panic!("holder write must be allowed, got {other:?}"),
        }

        // A second session's write over the SAME workspace is rejected.
        match controller
            .gate_workspace_write(&contender, true)
            .expect("gate contender write")
        {
            WorkspaceWriteGate::WriteDenied(conflict) => {
                assert!(
                    !conflict.agent_message().is_empty(),
                    "conflict carries an agent-readable message"
                );
            }
            other => panic!("second writer must be rejected, got {other:?}"),
        }

        // A read is NEVER blocked by the write lease.
        assert!(matches!(
            controller
                .gate_workspace_write(&contender, false)
                .expect("gate read"),
            WorkspaceWriteGate::ReadAllowed
        ));

        // Holder releases; the contender then succeeds.
        controller
            .release_workspace_write_lease(&holder, "turn complete")
            .expect("release");
        match controller
            .gate_workspace_write(&contender, true)
            .expect("gate contender write after release")
        {
            WorkspaceWriteGate::WriteAllowed { .. } => {}
            other => panic!("contender must succeed after release, got {other:?}"),
        }
    }

    // ---- enumerated behavior: checkpoint rollback restoring prior state -------

    fn checkpoint_scope(workspace: &Path, shadow_root: &Path, turn: &str) -> CheckpointScope {
        CheckpointScope {
            task_id: TaskId::new("task-sg10"),
            agent_id: AgentId::new("agent-sg10"),
            session_id: SessionId::new("session-sg10-cp"),
            run_id: RunId::new("run-sg10-cp"),
            turn_id: TurnId::new(turn),
            workspace_root: workspace.display().to_string(),
            shadow_git_root: shadow_root.display().to_string(),
        }
    }

    /// Checkpoint -> write -> restore returns the workspace byte-for-byte to the
    /// checkpointed state (revert a modified file, restore a deleted file, remove
    /// a file added after the checkpoint).
    #[test]
    fn sg10_checkpoint_rollback_restores_prior_state() {
        let tmp_state_root = temp_root();
        let controller =
            FakeBoundaryController::open(project(), &tmp_state_root).expect("controller");
        let workspace = temp_root();
        let shadow = temp_root();
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::write(workspace.join("keep.txt"), "original\n").expect("write keep");
        std::fs::write(workspace.join("edit.txt"), "before\n").expect("write edit");

        let scope = checkpoint_scope(&workspace, &shadow, "turn-1");
        let created = controller
            .create_checkpoint(&scope)
            .expect("create io")
            .expect("create ok");
        assert!(
            !created.commit_ref.is_empty(),
            "checkpoint records a commit ref"
        );

        // Real write after the checkpoint.
        std::fs::write(workspace.join("edit.txt"), "after\n").expect("modify");
        std::fs::write(workspace.join("added.txt"), "new\n").expect("add");
        std::fs::remove_file(workspace.join("keep.txt")).expect("delete");

        // One Restore command reverses all of it.
        controller
            .restore_checkpoint(&scope, &created.checkpoint_id)
            .expect("restore io")
            .expect("restore ok");
        assert_eq!(
            std::fs::read_to_string(workspace.join("edit.txt")).expect("read edit"),
            "before\n",
            "modified file reverted"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("keep.txt")).expect("read keep"),
            "original\n",
            "deleted file restored"
        );
        assert!(
            !workspace.join("added.txt").exists(),
            "file added after the checkpoint is removed by restore"
        );
    }

    // ---- consolidated restart/replay ------------------------------------------

    /// SG10 acceptance: grant lifecycle, lock leases, checkpoint refs, score
    /// outcomes, and recovery classifications ALL rebuild identically from the
    /// event log. One controller drives every safety behavior, then the store is
    /// reopened from disk (a restart), `rebuild_projections` replays the log, and
    /// each projection is asserted byte-identical before vs after the rebuild.
    #[test]
    fn sg10_full_lifecycle_rebuilds_identically_from_event_log() {
        let state_root = temp_root();
        let controller = FakeBoundaryController::open_with_permission_policy(
            project(),
            &state_root,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("open controller");
        let registration = controller.register_agent("sg10-replay").expect("agent");
        let refs = controller
            .send_task(&registration, "Drive the full SG10 lifecycle")
            .expect("send task");

        // --- grant lifecycle: created, revoked, expired ---
        seed_write_grant(&controller, "grant-sg10-live", "session-replay", None);
        seed_write_grant(&controller, "grant-sg10-rev", "session-replay-rev", None);
        seed_write_grant(
            &controller,
            "grant-sg10-exp",
            "session-replay-exp",
            Some("1"),
        );
        controller
            .revoke_capability_grant(
                &GrantRevocationScope {
                    task_id: refs.task_id.clone(),
                    agent_id: refs.agent_id.clone(),
                    session_id: refs.session_id.clone(),
                    run_id: refs.run_id.clone(),
                    turn_id: TurnId::new("turn-sg10-replay-revoke"),
                },
                "grant-sg10-rev",
                "replay revoke",
            )
            .expect("revoke");

        // --- lock lease: acquire then release ---
        let lease = lease_scope("session-replay", "run-replay", "/w/sg10-replay");
        controller
            .acquire_workspace_write_lease(&lease)
            .expect("acquire lease");
        controller
            .release_workspace_write_lease(&lease, "replay release")
            .expect("release lease");

        // --- checkpoint ref ---
        let workspace = temp_root();
        let shadow = temp_root();
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::write(workspace.join("f.txt"), "v1\n").expect("write");
        let cp_scope = checkpoint_scope(&workspace, &shadow, "turn-replay");
        let created = controller
            .create_checkpoint(&cp_scope)
            .expect("create io")
            .expect("create ok");

        // --- score outcome over OBSERVED evidence ---
        let verify_scope = VerificationScope {
            task_id: refs.task_id.clone(),
            agent_id: refs.agent_id.clone(),
            session_id: refs.session_id.clone(),
            run_id: refs.run_id.clone(),
            turn_id: TurnId::new("turn-sg10-replay-verify"),
        };
        controller
            .run_verification(
                &verify_scope,
                LocalProcessConfig::for_test(workspace.clone(), workspace.join("artifacts")),
                &shell_command(VerificationKind::Test, "exit 0", &workspace),
            )
            .expect("run verification");
        let score = controller
            .score_run(
                &RunScoreScope {
                    task_id: refs.task_id.clone(),
                    agent_id: refs.agent_id.clone(),
                    session_id: refs.session_id.clone(),
                    run_id: refs.run_id.clone(),
                    turn_id: TurnId::new("turn-sg10-replay-score"),
                    started_at: 1_700_000_000_000,
                    completed_at: 1_700_000_002_500,
                },
                &[AcceptanceCriterion::new(
                    "tests pass",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");
        assert_eq!(score.outcome, RunScoreOutcome::Passed);
        assert_eq!(
            score.duration_millis, 2_500,
            "wall-clock duration, not event delta"
        );

        // --- recovery classification: a gone in-flight run exits, not exited_unknown ---
        sg9_seed_inflight_run(
            &controller,
            &project(),
            "session-sg10-recover",
            "run-sg10-recover",
            None,
            None,
            None,
        );
        controller
            .recover_inflight_runs("recovery-sg10-replay")
            .expect("recover");

        // Snapshot every safety projection BEFORE the restart. The lease is read
        // back via the public `workspace_lease_holder` (the lease key is private),
        // which reads the durable projection.
        let state = controller.state();
        let grants_before = state.capability_grants().expect("grants");
        let lease_before = controller.workspace_lease_holder(&lease).expect("lease");
        let checkpoint_before = state
            .checkpoint_by_id(&created.checkpoint_id)
            .expect("checkpoint");
        let score_before = state
            .run_score_by_id(&score.projection.run_score_id)
            .expect("score");
        let recovered_run_before = state.run(&RunId::new("run-sg10-recover")).expect("run");

        // Restart: drop nothing in the way -- reopen a fresh controller over the
        // SAME on-disk state root (a restart) and replay the log into the rebuilt
        // projections via the reopened handle.
        let reopened = FakeBoundaryController::open_with_permission_policy(
            project(),
            &state_root,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("reopen controller");
        let reopened_state = reopened.state();
        reopened_state.rebuild_projections().expect("rebuild");

        // Every projection rebuilds identically.
        assert_eq!(
            grants_before,
            reopened_state.capability_grants().expect("grants after"),
            "grant lifecycle (created/revoked/expired) rebuilds identically"
        );
        assert_eq!(
            lease_before,
            reopened
                .workspace_lease_holder(&lease)
                .expect("lease after"),
            "lock lease rebuilds identically"
        );
        assert_eq!(
            checkpoint_before,
            reopened_state
                .checkpoint_by_id(&created.checkpoint_id)
                .expect("checkpoint after"),
            "checkpoint ref rebuilds identically"
        );
        assert_eq!(
            score_before,
            reopened_state
                .run_score_by_id(&score.projection.run_score_id)
                .expect("score after"),
            "score outcome rebuilds identically"
        );
        assert_eq!(
            recovered_run_before,
            reopened_state
                .run(&RunId::new("run-sg10-recover"))
                .expect("run after"),
            "recovery classification rebuilds identically"
        );

        // Sanity: the revoked grant reads revoked, the expired grant reads expired,
        // the recovered run is reconciled (never the blunt `exited_unknown`).
        let after = reopened_state.capability_grants().expect("grants");
        assert!(
            after
                .iter()
                .any(|g| g.capability_grant_id == "grant-sg10-rev" && g.is_revoked()),
            "revoked grant survives replay as revoked"
        );
        assert!(
            after
                .iter()
                .any(|g| g.capability_grant_id == "grant-sg10-exp"
                    && g.is_expired(&now_millis_string())),
            "expired grant survives replay as expired"
        );
        let recovered_status = reopened_state
            .run(&RunId::new("run-sg10-recover"))
            .expect("run")
            .expect("present")
            .status;
        assert_ne!(
            recovered_status, "exited_unknown",
            "recovery never stamps the blunt exited_unknown status"
        );
    }
}

/// SG11: the end-to-end safety gate plus the live opt-in safety smoke.
///
/// The workpad-wide acceptance+verification invariant (`knowledge.md`, SG0) is
/// that no task completes on operator self-attestation alone: every manual smoke
/// is paired with a deterministic assertion of the SAME shape. SG11 honours that
/// with two tests that share ONE end-to-end driver
/// ([`drive_gated_write_lifecycle`]):
///
/// 1. [`sg11_safety_e2e_gate_covers_permission_verification_and_rollback`] --
///    always runs (no live provider, no env mutation). It is the deterministic
///    E2E gate: it exercises permission DENY then ALLOW (via the SG3 grant
///    read-back), the SG6 [`FakeBoundaryController::run_verification`] (real
///    exit-status evidence, both a pass and a fail), and SG8 checkpoint/rollback
///    together on one controller over one real on-disk workspace + system git,
///    then computes the SG7 `score_run` over the OBSERVED evidence and proves the
///    whole lifecycle rebuilds identically from the event log after a restart.
///
/// 2. [`live_safety_smoke_is_paired_with_a_deterministic_assertion`] --
///    `#[ignore]`d AND behind the explicit opt-in env gate [`LIVE_SAFETY_ENV`]
///    (mirroring the `CAPO_SERVER_RUN_CODEX_LIVE` convention; it also skips
///    cleanly, passing, when unset or when system git is unavailable), so it
///    never runs in ordinary test runs. It drives ONE real gated write through
///    the SAME [`drive_gated_write_lifecycle`] path the gate uses -- permission
///    decided, checkpoint taken, write performed under the workspace lock,
///    verification run, `score_run` computed -- where the real shell command
///    emits a credential-shaped secret on stdout, and asserts the persisted
///    artifact is REDACTED (`RedactionState::Redacted`, the raw secret absent,
///    the credential placeholder present). The live evidence is thus a true
///    pairing with the deterministic gate and is never operator-attested.
mod sg11 {
    use std::path::{Path, PathBuf};

    use capo_runtime::{CREDENTIAL_REDACTION_PLACEHOLDER, LocalProcessConfig};
    use capo_state::RedactionState;

    use super::*;
    use crate::{
        AcceptanceCriterion, CheckpointScope, GrantReadBackSource, RunScoreOutcome, RunScoreScope,
        VerificationCommand, VerificationKind, VerificationScope, WorkspaceLeaseScope,
        WorkspaceWriteGate,
    };

    const PROJECT: &str = "project-capo";
    const PROFILE: &str = "trusted-local-dev";
    const WRITE_SCOPE: &str = "[\"filesystem:write:workspace\"]";

    /// The explicit opt-in env gate for the live safety smoke, mirroring the
    /// `CAPO_SERVER_RUN_CODEX_LIVE` convention used by the RTL13 workspace-write
    /// smoke and the ST12 streaming smoke. The live smoke is `#[ignore]`d AND
    /// env-gated, so it never runs in ordinary test runs and never stands as the
    /// only evidence for the task.
    const LIVE_SAFETY_ENV: &str = "CAPO_SERVER_RUN_SAFETY_LIVE";

    /// A credential-shaped secret the live smoke's real command echoes to stdout.
    /// The runtime's default credential-shape scan (ACI7) recognizes the
    /// `Bearer`-prefixed token and rewrites it to
    /// [`CREDENTIAL_REDACTION_PLACEHOLDER`] before the artifact is persisted, so
    /// the raw value must never reach the persisted artifact.
    const LIVE_SMOKE_SECRET: &str = "ghp_0123456789ABCDEFabcdef0123456789ABCD";

    fn project() -> ProjectId {
        ProjectId::new(PROJECT)
    }

    fn write_request(session: &str) -> PermissionRequest {
        PermissionRequest {
            session_id: SessionId::new(session.to_string()),
            capability_profile_id: PROFILE.to_string(),
            scope_json: WRITE_SCOPE.to_string(),
        }
    }

    /// Seed a durable ALLOW grant for the write scope, subject-scoped to
    /// `session`, so the SG3 grant read-back authorizes a write the static
    /// read-only policy would otherwise deny (the "allow" leg of deny+allow).
    fn seed_write_grant(controller: &FakeBoundaryController, grant_id: &str, session: &str) {
        controller
            .state()
            .append_event(
                capo_state::NewEvent::new(
                    format!("event-sg11-grant-{grant_id}"),
                    capo_state::EventKind::CapabilityGrantCreated,
                    "test",
                ),
                &[capo_state::ProjectionRecord::CapabilityGrant(
                    capo_state::CapabilityGrantProjection {
                        capability_grant_id: grant_id.to_string(),
                        capability_profile_id: PROFILE.to_string(),
                        scope_json: WRITE_SCOPE.to_string(),
                        effect: "allow".to_string(),
                        subject_json: format!("{{\"session_id\":\"{session}\"}}"),
                        decision_source: "user".to_string(),
                        persistence: "until_revoked".to_string(),
                        explanation: "operator-reviewed grant".to_string(),
                        created_at: Some("1700000000000".to_string()),
                        expires_at: None,
                        revoked_at: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("seed grant");
    }

    fn shell_command(kind: VerificationKind, script: &str, cwd: &Path) -> VerificationCommand {
        VerificationCommand::new(
            kind,
            "/bin/sh",
            vec!["-c".to_string(), script.to_string()],
            cwd.to_path_buf(),
        )
    }

    /// The observable outcome of the end-to-end gated write, captured once and
    /// asserted by both the deterministic gate and the live smoke so they verify
    /// the IDENTICAL contract (the SG0 pairing invariant).
    struct GatedWriteOutcome {
        /// The DENY leg: the policy denied the un-granted write before the grant
        /// was seeded.
        denied_source: GrantReadBackSource,
        denied_allowed: bool,
        /// The ALLOW leg: the durable grant authorized the same write.
        allowed_source: GrantReadBackSource,
        allowed_grant_id: Option<String>,
        /// The checkpoint commit ref taken before the write.
        checkpoint_ref: String,
        checkpoint_id: String,
        /// The verification verdict, keyed strictly off the real exit status.
        verification_passed: bool,
        verification_exit_code: Option<i32>,
        /// The redacted output artifact id the observed evidence points at.
        verification_artifact_id: Option<String>,
        /// The artifact root the verification command wrote its (redacted)
        /// stdout artifact under (so the live smoke can read it back).
        artifact_root: PathBuf,
        verification_run_id: RunId,
        verification_turn_id: String,
        /// The computed score over the OBSERVED evidence.
        score_outcome: RunScoreOutcome,
        score_passed: bool,
    }

    /// Drive ONE real gated write end-to-end on `controller` over the on-disk
    /// `workspace`: permission DENY then ALLOW, checkpoint, write under the
    /// workspace lock, verification run, and `score_run`. `verify_script` is the
    /// real shell body the verification runner executes (the deterministic gate
    /// passes a trivial body; the live smoke passes one that emits a secret).
    ///
    /// This is the shared path: the deterministic gate and the live smoke both
    /// call it and assert the SAME shape, so neither stands alone.
    fn drive_gated_write_lifecycle(
        controller: &FakeBoundaryController,
        workspace: &Path,
        shadow_root: &Path,
        artifact_root: &Path,
        verify_script: &str,
    ) -> GatedWriteOutcome {
        let session = "session-sg11";
        let registration = controller.register_agent("sg11-worker").expect("agent");
        let refs = controller
            .send_task(&registration, "Drive one gated write end-to-end")
            .expect("send task");

        // --- permission: DENY before any grant exists (static read-only policy) ---
        let denied = controller
            .decide_with_grant_read_back(write_request(session))
            .expect("decide deny");

        // --- permission: ALLOW after a durable grant is seeded (SG3 read-back) ---
        seed_write_grant(controller, "grant-sg11-write", session);
        let allowed = controller
            .decide_with_grant_read_back(write_request(session))
            .expect("decide allow");

        // --- checkpoint BEFORE the write so the write is reversible (SG8) ---
        std::fs::create_dir_all(workspace).expect("workspace");
        std::fs::write(workspace.join("tracked.txt"), "before\n").expect("seed tracked");
        let cp_scope = CheckpointScope {
            task_id: refs.task_id.clone(),
            agent_id: refs.agent_id.clone(),
            session_id: refs.session_id.clone(),
            run_id: refs.run_id.clone(),
            turn_id: TurnId::new("turn-sg11-checkpoint"),
            workspace_root: workspace.display().to_string(),
            shadow_git_root: shadow_root.display().to_string(),
        };
        let checkpoint = controller
            .create_checkpoint(&cp_scope)
            .expect("create checkpoint io")
            .expect("create checkpoint ok");

        // --- the real write under the single-writer workspace lock (SG5) ---
        let lease = WorkspaceLeaseScope {
            task_id: refs.task_id.clone(),
            agent_id: refs.agent_id.clone(),
            session_id: refs.session_id.clone(),
            run_id: refs.run_id.clone(),
            turn_id: TurnId::new("turn-sg11-write"),
            workspace_root: workspace.display().to_string(),
        };
        match controller
            .gate_workspace_write(&lease, true)
            .expect("gate write")
        {
            WorkspaceWriteGate::WriteAllowed { .. } => {}
            other => panic!("the lease holder's write must be allowed, got {other:?}"),
        }
        std::fs::write(workspace.join("tracked.txt"), "after\n").expect("perform write");
        controller
            .release_workspace_write_lease(&lease, "turn complete")
            .expect("release lease");

        // --- verification: real exit-status evidence (SG6) ---
        let verification_run_id = refs.run_id.clone();
        let verification_turn_id = "turn-sg11-verify".to_string();
        let verify_scope = VerificationScope {
            task_id: refs.task_id.clone(),
            agent_id: refs.agent_id.clone(),
            session_id: refs.session_id.clone(),
            run_id: verification_run_id.clone(),
            turn_id: TurnId::new(verification_turn_id.clone()),
        };
        let verification = controller
            .run_verification(
                &verify_scope,
                LocalProcessConfig::for_test(workspace.to_path_buf(), artifact_root.to_path_buf()),
                &shell_command(VerificationKind::Test, verify_script, workspace),
            )
            .expect("run verification");

        // --- score_run over the OBSERVED evidence (SG7), controlled clock ---
        let score = controller
            .score_run(
                &RunScoreScope {
                    task_id: refs.task_id.clone(),
                    agent_id: refs.agent_id.clone(),
                    session_id: refs.session_id.clone(),
                    run_id: verification_run_id.clone(),
                    turn_id: TurnId::new("turn-sg11-score"),
                    started_at: 1_700_000_000_000,
                    completed_at: 1_700_000_001_500,
                },
                &[AcceptanceCriterion::new(
                    "tests pass",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        GatedWriteOutcome {
            denied_source: denied.source,
            denied_allowed: denied.allowed,
            allowed_source: allowed.source,
            allowed_grant_id: allowed.authorizing_grant_id.clone(),
            checkpoint_ref: checkpoint.commit_ref.clone(),
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            verification_passed: verification.passed,
            verification_exit_code: verification.exit_code,
            verification_artifact_id: verification.output_artifact_id.clone(),
            artifact_root: artifact_root.to_path_buf(),
            verification_run_id,
            verification_turn_id,
            score_outcome: score.outcome,
            score_passed: score.passed,
        }
    }

    /// The deterministic shape both the gate and the live smoke assert: permission
    /// DENY then ALLOW, a checkpoint ref, a verification PASS keyed off exit
    /// status, and a `score_run` that passed over the observed evidence.
    fn assert_gated_write_shape(outcome: &GatedWriteOutcome) {
        // DENY leg: the static read-only policy denies the un-granted write.
        assert!(
            !outcome.denied_allowed,
            "an un-granted write is denied by the policy before any grant exists"
        );
        assert_eq!(
            outcome.denied_source,
            GrantReadBackSource::Policy,
            "the deny is attributed to the policy, not a grant"
        );
        // ALLOW leg: the durable grant authorizes the same write via read-back.
        assert_eq!(
            outcome.allowed_source,
            GrantReadBackSource::DurableGrant,
            "the seeded durable grant authorizes the same write"
        );
        assert_eq!(
            outcome.allowed_grant_id.as_deref(),
            Some("grant-sg11-write"),
            "the authorizing grant is the one we seeded"
        );
        // Checkpoint taken before the write.
        assert!(
            !outcome.checkpoint_ref.is_empty(),
            "a checkpoint commit ref is recorded before the write"
        );
        // Verification: PASS keyed strictly off the real exit status.
        assert!(
            outcome.verification_passed,
            "the scripted verification command exited 0 and is classified passed"
        );
        assert_eq!(outcome.verification_exit_code, Some(0));
        // Score: passed over the observed evidence.
        assert!(outcome.score_passed, "the run scored passed");
        assert_eq!(outcome.score_outcome, RunScoreOutcome::Passed);
    }

    /// A controller over the static read-only-local policy, which DENIES a
    /// workspace write by default -- so the deny+allow legs are real, not the
    /// permissive default's blanket allow.
    fn static_controller(state_root: &Path) -> FakeBoundaryController {
        FakeBoundaryController::open_with_permission_policy(
            project(),
            state_root,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("open static controller")
    }

    /// SG11 deterministic E2E gate: permission deny+allow, the VerificationRunner
    /// (real exit-status evidence), and checkpoint/rollback together on one
    /// deterministic path, then the score over observed evidence, then a
    /// restart/replay parity check. No live providers, no env mutation.
    #[test]
    fn sg11_safety_e2e_gate_covers_permission_verification_and_rollback() {
        let state_root = temp_root();
        let workspace = temp_root();
        let shadow = temp_root();
        let artifacts = temp_root();
        let controller = static_controller(&state_root);

        let outcome = drive_gated_write_lifecycle(
            &controller,
            &workspace,
            &shadow,
            &artifacts,
            // Trivial real command: exits 0, prints a benign line.
            "printf sg11-gate-ok; exit 0",
        );
        assert_gated_write_shape(&outcome);

        // The write actually landed under the lock.
        assert_eq!(
            std::fs::read_to_string(workspace.join("tracked.txt")).expect("read tracked"),
            "after\n",
            "the gated write modified the workspace"
        );

        // --- ROLLBACK: one Restore command reverses the write (SG8) ---
        let cp_scope = CheckpointScope {
            task_id: TaskId::new("task-sg11"),
            agent_id: AgentId::new("agent-sg11"),
            session_id: SessionId::new("session-sg11"),
            run_id: RunId::new("run-sg11"),
            turn_id: TurnId::new("turn-sg11-checkpoint"),
            workspace_root: workspace.display().to_string(),
            shadow_git_root: shadow.display().to_string(),
        };
        controller
            .restore_checkpoint(&cp_scope, &outcome.checkpoint_id)
            .expect("restore io")
            .expect("restore ok");
        assert_eq!(
            std::fs::read_to_string(workspace.join("tracked.txt")).expect("read tracked"),
            "before\n",
            "the rollback restored the pre-write workspace state"
        );

        // --- the observed verification evidence is OBSERVED, not agent-reported ---
        let evidence_events = controller
            .state()
            .evidence_events_for_run(&outcome.verification_run_id)
            .expect("evidence events");
        let observed = evidence_events
            .iter()
            .find(|event| event.actor == VERIFICATION_EVIDENCE_ACTOR)
            .expect("an observed-runner verification evidence event");
        let payload: serde_json::Value =
            serde_json::from_str(&observed.payload_json).expect("payload");
        assert_eq!(payload["source"], VERIFICATION_EVIDENCE_SOURCE);
        assert_eq!(payload["passed"], serde_json::Value::Bool(true));
        assert_eq!(payload["exit_status"], "0");

        // --- restart/replay parity: the whole lifecycle rebuilds identically ---
        let grants_before = controller.state().capability_grants().expect("grants");
        let checkpoint_before = controller
            .state()
            .checkpoint_by_id(&outcome.checkpoint_id)
            .expect("checkpoint");
        let reopened = static_controller(&state_root);
        reopened.state().rebuild_projections().expect("rebuild");
        assert_eq!(
            grants_before,
            reopened.state().capability_grants().expect("grants after"),
            "the grant authorizing the write rebuilds identically after a restart"
        );
        assert_eq!(
            checkpoint_before,
            reopened
                .state()
                .checkpoint_by_id(&outcome.checkpoint_id)
                .expect("checkpoint after"),
            "the checkpoint ref rebuilds identically after a restart"
        );

        // --- a FAILING verification is still keyed off exit status (honesty) ---
        let fail_scope = VerificationScope {
            task_id: TaskId::new("task-sg11-fail"),
            agent_id: AgentId::new("agent-sg11-fail"),
            session_id: SessionId::new("session-sg11-fail"),
            run_id: RunId::new("run-sg11-fail"),
            turn_id: TurnId::new("turn-sg11-fail"),
        };
        let fail = controller
            .run_verification(
                &fail_scope,
                LocalProcessConfig::for_test(workspace.clone(), artifacts.clone()),
                &shell_command(
                    VerificationKind::Check,
                    "printf boom >&2; exit 9",
                    &workspace,
                ),
            )
            .expect("run failing verification");
        assert!(!fail.passed, "a non-zero exit is classified failed");
        assert_eq!(fail.exit_code, Some(9));
    }

    /// SG11 live opt-in safety smoke. `#[ignore]`d AND behind the explicit env
    /// gate [`LIVE_SAFETY_ENV`]; it also skips cleanly (passing) if the gate is
    /// unset or if system `git` is unavailable, so it never runs in ordinary test
    /// runs and is non-fatal when the environment cannot support it.
    ///
    /// It drives ONE real gated write through the SAME
    /// [`drive_gated_write_lifecycle`] path the deterministic gate uses, where the
    /// real shell command echoes a credential-shaped secret on stdout, and asserts
    /// the persisted artifact is REDACTED (secret stripped, placeholder present,
    /// `RedactionState::Redacted`) AND that the deterministic gate shape holds --
    /// so the live evidence is paired and never operator-attested.
    ///
    /// Run it with:
    ///   `CAPO_SERVER_RUN_SAFETY_LIVE=1 cargo test -p capo-controller \`
    ///     `sg11::live_safety_smoke_is_paired_with_a_deterministic_assertion -- --ignored`
    #[test]
    #[ignore = "live safety smoke: set CAPO_SERVER_RUN_SAFETY_LIVE=1 to run it"]
    fn live_safety_smoke_is_paired_with_a_deterministic_assertion() {
        if std::env::var(LIVE_SAFETY_ENV).as_deref() != Ok("1") {
            eprintln!("skipping live safety smoke: set {LIVE_SAFETY_ENV}=1 to run it");
            return;
        }
        // System git is required for the shadow-git checkpoint. Skip clean if it
        // is unavailable rather than failing the smoke.
        let git_ok = std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !git_ok {
            eprintln!("skipping live safety smoke: system git is unavailable");
            return;
        }

        let state_root = temp_root();
        let workspace = temp_root();
        let shadow = temp_root();
        let artifacts = temp_root();
        let controller = static_controller(&state_root);

        // The real command echoes a credential-shaped secret on stdout. The
        // runtime's ACI7 credential-shape scan must strip it from the persisted
        // artifact before it is recorded.
        let verify_script = format!("printf 'token: Bearer {LIVE_SMOKE_SECRET}\\n'; exit 0");
        let outcome = drive_gated_write_lifecycle(
            &controller,
            &workspace,
            &shadow,
            &artifacts,
            &verify_script,
        );

        // Paired deterministic assertion: the SAME shape the gate proves.
        assert_gated_write_shape(&outcome);

        // SECRETS STRIPPED: read the persisted (redacted) stdout artifact back and
        // prove the raw secret never landed while the credential placeholder did.
        let artifact_id = outcome
            .verification_artifact_id
            .as_deref()
            .expect("verification recorded a stdout artifact ref");
        let stdout_path = outcome
            .artifact_root
            .join(outcome.verification_run_id.as_str())
            .join("turns")
            .join(sanitize_artifact_key(&outcome.verification_turn_id))
            .join("stdout.txt");
        let persisted = std::fs::read_to_string(&stdout_path).unwrap_or_else(|e| {
            panic!("read persisted stdout artifact {stdout_path:?} ({artifact_id}): {e}")
        });
        assert!(
            !persisted.contains(LIVE_SMOKE_SECRET),
            "the raw credential must never reach the persisted artifact; got {persisted:?}"
        );
        assert!(
            persisted.contains(CREDENTIAL_REDACTION_PLACEHOLDER),
            "the persisted artifact must carry the credential redaction placeholder; got {persisted:?}"
        );

        // The artifact the runtime persisted is a `RedactionState::Redacted`
        // artifact: it is the persistable classification, and the redaction we
        // just proved on the bytes is exactly what that state guarantees. A leaked
        // secret would be neither redacted nor persistable.
        assert!(
            RedactionState::Redacted.is_persistable_artifact(),
            "the live smoke persists only redacted artifacts"
        );
    }

    /// Mirror of `capo_runtime`'s `sanitize_artifact_key` so the smoke can locate
    /// the persisted stdout artifact for the verification turn. The runtime keys
    /// the turn directory by this sanitized form (`run_dir_for` -> `turns/<key>`):
    /// only ASCII alphanumerics, `-`, and `_` survive; everything else maps to
    /// `_`; an empty result becomes `"turn"`.
    fn sanitize_artifact_key(key: &str) -> String {
        let sanitized: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if sanitized.is_empty() {
            "turn".to_string()
        } else {
            sanitized
        }
    }
}

// --------------------------------------------------------------------------
// DP2 (acp-replay-dedupe.md): the end-to-end ACP replay/reconciliation
// producer path. These drive the `AcpReplayEngine` plan through the controller
// ingest seam (`ingest_acp_replay_plan`) so the 8 `adapter.attach_*` /
// `adapter.replay_*` event kinds and the 3 read-model projections are produced
// by a REAL producer (not dead code), event-sourced into capo-state, and
// proven to rebuild identically on restart.
// --------------------------------------------------------------------------

fn dp2_controller_with_session(
    label: &str,
) -> (FakeBoundaryController, FakeRunRefs, capo_tmptest::TempRoot) {
    let root = temp_root();
    let controller =
        FakeBoundaryController::open(ProjectId::new("project-capo"), &root).expect("controller");
    let registration = controller
        .register_agent(&format!("acp-{label}"))
        .expect("register agent");
    let refs = controller
        .send_task(&registration, "Drive an ACP replay")
        .expect("send task");
    (controller, refs, root)
}

fn acp_update(session: &str, body: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session, "update": body }
    })
}

#[test]
fn dp2_session_resume_attach_adds_no_items_but_records_attach_batch() {
    // Fixture 2: a `session/resume` attach emits attach_started -> attach_completed
    // with NO message/item replay events; the read-model session item count is
    // unchanged, and the batch records source=session_resume_attach.
    use capo_adapters::AcpReplayEngine;
    let (controller, refs, _state) = dp2_controller_with_session("resume");

    let tools_before = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools")
        .len();

    let plan = AcpReplayEngine::plan_resume_attach(
        "acp-ext-resume",
        &serde_json::json!({ "resumed": true }),
    );
    let report = controller
        .ingest_acp_replay_plan(&refs, &plan)
        .expect("ingest resume attach");

    assert_eq!(report.imported_count, 0, "resume imports no items");
    assert_eq!(report.duplicate_count, 0);
    assert_eq!(report.ambiguous_count, 0);

    // No new tool/item read-model rows were created by the attach.
    let tools_after = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools")
        .len();
    assert_eq!(tools_after, tools_before, "resume attach adds no items");

    // The attach batch is recorded with the resume source + completed status.
    let batches = controller
        .state()
        .adapter_replay_batches_for_session(&refs.session_id)
        .expect("batches");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].source, "session_resume_attach");
    assert_eq!(batches[0].status, "completed");

    // The attach lifecycle is event-sourced: attach_started + attach_completed.
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 200)
        .expect("events");
    assert!(
        events.iter().any(|e| e.kind == "adapter.attach_started"),
        "attach_started must be emitted"
    );
    assert!(
        events.iter().any(|e| e.kind == "adapter.attach_completed"),
        "attach_completed must be emitted"
    );
    assert!(
        !events.iter().any(|e| e.kind == "adapter.replay_started"),
        "a resume must not open a replay batch"
    );
}

#[test]
fn dp2_foreign_session_load_imports_each_item_once_then_rebuilds_identically() {
    // Fixture 4 + restart/replay invariant: a FOREIGN `session/load` (no local
    // history) imports each user/agent chunk and tool call exactly once; the raw
    // updates + timeline keys + batch are event-sourced; and a clear-and-replay
    // rebuild reconstructs every DP2 read model byte-identically.
    use capo_adapters::{AcpReplayEngine, AcpReplaySource};
    let (controller, refs, _state) = dp2_controller_with_session("foreign-load");

    let frames = vec![
        acp_update(
            "acp-ext-foreign",
            serde_json::json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": "do the task" }
            }),
        ),
        acp_update(
            "acp-ext-foreign",
            serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-foreign-1",
                "title": "write file",
                "status": "completed",
                "content": { "type": "text", "text": "done" }
            }),
        ),
    ];

    let existing = controller
        .acp_existing_item_fingerprints(&refs)
        .expect("fingerprints");
    assert!(
        existing.is_empty(),
        "a foreign session has no local history"
    );
    let plan = AcpReplayEngine::plan_load(
        AcpReplaySource::ForeignImport,
        "acp-ext-foreign",
        &frames,
        &existing,
    );
    let report = controller
        .ingest_acp_replay_plan(&refs, &plan)
        .expect("ingest foreign load");

    assert_eq!(report.raw_update_count, 2, "every raw frame persisted");
    assert_eq!(
        report.imported_count, 2,
        "import user chunk + tool call once"
    );
    assert_eq!(report.duplicate_count, 0);

    // Raw updates persisted (before normalization) and the tool imported once.
    let raw = controller
        .state()
        .adapter_raw_updates_for_batch(&report.acp_replay_batch_id)
        .expect("raw updates");
    assert_eq!(raw.len(), 2);
    let tools = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools");
    let acp_tools: Vec<_> = tools
        .iter()
        .filter(|t| t.tool_origin == "adapter_native:acp")
        .collect();
    assert_eq!(
        acp_tools.iter().filter(|t| t.status == "completed").count(),
        1,
        "the foreign tool call is inspectable exactly once"
    );

    // Restart/replay: snapshot every DP2 read model, then rebuild and compare.
    let batches_before = controller
        .state()
        .adapter_replay_batches_for_session(&refs.session_id)
        .expect("batches");
    let raw_before = controller
        .state()
        .adapter_raw_updates_for_batch(&report.acp_replay_batch_id)
        .expect("raw before");
    let keys_before = controller
        .state()
        .adapter_timeline_keys_for_session(&refs.session_id)
        .expect("keys before");
    let tools_before = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools before");

    controller.state().rebuild_projections().expect("rebuild");

    assert_eq!(
        controller
            .state()
            .adapter_replay_batches_for_session(&refs.session_id)
            .expect("batches after"),
        batches_before,
        "replay batches rebuild identically"
    );
    assert_eq!(
        controller
            .state()
            .adapter_raw_updates_for_batch(&report.acp_replay_batch_id)
            .expect("raw after"),
        raw_before,
        "raw updates rebuild identically"
    );
    assert_eq!(
        controller
            .state()
            .adapter_timeline_keys_for_session(&refs.session_id)
            .expect("keys after"),
        keys_before,
        "timeline keys rebuild identically"
    );
    assert_eq!(
        controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools after"),
        tools_before,
        "imported tool calls rebuild identically"
    );
}

#[test]
fn dp2_load_of_known_history_adds_no_duplicate_ui_items() {
    // Fixture 3: replaying the SAME `session/load` history a second time adds no
    // duplicate UI items -- the second load reconciles every candidate as a
    // duplicate observation against the timeline keys the first load recorded.
    use capo_adapters::{AcpReplayEngine, AcpReplaySource};
    let (controller, refs, _state) = dp2_controller_with_session("known-load");

    let frames = vec![
        acp_update(
            "acp-ext-known",
            serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "the answer is 42" }
            }),
        ),
        acp_update(
            "acp-ext-known",
            serde_json::json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-known-1",
                "title": "write file",
                "status": "completed",
                "content": { "type": "text", "text": "done" }
            }),
        ),
    ];

    // First load: import.
    let first = controller
        .ingest_acp_replay_plan(
            &refs,
            &AcpReplayEngine::plan_load(
                AcpReplaySource::SessionLoad,
                "acp-ext-known",
                &frames,
                &controller
                    .acp_existing_item_fingerprints(&refs)
                    .expect("fingerprints 1"),
            ),
        )
        .expect("ingest first load");
    assert_eq!(first.imported_count, 2);

    let tools_after_first = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools 1")
        .len();

    // Second load of the SAME history: now Capo has local timeline keys, so every
    // candidate reconciles as a duplicate -> no new UI items.
    let existing = controller
        .acp_existing_item_fingerprints(&refs)
        .expect("fingerprints 2");
    assert!(
        !existing.is_empty(),
        "the first load recorded local timeline keys"
    );
    let second = controller
        .ingest_acp_replay_plan(
            &refs,
            &AcpReplayEngine::plan_load(
                AcpReplaySource::SessionLoad,
                "acp-ext-known",
                &frames,
                &existing,
            ),
        )
        .expect("ingest second load");

    assert_eq!(
        second.imported_count, 0,
        "a known-history reload imports nothing"
    );
    assert_eq!(
        second.duplicate_count, 2,
        "both candidates reconcile as duplicate observations"
    );

    // The UI item count is unchanged by the second load.
    let tools_after_second = controller
        .state()
        .tool_calls_for_session(&refs.session_id)
        .expect("tools 2")
        .len();
    assert_eq!(
        tools_after_second, tools_after_first,
        "the second load adds no duplicate UI items"
    );

    // A duplicate-detected marker was event-sourced (not item events).
    let events = controller
        .state()
        .recent_events_for_session(&refs.session_id, 400)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|e| e.kind == "adapter.replay_duplicate_detected"),
        "a duplicate observation marker must be emitted"
    );
}

/// DP5: the live turn-context packet is assembled from the ELIGIBILITY-FILTERED
/// memory store (the previously-dead `packet_eligible_memory_records` gate is
/// now wired into production), and FTS5-ranked against the goal. An eligible
/// reviewed record whose subject matches the goal is retrieved into the packet;
/// an invalidated record with the same matching text is filtered out by the SQL
/// eligibility gate and never becomes a candidate. The packet is replayable: its
/// recorded artifact content hash is deterministic across rebuilds.
#[test]
fn dp5_live_packet_includes_eligible_store_record_and_excludes_invalidated() {
    let project_id = ProjectId::new("project-capo");
    let tmp_state_root = temp_root();
    let controller =
        FakeBoundaryController::open(project_id.clone(), &tmp_state_root).expect("open controller");
    let registration = controller.register_agent("fake-codex").expect("agent");

    // Seed two memory records whose bodies both match the goal terms below.
    // ONLY the reviewed+sourced record is packet-eligible; the invalidated one
    // is excluded by `packet_eligible_memory_records` before FTS ever sees it.
    let mut eligible = dp5_reviewed_record(&project_id, "memory-record-eligible");
    eligible.subject = "Sandbox confinement decision".to_string();
    eligible.body =
        "The sandbox confinement policy denies egress outside the granted scope.".to_string();
    let mut invalidated = dp5_reviewed_record(&project_id, "memory-record-invalidated");
    invalidated.subject = "Stale sandbox note".to_string();
    invalidated.body = "Old sandbox confinement policy that was superseded.".to_string();
    invalidated.review_state = "invalidated".to_string();
    invalidated.invalidated_at = Some("2026-06-01T00:00:00Z".to_string());

    controller
        .state()
        .append_event(
            NewEvent::new(
                "event-dp5-seed-memory",
                EventKind::MemoryRecordIngested,
                "dp5-test",
            ),
            &[
                ProjectionRecord::MemoryRecord(Box::new(eligible)),
                ProjectionRecord::MemorySource(capo_state::MemorySourceProjection {
                    memory_source_id: "memory-source-eligible".to_string(),
                    memory_record_id: "memory-record-eligible".to_string(),
                    source_kind: "markdown".to_string(),
                    source_event_id: None,
                    source_artifact_id: None,
                    source_path: Some("workpads/depth/knowledge.md".to_string()),
                    source_anchor: Some("DP5".to_string()),
                    source_content_hash: Some("sha256:eligible".to_string()),
                    source_sequence: Some(1),
                    quote_artifact_id: None,
                    observed_at: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::MemoryRecord(Box::new(invalidated)),
            ],
        )
        .expect("seed memory records");

    // The SQL gate already excludes the invalidated record.
    let eligible_records = controller
        .state()
        .packet_eligible_memory_records(&project_id)
        .expect("eligible records");
    assert_eq!(eligible_records.len(), 1);
    assert_eq!(
        eligible_records[0].memory_record_id,
        "memory-record-eligible"
    );

    let goal = "sandbox confinement policy egress";
    let refs = controller
        .send_task(&registration, goal)
        .expect("send task");

    let observation = controller.observe(&refs).expect("observe");
    let memory_event = observation
        .recent_events
        .iter()
        .find(|event| event.kind == "memory.packet_built")
        .expect("memory packet event");
    let seeded_included = dp5_included_count(&memory_event.payload_json);

    // Baseline: an identical controller WITHOUT the seeded eligible record runs
    // the same goal. The only difference in the candidate corpus is the eligible
    // store record, so the seeded run must include exactly one MORE retrieved
    // item -- proving the eligibility-filtered store record flowed into the
    // packet (and the invalidated record never did).
    let tmp_state_root = temp_root();
    let baseline_controller = FakeBoundaryController::open(project_id.clone(), &tmp_state_root)
        .expect("open baseline controller");
    let baseline_registration = baseline_controller
        .register_agent("fake-codex")
        .expect("baseline agent");
    let baseline_refs = baseline_controller
        .send_task(&baseline_registration, goal)
        .expect("baseline send task");
    let baseline_observation = baseline_controller
        .observe(&baseline_refs)
        .expect("observe");
    let baseline_included = dp5_included_count(
        &baseline_observation
            .recent_events
            .iter()
            .find(|event| event.kind == "memory.packet_built")
            .expect("baseline memory packet event")
            .payload_json,
    );
    assert_eq!(
        seeded_included,
        baseline_included + 1,
        "the eligible store record must add exactly one retrieved packet item"
    );

    // The packet artifact exists and carries a deterministic content hash (the
    // replayability anchor: same retrieved sources -> same packet markdown).
    let packets = controller
        .state()
        .memory_packets_for_session(&refs.session_id)
        .expect("packets");
    let artifact_id = packets[0]
        .packet_artifact_id
        .as_deref()
        .expect("packet artifact id");
    let artifact = controller
        .state()
        .artifact_by_id(artifact_id)
        .expect("artifact lookup")
        .expect("packet artifact recorded");
    assert!(!artifact.content_hash.is_empty());
}

fn dp5_included_count(payload_json: &str) -> usize {
    let marker = "\"included_count\":";
    let start = payload_json.find(marker).expect("included_count present") + marker.len();
    let rest = &payload_json[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().expect("included_count is a number")
}

fn dp5_reviewed_record(
    project_id: &ProjectId,
    memory_record_id: &str,
) -> capo_state::MemoryRecordProjection {
    capo_state::MemoryRecordProjection {
        memory_record_id: memory_record_id.to_string(),
        project_id: project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: project_id.to_string(),
        subject_ref: Some("workpads/depth/knowledge.md".to_string()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "decision".to_string(),
        subject: "Decision".to_string(),
        predicate: "is".to_string(),
        object: "recorded".to_string(),
        body: "body".to_string(),
        confidence: "high".to_string(),
        review_state: "reviewed".to_string(),
        source_count: 1,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Safe.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{memory_record_id}")),
        updated_sequence: 0,
    }
}
