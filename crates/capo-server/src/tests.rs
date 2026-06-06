use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    thread,
};

use capo_core::{ProjectId, SessionId};
use capo_state::SqliteStateStore;

use crate::{
    CapoServer, ServerClientOrigin, ServerCommand, ServerError, ServerInputOrigin, ServerRequest,
    ServerResponse, ServerResponsePayload, send_tcp, serve_tcp,
};

mod cancel_registry;
mod claude_chat;
mod claude_loop_route;
mod codex_chat;
mod codex_workspace_write;
mod contract;
mod controller_routing;
mod crash_recovery;
mod dispatch;
mod dt3;
mod dt4a;
mod dt4b;
mod dt5;
mod dt6;
mod e2e_gate;
mod event_tail;
mod foundation;
mod goal;
mod live_provider;
mod live_smoke;
mod multi_turn_edit;
mod per_turn_artifacts;
mod remote_crash_safety;
mod remote_live_smoke;
mod remote_materialization;
mod remote_recovery;
mod replay;
mod safety_floor;
mod sessions;
mod stream;
mod transport;
mod turn_orchestration;

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

fn temp_root() -> capo_tmptest::TempRoot {
    capo_tmptest::TempRoot::new("capo-server")
}
