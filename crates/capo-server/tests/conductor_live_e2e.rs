//! SLICE-A / Layer 3 (STRETCH): the LIVE CONDUCTOR end-to-end.
//!
//! This proves the full nested depth-discipline flow over the real Claude
//! subscription (NO api key), with capo owning every session:
//!
//!   capo drives a CONDUCTOR `@zed-industries/claude-code-acp` session
//!     -> the conductor (real nested `claude`) dials capo's in-process
//!        localhost HTTP MCP endpoint (forwarded via `session/new`'s
//!        `mcpServers`) and CALLS the `start_agent` capo tool
//!     -> that tool drives a WORKER `@zed-industries/claude-code-acp` turn
//!        (`RunAcpLiveTurnLocal`, DEFAULT permission mode) that WRITES a file on
//!        disk via an ON-WIRE `fs/write_text_file` the worker's `Write` tool
//!        round-trips to capo, allowed by the controller's permission decider.
//!
//! It asserts BOTH halves of the proof:
//!   (a) the conductor actually CALLED `start_agent` (observable in capo's MCP
//!       server invocation log), AND
//!   (b) a real worker produced `HELLO.txt == "capo-works"` on disk in the
//!       confined project workspace.
//!
//! GATING: `#[ignore]`d AND returns early unless `CAPO_E2E_LIVE_ACP=1` (slow:
//! TWO nested `npx`/`claude` launches). Run with:
//!
//! ```text
//! export CARGO_TARGET_DIR=/Users/.../capo/target \
//!   CAPO_E2E_LIVE_ACP=1 CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1
//! cargo test -p capo-server --test conductor_live_e2e -- --ignored --nocapture
//! ```
//!
//! HOME is passed through (the runtime allowlist) so `~/.claude` subscription
//! creds are visible; `ANTHROPIC_API_KEY` and the `CLAUDECODE`/`CLAUDE_CODE_*`
//! nested-session guards are scrubbed by the runtime's `env_clear()` + allowlist.

use std::sync::{Arc, Mutex};

use capo_core::ProjectId;
use capo_server::{
    AcpWorkerToolConfig, CapoServer, McpState, ServerCommand, ServerRequest, ServerResponsePayload,
    ToolInvocation, acp_mcp_router,
};

fn live_gate_on() -> bool {
    std::env::var("CAPO_E2E_LIVE_ACP").as_deref() == Ok("1")
}

#[test]
#[ignore = "live: spawns TWO nested npx @zed-industries/claude-code-acp + claude sessions \
            (conductor + worker) over the subscription; set CAPO_E2E_LIVE_ACP=1 \
            (and the live ACP env gate) to run"]
fn live_conductor_drives_worker_that_writes_file() {
    if !live_gate_on() {
        eprintln!(
            "skipping live conductor E2E: CAPO_E2E_LIVE_ACP != 1 (this test only runs when \
             explicitly opted in)"
        );
        return;
    }

    // Open the live ACP gate the adapter + server arms self-check.
    // SAFETY: this is the only test in this dedicated live binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-conductor-live-e2e");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The conductor (and, sharing the same default workspace, the worker) run in
    // the server's project-dir ACP workspace `<state_root>/acp/workspace`. The
    // real bridge prefers a git repo (Claude Code expects a project root), so
    // pre-create + git-init it. The worker writes HELLO.txt here.
    let project_ws = root.join("acp").join("workspace");
    std::fs::create_dir_all(&project_ws).expect("project workspace dir");
    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project_ws)
        .status()
        .expect("git init");
    assert!(git_init.success(), "git init must succeed");

    // Boot capo's in-process STATELESS HTTP MCP server (Layer 1) with the LIVE
    // worker config: `start_agent` drives a REAL `npx` worker turn in DEFAULT
    // permission mode, where the worker's `Write` tool round-trips an on-wire
    // `fs/write_text_file` that capo's controller permission decider allows and
    // executes -- the supervised path that reliably produces the file.
    let bearer = "capo-conductor-e2e-secret".to_string();
    let worker = AcpWorkerToolConfig {
        acp_program: "npx".to_string(),
        acp_argv: vec![
            "-y".to_string(),
            "@zed-industries/claude-code-acp".to_string(),
        ],
        default_workspace_root: Some(project_ws.to_string_lossy().to_string()),
        // DEFAULT permission mode (NOT bypassPermissions): under `default` the real
        // bridge invokes its `Write` tool directly and asks for permission via
        // `session/request_permission`, which capo's controller decider answers with
        // Selected{allow}; the bridge then performs the write ON-WIRE via
        // `fs/write_text_file` (capo executes it, confined to the workspace). Under
        // `bypassPermissions` the worker is free to delegate the write to a `Task`
        // sub-agent whose Bash filesystem ops are SIMULATED (never round-tripped on
        // the wire), so no file lands -- the source of the original flake. `default`
        // forecloses that shortcut and keeps the write under capo supervision.
        acp_session_mode: Some("default".to_string()),
    };
    let state = McpState::new(server.clone(), worker, bearer.clone());
    let invocation_log: Arc<Mutex<Vec<ToolInvocation>>> = state.invocation_log();
    let app = acp_mcp_router(state);

    // A dedicated multi-thread tokio runtime hosts the MCP server. We keep the
    // conductor turn (a blocking `server.handle`) OFF this runtime's worker
    // threads (on the test thread) so the runtime stays free to service the
    // conductor's MCP callback while the blocking conductor turn is in flight.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let _server_guard = runtime.spawn(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral");
        let addr = listener.local_addr().expect("local addr");
        addr_tx.send(addr).expect("send addr");
        axum::serve(listener, app).await.expect("serve");
    });
    let addr = addr_rx.recv().expect("mcp server addr");
    let mcp_url = format!("http://{addr}/mcp");
    eprintln!("capo in-process MCP endpoint: {mcp_url}");

    // Register + start the CONDUCTOR acp session through the server surface.
    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "conductor".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register conductor");

    let session_id = "session-conductor-live";
    let run_id = "run-conductor-live";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "conductor".to_string(),
            goal: "manage worker agents".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start conductor session");

    // The conductor's system goal: it has capo tools; delegate via start_agent.
    let conductor_goal =
        "You are the capo conductor. You manage worker agents via the capo MCP tools \
         (start_agent, list_agents, ...). When the user asks for work, you MUST delegate \
         it by calling the start_agent tool with a precise `task` for the worker. Do NOT \
         do the work yourself.";
    // The tiny user chat message that should trigger exactly one start_agent call.
    let user_message =
        "Start an agent to create a file HELLO.txt containing exactly capo-works in the \
         project, then tell me it's done.";

    // Drive ONE live conductor turn. This BLOCKS while the real nested conductor
    // `claude` runs, dials our MCP endpoint, calls start_agent (which itself
    // blocks driving a real worker turn), and replies. Long timeout: two nested
    // npx+claude launches.
    let resp = server
        .handle(ServerRequest::cli(ServerCommand::RunConductorTurnLocal {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            turn_id: "turn-conductor-live".to_string(),
            user_message: user_message.to_string(),
            conductor_goal: conductor_goal.to_string(),
            mcp_url: mcp_url.clone(),
            mcp_headers: vec![("Authorization".to_string(), format!("Bearer {bearer}"))],
            acp_program: "npx".to_string(),
            acp_argv: vec![
                "-y".to_string(),
                "@zed-industries/claude-code-acp".to_string(),
            ],
            acp_session_mode: Some("default".to_string()),
            live_acp_opt_in: true,
        }))
        .expect("run live conductor turn");

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => panic!("expected AcpLiveTurn, got {other:?}"),
    };
    eprintln!(
        "live conductor turn: event_count={} appended={} stop_reason={:?}",
        summary.event_count, summary.appended_event_count, summary.stop_reason
    );

    // ASSERT (a): the conductor actually CALLED start_agent over capo's MCP.
    let calls = invocation_log.lock().expect("invocation log");
    let start_agent_calls: Vec<&ToolInvocation> =
        calls.iter().filter(|c| c.name == "start_agent").collect();
    assert!(
        !start_agent_calls.is_empty(),
        "the conductor must have CALLED start_agent over capo's MCP endpoint; \
         observed invocations: {:?}",
        calls.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
    );
    assert!(
        start_agent_calls.iter().any(|c| !c.is_error),
        "at least one start_agent call must have succeeded (isError:false); got: {:?}",
        start_agent_calls
    );
    drop(calls);

    // ASSERT (b): a real WORKER produced HELLO.txt == "capo-works" on disk in the
    // confined project workspace.
    let written = std::fs::read_to_string(project_ws.join("HELLO.txt")).expect(
        "the worker driven by the conductor's start_agent call must have written HELLO.txt \
         into the confined project workspace",
    );
    assert_eq!(
        written.trim_end_matches('\n'),
        "capo-works",
        "HELLO.txt must contain exactly `capo-works`; got {written:?}"
    );

    // Cross-check: the event log shows the conductor session registered and a
    // distinct WORKER session (the one start_agent created, NOT the conductor's)
    // ingested on-wire tool events through the loop's normal route. This proves the
    // write came from a real nested worker turn driven by the conductor's
    // start_agent call, not from the conductor itself.
    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe backlog");
    let kinds: Vec<&str> = backlog.events.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        kinds.contains(&"agent.registered"),
        "event log must show agent.registered; got {kinds:?}"
    );
    let worker_tool_events = backlog
        .events
        .iter()
        .filter(|e| {
            e.kind.starts_with("tool.")
                && e.session_id
                    .as_deref()
                    .is_some_and(|s| s != session_id && s.contains("worker"))
        })
        .count();
    assert!(
        worker_tool_events >= 1,
        "the conductor-driven WORKER session must have ingested at least one tool.* event \
         (its on-wire fs/write round-trip); tool sessions seen: {:?}",
        backlog
            .events
            .iter()
            .filter(|e| e.kind.starts_with("tool."))
            .map(|e| e.session_id.clone())
            .collect::<Vec<_>>()
    );
}
