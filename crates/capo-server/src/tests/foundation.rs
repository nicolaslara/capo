use super::*;

#[test]
fn client_tracks_mock_agent_through_server_boundary_and_recovers() {
    let root = temp_root();
    let project_id = ProjectId::new("project-capo");
    let server = CapoServer::open(project_id.clone(), &root).expect("server");

    let registered = handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "mock-codex".to_string(),
        },
    );
    assert_agent_registered(&registered, "mock-codex");

    let sent = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "mock-codex".to_string(),
            goal: "Prove server-owned mock agent tracking".to_string(),
            scenario: "tool-memory".to_string(),
        },
    );
    assert_eq!(
        sent.request_id,
        "server-task-send-mock-codex-prove-server-owned-mock-agent-tracking"
    );
    assert_eq!(sent.client_id, "local-cli");
    assert_eq!(sent.input_origin, ServerInputOrigin::Cli);
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent response");
    };
    assert_eq!(
        run.task_id.as_str(),
        "task-prove-server-owned-mock-agent-tracking"
    );
    assert_eq!(run.session_id.as_str(), "session-mock-codex");
    assert_eq!(run.run_id.as_str(), "run-mock-codex");

    let listed = handle(&server, ServerCommand::ListAgents);
    let ServerResponsePayload::Agents(agents) = listed.payload else {
        panic!("expected agent list response");
    };
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "mock-codex");
    assert_eq!(agents[0].status, "running");
    assert_eq!(
        agents[0].current_session_id.as_ref().map(|id| id.as_str()),
        Some("session-mock-codex")
    );
    assert_eq!(
        agents[0]
            .session
            .as_ref()
            .map(|session| session.tool_call_count),
        Some(1)
    );
    assert_eq!(
        agents[0]
            .session
            .as_ref()
            .map(|session| session.memory_packet_count),
        Some(1)
    );

    let dashboard = handle(
        &server,
        ServerCommand::Dashboard {
            recent_event_limit: 8,
        },
    );
    let ServerResponsePayload::Dashboard(snapshot) = dashboard.payload else {
        panic!("expected dashboard response");
    };
    assert_eq!(snapshot.project_id, project_id);
    assert_eq!(snapshot.agent_count, 1);
    assert_eq!(snapshot.active_session_count, 1);
    assert_eq!(
        snapshot.agents[0]
            .session
            .as_ref()
            .map(|session| session.recent_event_count)
            .unwrap_or_default(),
        8
    );

    let recovery = handle(&server, ServerCommand::Recover);
    let ServerResponsePayload::Recovery(recovery) = recovery.payload else {
        panic!("expected recovery response");
    };
    assert_eq!(recovery.recovered_run_count, 1);
    assert!(recovery.watermark.is_some());

    let reopened = CapoServer::open(ProjectId::new("project-capo"), &root).expect("reopen server");
    let recovered_dashboard = reopened
        .dashboard_snapshot()
        .expect("recovered dashboard snapshot");
    assert_eq!(recovered_dashboard.agent_count, 1);
    assert_eq!(recovered_dashboard.active_session_count, 0);
    let session = recovered_dashboard.agents[0]
        .session
        .as_ref()
        .expect("session survives recovery");
    assert_eq!(session.status, "active");
    // RTL10: restart recovery reaps the orphaned in-flight run and records a
    // terminal `run.recovered`, so the reconciled run status is `recovered`.
    assert_eq!(session.run_status.as_deref(), Some("recovered"));
    assert_eq!(session.tool_call_count, 1);
    assert_eq!(session.tool_observation_count, 0);
    assert_eq!(session.memory_packet_count, 1);

    let state = SqliteStateStore::open(&root).expect("state");
    let session_events = state
        .recent_events_for_session(&run.session_id, 20)
        .expect("session events");
    let server_event = session_events
        .iter()
        .find(|event| event.kind == "server.request_handled")
        .expect("server request audit event");
    assert_eq!(server_event.actor, "local-user");
    assert_eq!(
        server_event.item_id.as_deref(),
        Some(sent.request_id.as_str())
    );
    assert!(
        server_event
            .payload_json
            .contains("\"command_kind\":\"send_task\"")
    );
    assert!(
        server_event
            .idempotency_key
            .as_deref()
            .is_some_and(|key| key.contains("server:local-cli:local-user"))
    );
}

#[test]
fn server_steers_existing_agent_session_through_boundary() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "mock-operator".to_string(),
        },
    );
    let sent = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "mock-operator".to_string(),
            goal: "Initial operator goal".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent response");
    };

    let raw_goal = "Please summarize current progress and wait for review";
    let steered = server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "mock-operator".to_string(),
            goal: raw_goal.to_string(),
        }))
        .expect("steer agent");
    assert!(!steered.request_id.contains("please-summarize"));
    let ServerResponsePayload::AgentStatus(agent) = steered.payload else {
        panic!("expected agent status response");
    };
    assert_eq!(agent.name, "mock-operator");
    assert_eq!(
        agent
            .session
            .as_ref()
            .and_then(|session| session.run_id.as_ref()),
        Some(&run.run_id)
    );

    let state = SqliteStateStore::open(&root).expect("state");
    let session = state
        .session(&run.session_id)
        .expect("session query")
        .expect("session");
    assert_eq!(session.current_goal, raw_goal);
    let session_events = state
        .recent_events_for_session(&run.session_id, 40)
        .expect("session events");
    assert!(session_events.iter().any(|event| {
        event.kind == "session.redirected" && event.payload_json.contains(raw_goal)
    }));
    let server_event = session_events
        .iter()
        .find(|event| {
            event.kind == "server.request_handled"
                && event
                    .payload_json
                    .contains("\"command_kind\":\"steer_agent\"")
        })
        .expect("server steer audit event");
    assert!(
        server_event
            .payload_json
            .contains("\"raw_goal_policy\":\"not_rendered\"")
    );
    assert!(!server_event.payload_json.contains(raw_goal));
}

#[test]
fn server_carries_client_origin_and_rejects_unknown_agent_status() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let response = server
        .handle(ServerRequest {
            request_id: "request-from-dashboard".to_string(),
            origin: ServerClientOrigin {
                client_id: "dashboard-client".to_string(),
                actor_id: "nicolas".to_string(),
                input_origin: ServerInputOrigin::Dashboard,
            },
            command: ServerCommand::RegisterAgent {
                name: "mock-reviewer".to_string(),
            },
        })
        .expect("register");

    assert_eq!(response.request_id, "request-from-dashboard");
    assert_eq!(response.client_id, "dashboard-client");
    assert_eq!(response.actor_id, "nicolas");
    assert_eq!(response.input_origin, ServerInputOrigin::Dashboard);
    assert_agent_registered(&response, "mock-reviewer");

    let error = server
        .handle(ServerRequest::local_cli(
            "missing-agent-status",
            ServerCommand::AgentStatus {
                agent_name: "missing".to_string(),
            },
        ))
        .expect_err("missing agent should fail");
    assert!(
        matches!(error, crate::ServerError::UnknownAgent { agent_name } if agent_name == "missing")
    );
}

#[test]
fn server_rejects_steer_when_agent_has_no_active_session() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "idle-agent".to_string(),
        },
    );

    let error = server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "idle-agent".to_string(),
            goal: "This should not create a session".to_string(),
        }))
        .expect_err("idle steer should fail");
    assert!(
        matches!(error, crate::ServerError::AgentHasNoActiveSession { agent_name } if agent_name == "idle-agent")
    );
}

#[test]
fn server_rejects_unknown_and_repeated_task_sends_before_fake_id_collision() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    let unknown = server
        .handle(ServerRequest::local_cli(
            "send-missing-agent",
            ServerCommand::SendTask {
                agent_name: "missing".to_string(),
                goal: "This should fail before controller dispatch".to_string(),
                scenario: "default".to_string(),
            },
        ))
        .expect_err("unknown task send should fail");
    assert!(
        matches!(unknown, crate::ServerError::UnknownAgent { agent_name } if agent_name == "missing")
    );

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "mock-codex".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "mock-codex".to_string(),
            goal: "First task should start".to_string(),
            scenario: "default".to_string(),
        },
    );
    let repeated = server
        .handle(ServerRequest::local_cli(
            "send-repeated-agent",
            ServerCommand::SendTask {
                agent_name: "mock-codex".to_string(),
                goal: "Second task would collide with fixed fake IDs".to_string(),
                scenario: "default".to_string(),
            },
        ))
        .expect_err("repeated task send should fail");
    assert!(matches!(
        repeated,
        crate::ServerError::AgentAlreadyHasSession {
            agent_name,
            session_id,
            run_status
        } if agent_name == "mock-codex"
            && session_id == "session-mock-codex"
            && run_status.as_deref() == Some("running")
    ));
}

#[test]
fn server_request_idempotency_is_bound_to_command_identity() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    server
        .handle(ServerRequest::local_cli(
            "same-request-id",
            ServerCommand::RegisterAgent {
                name: "alpha".to_string(),
            },
        ))
        .expect("register alpha");
    server
        .handle(ServerRequest::local_cli(
            "same-request-id",
            ServerCommand::RegisterAgent {
                name: "beta".to_string(),
            },
        ))
        .expect("register beta");

    let listed = handle(&server, ServerCommand::ListAgents);
    let ServerResponsePayload::Agents(agents) = listed.payload else {
        panic!("expected agents");
    };
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().any(|agent| agent.name == "alpha"));
    assert!(agents.iter().any(|agent| agent.name == "beta"));

    let state = SqliteStateStore::open(&root).expect("state");
    assert_eq!(state.event_count().expect("event count"), 4);
}
