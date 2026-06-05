//! SLICE-A server-level integration test: drive an `acp`-bound agent THROUGH the
//! server command surface into the controller's `drive_acp_live_turn` seam, and
//! assert an OBSERVED file change on disk plus the event log shape
//! (`agent.registered` + a turn + the ACP-origin tool call).
//!
//! This exercises the SAME `AcpLiveAdapter` + `drive_acp_live_turn` path the
//! test-only DP11 smoke drives, but reaches it through the public `CapoServer`
//! command surface (`RegisterAgent` -> `StartSession` -> `RunAcpLiveTurnLocal`).
//! The agent is a LOCAL `/bin/sh` ACP stub (deterministic, `env_clear()` spawn,
//! no network) -- NOT the live `npx` ACP bridge.
//!
//! It is gated by the SAME live ACP env gate the adapter self-checks
//! (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` + `CAPO_SERVER_RUN_ACP_LIVE=1`); the
//! test sets the gate itself so it runs deterministically without an operator.

use capo_core::ProjectId;
use capo_server::{CapoServer, ServerCommand, ServerRequest, ServerResponsePayload};

/// Write an executable `/bin/sh` ACP-compatible stub agent that, on
/// `session/prompt`, WRITES a real file into its confined cwd (the adapter's
/// `workspace_root`) and finalizes the turn `end_turn` with a completed
/// tool_call. The file write is performed by the agent process itself, confined
/// to the workspace -- the OBSERVED file change Slice A proves. It writes only to
/// stdout/stderr + the one workspace file, and exits on stdin EOF.
fn write_file_acp_agent_stub(dir: &std::path::Path, out_file: &str, contents: &str) -> String {
    std::fs::create_dir_all(dir).expect("stub dir");
    let stub = dir.join("acp-write-agent-stub.sh");
    // The stub matches on the method substring (no JSON parser; runs under the
    // runtime's env_clear() PATH). On `session/prompt` it writes the workspace
    // file in its cwd, streams a completed tool_call update, then answers the
    // prompt with stopReason end_turn.
    let script = format!(
        r#"#!/bin/sh
emit() {{ printf '%s\n' "$1"; }}
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      emit "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"protocolVersion\":1}}}}"
      ;;
    *'"method":"session/new"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      emit "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"sessionId\":\"acp-slicea-session\"}}}}"
      ;;
    *'"method":"session/prompt"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      # OBSERVED file change: the agent writes a file into its confined cwd.
      printf '%s' "{contents}" > "{out_file}"
      # Stream a completed tool_call update for the edit.
      emit "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"acp-slicea-session\",\"update\":{{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"tool-slicea-1\",\"title\":\"write {out_file}\",\"status\":\"completed\"}}}}}}"
      emit "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"stopReason\":\"end_turn\"}}}}"
      ;;
    *) : ;;
  esac
done
"#,
        contents = contents,
        out_file = out_file,
    );
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
fn server_acp_dispatch_writes_observed_file_and_logs_events() {
    // Open the live ACP gate the adapter self-checks (and the new server arm
    // double-checks). The DP11 live smoke is the paired evidence for this path.
    // SAFETY: no other capo-server test reads these env vars.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-server-acp-slicea");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The confined working directory the agent runs in and writes into.
    let workspace = root.join("acp-ws");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let stub_dir = root.join("acp-stub");
    let program = write_file_acp_agent_stub(&stub_dir, "out.txt", "hi from acp");

    // ACT 1: register an acp-bound agent through the server command surface.
    let reg = server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "acp-worker".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register agent");
    assert!(
        matches!(reg.payload, ServerResponsePayload::AgentRegistered(_)),
        "expected AgentRegistered, got {:?}",
        reg.payload
    );

    // ACT 2: create the acp session+run through the server.
    let session_id = "session-acp-slicea";
    let run_id = "run-acp-slicea";
    let started = server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "acp-worker".to_string(),
            goal: "write a file".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start session");
    assert!(
        matches!(started.payload, ServerResponsePayload::SessionStarted(_)),
        "expected SessionStarted, got {:?}",
        started.payload
    );

    // ACT 3: drive ONE live ACP turn through the server command surface into the
    // controller's drive_acp_live_turn seam, confined to `workspace`.
    let resp = server
        .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            goal: "write a file".to_string(),
            turn_id: "turn-acp-slicea".to_string(),
            acp_program: program,
            acp_argv: Vec::new(),
            workspace_root: Some(workspace.to_string_lossy().to_string()),
            live_acp_opt_in: true,
        }))
        .expect("run acp live turn");

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => panic!("expected AcpLiveTurn, got {other:?}"),
    };
    assert!(
        summary.event_count >= 1,
        "the driven ACP turn must stream at least one normalized event"
    );
    assert!(
        summary.appended_event_count >= 1,
        "the per-event batch must be ingested through the loop's normal route"
    );
    assert_eq!(
        summary.stop_reason.as_deref(),
        Some("end_turn"),
        "the stub finalizes the turn end_turn"
    );

    // ASSERT (a): OBSERVED file on disk in the confined workspace.
    let written = std::fs::read_to_string(workspace.join("out.txt"))
        .expect("agent must have written the observed file into the confined workspace");
    assert_eq!(written, "hi from acp");

    // ASSERT (b): the event log (public subscribe backlog) shows the registration
    // and an ACP-origin tool call for this session.
    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe backlog");
    let kinds: Vec<&str> = backlog.events.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        kinds.contains(&"agent.registered"),
        "event log must show agent.registered; got {kinds:?}"
    );
    // The ACP turn ingested at least one tool/adapter event for the session: a
    // tool.* event whose payload carries the ACP adapter origin (`adapter_kind`).
    let has_acp_tool_event = backlog.events.iter().any(|e| {
        e.session_id.as_deref() == Some(session_id)
            && e.kind.starts_with("tool.")
            && e.payload_json.contains("\"adapter_kind\":\"acp\"")
    });
    assert!(
        has_acp_tool_event,
        "event log must show an ACP-origin tool event for the turn; kinds: {kinds:?}"
    );
}

/// Fail-closed contract: with the per-command `live_acp_opt_in=false`, the server
/// MUST reject the turn BEFORE spawning anything and MUST NOT write any file --
/// regardless of the env gate. This pins the "default behavior stays gated"
/// invariant so the gate can't silently regress (e.g. an inverted bool).
///
/// This case is deterministic and race-free w.r.t. the gate-open test above: the
/// handler short-circuits on `!live_acp_opt_in` first, so it does not depend on
/// the process-global `CAPO_SERVER_*` env vars at all.
#[test]
fn server_acp_dispatch_fails_closed_when_opt_in_is_false() {
    let root = capo_tmptest::TempRoot::new("capo-server-acp-slicea-closed");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    let workspace = root.join("acp-ws-closed");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    // A program path that would write a file IF it were ever spawned -- it must not be.
    let stub_dir = root.join("acp-stub-closed");
    let program = write_file_acp_agent_stub(&stub_dir, "out.txt", "should-not-exist");

    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "acp-worker-closed".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register agent");
    let session_id = "session-acp-slicea-closed";
    let run_id = "run-acp-slicea-closed";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "acp-worker-closed".to_string(),
            goal: "write a file".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start session");

    let result = server.handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        goal: "write a file".to_string(),
        turn_id: "turn-acp-slicea-closed".to_string(),
        acp_program: program,
        acp_argv: Vec::new(),
        workspace_root: Some(workspace.to_string_lossy().to_string()),
        live_acp_opt_in: false,
    }));

    // Fail-closed may surface as a Rust `Err` OR as a non-success payload; either
    // is acceptable. What is NOT acceptable is a successful turn.
    if let Ok(resp) = result {
        assert!(
            !matches!(resp.payload, ServerResponsePayload::AcpLiveTurn(_)),
            "a closed gate must NOT produce a successful AcpLiveTurn, got {:?}",
            resp.payload
        );
    }

    // The decisive safety assertion: nothing was written.
    assert!(
        !workspace.join("out.txt").exists(),
        "no file may be written when the acp turn is fail-closed"
    );
}
