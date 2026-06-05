//! SLICE-A (DESIGN-B Layer 2) server-level integration test: drive ONE CONDUCTOR
//! turn THROUGH the server command surface (`RunConductorTurnLocal`) into the
//! controller's `drive_acp_live_turn` seam, and assert the two Layer-2 deltas on
//! the WIRE, deterministically (NO live `npx` bridge):
//!
//!   1. capo's in-process HTTP MCP endpoint is forwarded into `session/new`'s
//!      `mcpServers` array as `{ "type":"http", "url", "headers":[{name,value}] }`.
//!   2. the conductor prompt is composed as `"{conductor_goal}\n\n[user]: {msg}"`.
//!
//! The agent is a LOCAL `/bin/sh` ACP stub (deterministic, `env_clear()` spawn,
//! no network) that DUMPS the inbound `session/new` and `session/prompt` frames
//! to files in its confined cwd so the test can inspect them. Gated by the SAME
//! live ACP env gate the adapter self-checks; the test sets the gate itself.

use capo_core::ProjectId;
use capo_server::{CapoServer, ServerCommand, ServerRequest, ServerResponsePayload};

/// A `/bin/sh` ACP stub that, on `session/new`, dumps the FULL inbound line to
/// `newline.json` in its cwd (so the test can inspect the forwarded
/// `mcpServers`), and on `session/prompt` dumps the full inbound line to
/// `prompt.json` (so the test can inspect the composed conductor prompt) before
/// finalizing the turn `end_turn`.
fn dump_frames_acp_agent_stub(dir: &std::path::Path) -> String {
    std::fs::create_dir_all(dir).expect("stub dir");
    let stub = dir.join("acp-conductor-stub.sh");
    let script = r#"#!/bin/sh
emit() { printf '%s\n' "$1"; }
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}"
      ;;
    *'"method":"session/new"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '%s' "$line" > newline.json
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"acp-conductor-session\"}}"
      ;;
    *'"method":"session/prompt"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '%s' "$line" > prompt.json
      emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"acp-conductor-session\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"ok\"}}}}"
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"stopReason\":\"end_turn\"}}"
      ;;
    *) : ;;
  esac
done
"#;
    std::fs::write(&stub, script).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).expect("chmod");
    }
    stub.to_string_lossy().to_string()
}

#[test]
fn server_conductor_turn_forwards_mcp_and_composes_prompt() {
    // Open the live ACP gate the adapter self-checks (and the conductor arm
    // double-checks). SAFETY: no other capo-server test reads these env vars
    // EXCEPT acp_dispatch_smoke, which sets them to the SAME values.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-server-conductor");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The conductor runs in the server's default project dir (acp_workspace_root);
    // the stub dumps frames into its cwd, which is that workspace root.
    let stub_dir = root.join("acp-stub");
    let program = dump_frames_acp_agent_stub(&stub_dir);

    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "conductor".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register conductor");

    let session_id = "session-conductor";
    let run_id = "run-conductor";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "conductor".to_string(),
            goal: "manage workers".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start session");

    let mcp_url = "http://127.0.0.1:54321/mcp";
    let resp = server
        .handle(ServerRequest::cli(ServerCommand::RunConductorTurnLocal {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            turn_id: "turn-conductor".to_string(),
            user_message: "spin up a worker for the docs task".to_string(),
            conductor_goal: "You are the capo conductor.".to_string(),
            mcp_url: mcp_url.to_string(),
            mcp_headers: vec![(
                "Authorization".to_string(),
                "Bearer capo-secret".to_string(),
            )],
            acp_program: program,
            acp_argv: Vec::new(),
            acp_session_mode: None,
            live_acp_opt_in: true,
        }))
        .expect("run conductor turn");

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => panic!("expected AcpLiveTurn, got {other:?}"),
    };
    assert_eq!(
        summary.stop_reason.as_deref(),
        Some("end_turn"),
        "the conductor turn finalizes end_turn"
    );

    // The stub's cwd is the conductor workspace root; find the dumped frames.
    let ws = std::path::PathBuf::from(&summary.workspace_root);
    let newline = std::fs::read_to_string(ws.join("newline.json"))
        .expect("stub must have dumped the session/new frame");
    let prompt = std::fs::read_to_string(ws.join("prompt.json"))
        .expect("stub must have dumped the session/prompt frame");

    // DELTA 1: the forwarded HTTP MCP entry shape lands in session/new.
    let newline_json: serde_json::Value =
        serde_json::from_str(&newline).expect("session/new is valid JSON");
    let mcp = newline_json
        .pointer("/params/mcpServers")
        .and_then(|v| v.as_array())
        .expect("session/new params.mcpServers array");
    assert_eq!(mcp.len(), 1, "exactly one forwarded MCP server");
    let entry = &mcp[0];
    assert_eq!(entry.get("type").and_then(|v| v.as_str()), Some("http"));
    assert_eq!(entry.get("url").and_then(|v| v.as_str()), Some(mcp_url));
    let headers = entry
        .get("headers")
        .and_then(|v| v.as_array())
        .expect("headers array");
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[0].get("name").and_then(|v| v.as_str()),
        Some("Authorization")
    );
    assert_eq!(
        headers[0].get("value").and_then(|v| v.as_str()),
        Some("Bearer capo-secret")
    );

    // DELTA 2: the conductor prompt is composed as goal + [user]: message.
    assert!(
        prompt.contains("You are the capo conductor.")
            && prompt.contains("[user]: spin up a worker for the docs task"),
        "the composed conductor prompt must carry both the goal and the user \
         message; got: {prompt}"
    );
}

/// Fail-closed contract for the conductor command: with `live_acp_opt_in=false`
/// the server MUST reject the turn BEFORE spawning anything, regardless of the
/// env gate. Deterministic and race-free (short-circuits on the bool first).
#[test]
fn server_conductor_turn_fails_closed_when_opt_in_is_false() {
    let root = capo_tmptest::TempRoot::new("capo-server-conductor-closed");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    let stub_dir = root.join("acp-stub-closed");
    let program = dump_frames_acp_agent_stub(&stub_dir);

    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "conductor-closed".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register conductor");
    let session_id = "session-conductor-closed";
    let run_id = "run-conductor-closed";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "conductor-closed".to_string(),
            goal: "manage workers".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start session");

    let result = server.handle(ServerRequest::cli(ServerCommand::RunConductorTurnLocal {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        turn_id: "turn-conductor-closed".to_string(),
        user_message: "do something".to_string(),
        conductor_goal: "conductor".to_string(),
        mcp_url: "http://127.0.0.1:1/mcp".to_string(),
        mcp_headers: Vec::new(),
        acp_program: program,
        acp_argv: Vec::new(),
        acp_session_mode: None,
        live_acp_opt_in: false,
    }));

    if let Ok(resp) = result {
        assert!(
            !matches!(resp.payload, ServerResponsePayload::AcpLiveTurn(_)),
            "a closed gate must NOT produce a successful conductor turn, got {:?}",
            resp.payload
        );
    }
}
