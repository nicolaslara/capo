use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use capo_core::ProjectId;

use crate::{CapoServer, ServerCommand, ServerRequest, ServerResponse};

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
        },
    );
    let ServerResponse::TaskSent(run) = sent else {
        panic!("expected task sent response");
    };
    assert_eq!(
        run.task_id.as_str(),
        "task-prove-server-owned-mock-agent-tracking"
    );
    assert_eq!(run.session_id.as_str(), "session-mock-codex");
    assert_eq!(run.run_id.as_str(), "run-mock-codex");

    let listed = handle(&server, ServerCommand::ListAgents);
    let ServerResponse::Agents(agents) = listed else {
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
    let ServerResponse::Dashboard(snapshot) = dashboard else {
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
    let ServerResponse::Recovery(recovery) = recovery else {
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
}

fn handle(server: &CapoServer, command: ServerCommand) -> ServerResponse {
    server
        .handle(ServerRequest::cli(command))
        .unwrap_or_else(|error| panic!("server request failed: {error:?}"))
}

fn assert_agent_registered(response: &ServerResponse, name: &str) {
    let ServerResponse::AgentRegistered(agent) = response else {
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
