use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{net::TcpListener, thread};

use capo_core::ProjectId;
use capo_state::SqliteStateStore;

use crate::{
    CapoServer, ServerClientOrigin, ServerCommand, ServerInputOrigin, ServerRequest,
    ServerResponse, ServerResponsePayload, send_tcp, serve_tcp,
};

static TEMP_ROOT_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
    assert_eq!(session.run_status.as_deref(), Some("exited_unknown"));
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
fn tcp_transport_round_trips_server_requests_and_recovers_state() {
    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(3),
        )
        .expect("serve tcp")
    });

    let registered = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-register-mock-codex",
            ServerCommand::RegisterAgent {
                name: "mock-codex".to_string(),
            },
        ),
    )
    .expect("register over tcp");
    assert_agent_registered(&registered, "mock-codex");

    let sent = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-send-mock-codex",
            ServerCommand::SendTask {
                agent_name: "mock-codex".to_string(),
                goal: "Prove TCP transport".to_string(),
                scenario: "default".to_string(),
            },
        ),
    )
    .expect("send over tcp");
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent");
    };
    assert_eq!(run.session_id.as_str(), "session-mock-codex");

    let dashboard = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-dashboard",
            ServerCommand::Dashboard {
                recent_event_limit: 8,
            },
        ),
    )
    .expect("dashboard over tcp");
    let ServerResponsePayload::Dashboard(snapshot) = dashboard.payload else {
        panic!("expected dashboard");
    };
    assert_eq!(snapshot.agent_count, 1);
    assert_eq!(snapshot.active_session_count, 1);

    assert_eq!(server_thread.join().expect("server thread"), 3);

    let listener = TcpListener::bind("127.0.0.1:0").expect("restart listener");
    let restart_address = listener.local_addr().expect("restart address");
    let restart_root = root.clone();
    let restart_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            restart_root,
            Some(2),
        )
        .expect("serve restarted tcp")
    });

    let recovered = send_tcp(
        restart_address,
        &ServerRequest::local_cli("tcp-recover", ServerCommand::Recover),
    )
    .expect("recover after restart over tcp");
    let ServerResponsePayload::Recovery(recovery) = recovered.payload else {
        panic!("expected recovery");
    };
    assert_eq!(recovery.recovered_run_count, 1);

    let restarted_dashboard = send_tcp(
        restart_address,
        &ServerRequest::local_cli(
            "tcp-dashboard-after-restart",
            ServerCommand::Dashboard {
                recent_event_limit: 8,
            },
        ),
    )
    .expect("dashboard after restart over tcp");
    let ServerResponsePayload::Dashboard(snapshot) = restarted_dashboard.payload else {
        panic!("expected restarted dashboard");
    };
    assert_eq!(snapshot.agent_count, 1);
    assert_eq!(snapshot.active_session_count, 0);
    assert_eq!(restart_thread.join().expect("restart thread"), 2);

    let reopened = CapoServer::open(ProjectId::new("project-capo"), root).expect("reopen");
    let dashboard = reopened.dashboard_snapshot().expect("dashboard");
    assert_eq!(dashboard.agent_count, 1);
    assert_eq!(dashboard.active_session_count, 0);
}

fn handle(server: &CapoServer, command: ServerCommand) -> ServerResponse {
    server
        .handle(ServerRequest::cli(command))
        .unwrap_or_else(|error| panic!("server request failed: {error:?}"))
}

fn assert_agent_registered(response: &ServerResponse, name: &str) {
    let ServerResponsePayload::AgentRegistered(agent) = &response.payload else {
        panic!("expected agent registered response");
    };
    assert_eq!(agent.name, name);
    assert_eq!(agent.status, "available");
    assert_eq!(agent.current_session_id, None);
}

fn temp_root() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("capo-server-{nanos}-{counter}"))
}
