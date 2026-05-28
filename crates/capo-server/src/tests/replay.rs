use super::*;

#[test]
fn server_replays_codex_fixture_through_server_boundary() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );
    let session_id = "session-codex-local-1";
    let started = handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Replay Codex fixture through the server".to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some("run-codex-local-1".to_string()),
        },
    );
    let ServerResponsePayload::SessionStarted(started) = started.payload else {
        panic!("expected session started response");
    };
    assert_eq!(started.session_id.as_str(), session_id);

    let response = server
        .handle(ServerRequest::local_cli(
            "replay-codex-through-server",
            ServerCommand::ReplayAdapterFixture {
                adapter: "codex".to_string(),
                session_id: session_id.to_string(),
                run_id: "run-codex-local-1".to_string(),
                turn_id: "turn-codex-local-1".to_string(),
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                    .to_string(),
            },
        ))
        .expect("replay codex fixture");

    let ServerResponsePayload::AdapterFixtureReplayed(replay) = response.payload else {
        panic!("expected adapter replay response");
    };
    assert_eq!(replay.adapter, "codex_exec");
    assert_eq!(replay.agent_name, "codex-local");
    assert!(!replay.provider_cli_executed);
    assert_eq!(replay.raw_content_policy, "content_hashed_not_rendered");
    assert_eq!(replay.input_event_count, 5);
    assert_eq!(replay.appended_event_count, 6);
    assert_eq!(replay.tool_event_count, 2);
    assert_eq!(replay.completed_turn_count, 1);
    assert_eq!(replay.session_id.as_str(), session_id);
    assert_eq!(replay.run_id.as_str(), "run-codex-local-1");
    assert_eq!(replay.turn_id, "turn-codex-local-1");

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    assert_eq!(dashboard.agent_count, 1);
    let agent = dashboard
        .agents
        .iter()
        .find(|agent| agent.name == "codex-local")
        .expect("codex agent");
    let session = agent.session.as_ref().expect("codex session");
    assert_eq!(session.run_status.as_deref(), Some("running"));
    assert_eq!(session.adapter_kind.as_deref(), Some("codex_exec"));
    assert_eq!(session.evidence_count, 1);
    assert!(
        session
            .evidence_refs
            .iter()
            .any(|evidence| evidence.contains("codex_exec"))
    );
    assert_eq!(session.turn_ids, vec!["turn-codex-local-1"]);
    assert_eq!(session.tool_call_count, 1);
    assert_eq!(session.tool_observation_count, 1);

    let state = SqliteStateStore::open(&root).expect("state");
    let session_events = state
        .recent_events_for_session(&replay.session_id, 20)
        .expect("session events");
    assert!(session_events.iter().any(|event| {
        event.kind == "server.request_handled"
            && event.payload_json.contains("replay_adapter_fixture")
            && event
                .payload_json
                .contains("\"provider_cli_executed\":false")
            && event
                .payload_json
                .contains("\"raw_content_policy\":\"content_hashed_not_rendered\"")
            && event
                .payload_json
                .contains("\"raw_fixture_body_persisted\":false")
            && event.payload_json.contains("\"fixture_hash\":")
    }));
    assert!(session_events.iter().any(
        |event| event.kind == "evidence.recorded" && event.payload_json.contains("codex_exec")
    ));
    assert!(
        session_events
            .iter()
            .all(|event| !event.payload_json.contains("Codex fixture response."))
    );
}

#[test]
fn server_replays_acp_fixture_into_server_native_session() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "acp-mock".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "acp-mock".to_string(),
            goal: "Replay ACP-shaped mock fixture through the server".to_string(),
            adapter: "acp".to_string(),
            session_id: Some("session-acp-mock-1".to_string()),
            run_id: Some("run-acp-mock-1".to_string()),
        },
    );

    let response = server
        .handle(ServerRequest::local_cli(
            "replay-acp-through-server",
            ServerCommand::ReplayAdapterFixture {
                adapter: "acp".to_string(),
                session_id: "session-acp-mock-1".to_string(),
                run_id: "run-acp-mock-1".to_string(),
                turn_id: "turn-acp-mock-1".to_string(),
                fixture_name: "crates/capo-adapters/fixtures/acp-replay.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/acp-replay.jsonl")
                    .to_string(),
            },
        ))
        .expect("replay acp fixture");

    let ServerResponsePayload::AdapterFixtureReplayed(replay) = response.payload else {
        panic!("expected adapter replay response");
    };
    assert_eq!(replay.adapter, "acp");
    assert_eq!(replay.agent_name, "acp-mock");
    assert_eq!(replay.session_id.as_str(), "session-acp-mock-1");
    assert!(!replay.provider_cli_executed);
    assert_eq!(replay.raw_content_policy, "content_hashed_not_rendered");
    assert_eq!(replay.input_event_count, 6);
    assert_eq!(replay.tool_event_count, 3);

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let agent = dashboard
        .agents
        .iter()
        .find(|agent| agent.name == "acp-mock")
        .expect("acp mock agent");
    let session = agent.session.as_ref().expect("acp session");
    assert_eq!(session.adapter_kind.as_deref(), Some("acp"));
    assert_eq!(session.turn_ids, vec!["turn-acp-mock-1"]);
    assert_eq!(session.tool_call_count, 1);
    assert_eq!(session.tool_observation_count, 1);
}

#[test]
fn server_rejects_adapter_replay_and_dispatch_that_mismatch_session_adapter() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Reject cross-adapter mutation".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-cross-adapter".to_string()),
            run_id: Some("run-cross-adapter".to_string()),
        },
    );

    let replay_error = server
        .handle(ServerRequest::local_cli(
            "replay-acp-into-codex-session",
            ServerCommand::ReplayAdapterFixture {
                adapter: "acp".to_string(),
                session_id: "session-cross-adapter".to_string(),
                run_id: "run-cross-adapter".to_string(),
                turn_id: "turn-cross-adapter".to_string(),
                fixture_name: "crates/capo-adapters/fixtures/acp-replay.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/acp-replay.jsonl")
                    .to_string(),
            },
        ))
        .expect_err("replay must reject adapter/session mismatch");
    assert!(matches!(
        replay_error,
        ServerError::AdapterSessionMismatch {
            session_adapter,
            requested_adapter,
            ..
        } if session_adapter == "codex_exec" && requested_adapter == "acp"
    ));

    let dispatch_error = server
        .handle(ServerRequest::local_cli(
            "plan-acp-into-codex-session",
            ServerCommand::PlanDispatch {
                agent_name: "codex-local".to_string(),
                adapter: "acp".to_string(),
                goal: "Reject cross-adapter dispatch".to_string(),
                workspace: "/tmp/capo-workspace".to_string(),
                artifacts: "/tmp/capo-artifacts".to_string(),
                session_id: "session-cross-adapter".to_string(),
                run_id: "run-cross-adapter".to_string(),
                turn_id: "turn-cross-adapter".to_string(),
                deterministic_opt_in: true,
            },
        ))
        .expect_err("dispatch planning must reject adapter/session mismatch");
    assert!(matches!(
        dispatch_error,
        ServerError::AdapterSessionMismatch {
            session_adapter,
            requested_adapter,
            ..
        } if session_adapter == "codex_exec" && requested_adapter == "acp"
    ));
}
