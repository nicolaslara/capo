//! LIVE bridge smoke: drive the REAL `@zed-industries/claude-code-acp` ACP bridge
//! (over the headless Claude subscription, NO API key) THROUGH the public
//! `CapoServer` command surface into the controller's `drive_acp_live_turn` seam,
//! and assert an OBSERVED file change on disk plus the event log shape.
//!
//! This is the live counterpart to `acp_dispatch_smoke.rs` (the deterministic
//! `/bin/sh` stub). It spawns `npx -y @zed-industries/claude-code-acp` confined to
//! a scratch git workspace, switches the session to a permission-bypassing mode so
//! the bridge emits a real on-wire `fs/write_text_file` callback (rather than
//! simulating the Write tool in its default mode), and proves the agent wrote
//! `HELLO.txt` containing exactly `capo-works`.
//!
//! GATING: this test is `#[ignore]`d AND returns early unless `CAPO_E2E_LIVE_ACP=1`,
//! so a plain `cargo test` never runs it (it is slow: an `npx` fetch + a nested
//! `claude`). Run it explicitly with:
//!
//! ```text
//! export CARGO_TARGET_DIR=/Users/.../capo/target \
//!   CAPO_E2E_LIVE_ACP=1 CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1
//! cargo test -p capo-server --test acp_live_bridge_smoke -- --ignored --nocapture
//! ```
//!
//! It passes `HOME` through (so the subscription creds under `~/.claude` are
//! visible) and never passes `ANTHROPIC_API_KEY` (the runtime's `env_clear()` +
//! allowlist scrubs it, along with the `CLAUDECODE` / `CLAUDE_CODE_*` nested-session
//! guard vars, so the bridge's nested `claude` launches cleanly).

use capo_core::ProjectId;
use capo_server::{CapoServer, ServerCommand, ServerRequest, ServerResponsePayload};

/// Whether the operator opted into the slow live bridge run.
fn live_gate_on() -> bool {
    std::env::var("CAPO_E2E_LIVE_ACP").as_deref() == Ok("1")
}

#[test]
#[ignore = "live: spawns npx @zed-industries/claude-code-acp + a nested claude over the subscription; \
            set CAPO_E2E_LIVE_ACP=1 (and the live ACP env gate) to run"]
fn live_acp_bridge_writes_observed_file_through_server() {
    if !live_gate_on() {
        eprintln!(
            "skipping live ACP bridge smoke: CAPO_E2E_LIVE_ACP != 1 (this test only runs when \
             explicitly opted in)"
        );
        return;
    }

    // Open the live ACP gate the adapter + server arm self-check. The operator is
    // expected to also export these, but we set them so the gated run is
    // self-contained.
    // SAFETY: no other capo-server test reads these vars concurrently in this
    // dedicated live test binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-server-acp-live-bridge");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The confined working directory the bridge runs in and writes into. The real
    // bridge prefers a git repo (Claude Code expects a project root); make it one.
    let workspace = root.join("acp-live-ws");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&workspace)
        .status()
        .expect("git init");
    assert!(git_init.success(), "git init must succeed");

    // ACT 1: register an acp-bound agent through the server command surface.
    let reg = server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "acp-live-worker".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register agent");
    assert!(
        matches!(reg.payload, ServerResponsePayload::AgentRegistered(_)),
        "expected AgentRegistered, got {:?}",
        reg.payload
    );

    // ACT 2: create the acp session+run through the server.
    let session_id = "session-acp-live-bridge";
    let run_id = "run-acp-live-bridge";
    let goal = "Use the Write tool to create a file named HELLO.txt in the current directory \
                containing exactly: capo-works (no trailing newline, no other text).";
    let started = server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "acp-live-worker".to_string(),
            goal: goal.to_string(),
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
    // controller's drive_acp_live_turn seam, against the REAL bridge. The session
    // mode `bypassPermissions` is the proven path that makes the bridge emit a real
    // on-wire `fs/write_text_file` callback (in its `default` mode it simulates the
    // Write tool in a subagent and never writes).
    let resp = server
        .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            turn_id: "turn-acp-live-bridge".to_string(),
            acp_program: "npx".to_string(),
            acp_argv: vec![
                "-y".to_string(),
                "@zed-industries/claude-code-acp".to_string(),
            ],
            workspace_root: Some(workspace.to_string_lossy().to_string()),
            live_acp_opt_in: true,
            acp_session_mode: Some("bypassPermissions".to_string()),
            mcp_url: None,
            mcp_headers: vec![],
            steer_window_secs: 0,
        }))
        .expect("run acp live turn against the real bridge");

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => panic!("expected AcpLiveTurn, got {other:?}"),
    };
    eprintln!(
        "live ACP turn: event_count={} appended={} stop_reason={:?}",
        summary.event_count, summary.appended_event_count, summary.stop_reason
    );
    assert!(
        summary.event_count >= 1,
        "the driven live ACP turn must stream at least one normalized event"
    );
    assert!(
        summary.appended_event_count >= 1,
        "the per-event batch must be ingested through the loop's normal route"
    );

    // ASSERT (a): the OBSERVED file the REAL agent wrote into the confined
    // workspace, with EXACTLY the requested content.
    let written = std::fs::read_to_string(workspace.join("HELLO.txt"))
        .expect("the real ACP bridge must have written HELLO.txt into the confined workspace");
    assert_eq!(
        written, "capo-works",
        "HELLO.txt must contain exactly `capo-works`; got {written:?}"
    );

    // ASSERT (b): the event log (public subscribe backlog) shows the registration
    // and that the live ACP turn was ingested as tool events for this session.
    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe backlog");
    let kinds: Vec<&str> = backlog.events.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        kinds.contains(&"agent.registered"),
        "event log must show agent.registered; got {kinds:?}"
    );

    // The live turn ingested at least one `tool.*` event for THIS session through
    // the loop's normal route. We assert on kind + session_id rather than the raw
    // payload substring: unlike the deterministic `/bin/sh` stub (whose tool title
    // is a benign `write out.txt`), the real bridge's tool_call payload can carry a
    // diff / rawInput / cache path that trips capo's credential-shape redactor, in
    // which case the persisted `payload_json` is the `[REDACTED:credential]`
    // placeholder -- capo working as intended. The presence of the ACP-origin tool
    // events (the turn's `appended_event_count` above) is the load-bearing proof
    // the turn flowed through ingestion.
    let acp_tool_events = backlog
        .events
        .iter()
        .filter(|e| e.session_id.as_deref() == Some(session_id) && e.kind.starts_with("tool."))
        .count();
    assert!(
        acp_tool_events >= 1,
        "event log must show at least one tool.* event for the live ACP turn's session; \
         kinds: {kinds:?}"
    );
    // And the unredacted ones, if any, carry the ACP adapter origin.
    let has_unredacted_acp_origin = backlog.events.iter().any(|e| {
        e.session_id.as_deref() == Some(session_id)
            && e.kind.starts_with("tool.")
            && e.payload_json.contains("\"adapter_kind\":\"acp\"")
    });
    eprintln!(
        "live ACP turn ingested {acp_tool_events} tool.* events for {session_id} \
         (unredacted acp-origin payload visible: {has_unredacted_acp_origin})"
    );
}
