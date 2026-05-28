use super::*;

#[test]
fn server_native_sessions_allow_multiple_historical_sessions_per_agent() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );

    let first = handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Historical session 1".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-local-1".to_string()),
            run_id: Some("run-codex-local-1".to_string()),
        },
    );
    let ServerResponsePayload::SessionStarted(first) = first.payload else {
        panic!("expected first session started");
    };
    assert_eq!(first.session_id.as_str(), "session-codex-local-1");

    let concurrent_error = server
        .handle(ServerRequest::local_cli(
            "start-concurrent-session",
            ServerCommand::StartSession {
                agent_name: "codex-local".to_string(),
                goal: "Concurrent session should wait".to_string(),
                adapter: "codex".to_string(),
                session_id: Some("session-codex-local-concurrent".to_string()),
                run_id: Some("run-codex-local-concurrent".to_string()),
            },
        ))
        .expect_err("concurrent active session should be rejected");
    assert!(matches!(
        concurrent_error,
        ServerError::AgentAlreadyHasSession { session_id, .. }
            if session_id == "session-codex-local-1"
    ));

    handle(&server, ServerCommand::Recover);

    let duplicate_run_error = server
        .handle(ServerRequest::local_cli(
            "start-duplicate-run-session",
            ServerCommand::StartSession {
                agent_name: "codex-local".to_string(),
                goal: "Duplicate run ID should fail".to_string(),
                adapter: "codex".to_string(),
                session_id: Some("session-codex-local-duplicate-run".to_string()),
                run_id: Some("run-codex-local-1".to_string()),
            },
        ))
        .expect_err("duplicate run id should be rejected");
    assert!(matches!(
        duplicate_run_error,
        ServerError::RunAlreadyExists { run_id } if run_id == "run-codex-local-1"
    ));

    let second = handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Historical session 2".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-local-2".to_string()),
            run_id: Some("run-codex-local-2".to_string()),
        },
    );
    let ServerResponsePayload::SessionStarted(second) = second.payload else {
        panic!("expected second session started");
    };
    assert_eq!(second.session_id.as_str(), "session-codex-local-2");

    let state = SqliteStateStore::open(&root).expect("state");
    assert!(
        state
            .session(&SessionId::new("session-codex-local-1"))
            .expect("first session lookup")
            .is_some()
    );
    assert!(
        state
            .session(&SessionId::new("session-codex-local-2"))
            .expect("second session lookup")
            .is_some()
    );

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let agent = dashboard
        .agents
        .iter()
        .find(|agent| agent.name == "codex-local")
        .expect("codex agent");
    assert_eq!(
        agent.current_session_id.as_ref().map(ToString::to_string),
        Some("session-codex-local-2".to_string())
    );
}

#[test]
fn server_native_session_start_persists_goal_hash_instead_of_raw_goal() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );
    let raw_goal = "DO_NOT_PERSIST_RAW_GOAL_SV13";
    let response = handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: raw_goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-redacted-goal".to_string()),
            run_id: Some("run-codex-redacted-goal".to_string()),
        },
    );
    let raw_request = ServerRequest::cli(ServerCommand::StartSession {
        agent_name: "codex-local".to_string(),
        goal: raw_goal.to_string(),
        adapter: "codex".to_string(),
        session_id: Some("session-codex-redacted-goal-request-id".to_string()),
        run_id: Some("run-codex-redacted-goal-request-id".to_string()),
    });
    assert!(!raw_request.request_id.contains(raw_goal));
    assert!(!raw_request.request_id.contains("DO_NOT_PERSIST"));
    assert!(
        raw_request
            .request_id
            .contains("session-codex-redacted-goal-request-id")
    );
    let ServerResponsePayload::SessionStarted(run) = response.payload else {
        panic!("expected session started");
    };
    let state = SqliteStateStore::open(&root).expect("state");
    let session = state
        .session(&run.session_id)
        .expect("session query")
        .expect("session");
    assert!(session.current_goal.starts_with("goal_hash:"));
    assert!(session.current_goal.contains("raw_policy:not_rendered"));
    assert!(!session.current_goal.contains(raw_goal));
    let task = state.task(&run.task_id).expect("task query").expect("task");
    assert!(task.title.starts_with("goal_hash:"));
    assert!(!task.title.contains(raw_goal));
    let events = state
        .recent_events_for_session(&run.session_id, 20)
        .expect("session events");
    assert!(events.iter().any(|event| {
        event.kind == "server.request_handled"
            && event
                .payload_json
                .contains("\"raw_goal_policy\":\"not_rendered\"")
            && event.payload_json.contains("\"goal_hash\":")
    }));
    for event in events {
        assert!(
            !event.payload_json.contains(raw_goal),
            "raw goal leaked in {} payload: {}",
            event.kind,
            event.payload_json
        );
    }
}

#[test]
fn server_rejects_adapter_fixture_replay_without_existing_session() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let error = server
        .handle(ServerRequest::local_cli(
            "replay-missing-session",
            ServerCommand::ReplayAdapterFixture {
                adapter: "codex".to_string(),
                session_id: "session-missing".to_string(),
                run_id: "run-missing".to_string(),
                turn_id: "turn-missing".to_string(),
                fixture_name: "codex-exec.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                    .to_string(),
            },
        ))
        .expect_err("missing session should be rejected");
    assert!(matches!(
        error,
        ServerError::UnknownSession { session_id } if session_id == "session-missing"
    ));
}

#[test]
fn server_rejects_adapter_fixtures_over_raw_body_cap() {
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
            goal: "Reject oversized fixture".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-local-cap".to_string()),
            run_id: Some("run-codex-local-cap".to_string()),
        },
    );

    let error = server
        .handle(ServerRequest::local_cli(
            "oversized-fixture",
            ServerCommand::ReplayAdapterFixture {
                adapter: "codex".to_string(),
                session_id: "session-codex-local-cap".to_string(),
                run_id: "run-codex-local-cap".to_string(),
                turn_id: "turn-codex-local-cap".to_string(),
                fixture_name: "oversized.jsonl".to_string(),
                fixture_jsonl: "x".repeat(300 * 1024),
            },
        ))
        .expect_err("oversized fixture should be rejected");
    assert!(matches!(error, ServerError::AdapterFixture(message) if message.contains("too large")));
}
