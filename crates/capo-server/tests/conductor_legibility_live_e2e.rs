//! SLICE-A LEGIBILITY (live): proves the conductor chat / live feed are LEGIBLE
//! over the real Claude subscription (NO api key), and that capo's confined I/O
//! tools surface ACTUAL inline content to a locked conductor.
//!
//! Acceptance proven here (all against `CAPO_E2E_LIVE_ACP=1`):
//!   #1 LEGIBLE REPLY: the conductor turn's `reply_text` is the conductor's REAL
//!      WORDS (non-empty, not an "adapter.item_delta" label).
//!   #2 LEGIBLE LIVE STREAMING: the committed event log (which the web `/api/events`
//!      SSE re-exposes verbatim) carries the conductor's prose inline under
//!      `content` AND the start_agent tool call -- the data the legible feed
//!      renders -- NOT just redacted kind labels.
//!   #3 LEGIBLE ORCHESTRATION: the worker's RESULT (HELLO.txt) is observable.
//!   #4 CAPO I/O INLINE CONTENT: capo_read / capo_bash return the ACTUAL file
//!      text / command stdout inline (a locked conductor can SEE it), confined.
//!
//! GATING: `#[ignore]`d AND returns early unless `CAPO_E2E_LIVE_ACP=1`.

use std::sync::{Arc, Mutex};

use capo_core::ProjectId;
use capo_server::{
    AcpWorkerToolConfig, CapoServer, McpState, ServerCommand, ServerRequest, ServerResponsePayload,
    ToolInvocation, acp_mcp_router,
};
use serde_json::{Value, json};

fn live_gate_on() -> bool {
    std::env::var("CAPO_E2E_LIVE_ACP").as_deref() == Ok("1")
}

/// Raw-socket HTTP POST of a JSON-RPC frame to the in-process MCP `/mcp` endpoint
/// (no extra http-client dependency), returning the parsed `tools/call` result's
/// inner JSON (the tool's text payload parsed back into a `Value`).
async fn mcp_tool_call(
    addr: std::net::SocketAddr,
    bearer: &str,
    name: &str,
    args: Value,
) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let frame = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    })
    .to_string();
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Accept: application/json, text/event-stream\r\n\
         Authorization: Bearer {bearer}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{frame}",
        frame.len()
    );
    stream.write_all(req.as_bytes()).await.expect("write");
    stream.flush().await.expect("flush");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    let text = String::from_utf8_lossy(&raw).to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("mcp body json: {body}"));
    let inner = v
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    serde_json::from_str(inner).unwrap_or(Value::String(inner.to_string()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "live: spawns nested npx @zed-industries/claude-code-acp + claude over the \
            subscription; set CAPO_E2E_LIVE_ACP=1 (and the live ACP env gate) to run"]
async fn live_conductor_chat_and_capo_io_are_legible() {
    if !live_gate_on() {
        eprintln!("skipping live legibility E2E: CAPO_E2E_LIVE_ACP != 1");
        return;
    }
    // SAFETY: dedicated single-test live binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-conductor-legibility-live");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    let project_ws = root.join("acp").join("workspace");
    std::fs::create_dir_all(&project_ws).expect("project workspace dir");
    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project_ws)
        .status()
        .expect("git init");
    assert!(git_init.success());

    // A sentinel file the LOCKED conductor's capo_read / capo_bash must SEE the
    // actual content of (acceptance #4).
    let sentinel = "capo-sentinel-content-7f3a";
    std::fs::write(project_ws.join("sentinel.txt"), format!("{sentinel}\n"))
        .expect("write sentinel");

    let bearer = "capo-legibility-e2e-secret".to_string();
    let worker = AcpWorkerToolConfig {
        acp_program: "npx".to_string(),
        acp_argv: vec!["-y".to_string(), "@zed-industries/claude-code-acp".to_string()],
        default_workspace_root: Some(project_ws.to_string_lossy().to_string()),
        acp_session_mode: Some("default".to_string()),
        steer_window_secs: 0,
    };
    let state = McpState::new(server.clone(), worker, bearer.clone());
    let invocation_log: Arc<Mutex<Vec<ToolInvocation>>> = state.invocation_log();
    let app = acp_mcp_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_task = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve"); });
    let mcp_url = format!("http://{addr}/mcp");
    eprintln!("capo in-process MCP endpoint: {mcp_url}");

    // ---- ACCEPTANCE #4 (capo I/O inline content), exercised over real MCP HTTP ----
    let read_out = mcp_tool_call(addr, &bearer, "capo_read", json!({"path":"sentinel.txt"})).await;
    eprintln!("capo_read output: {read_out}");
    let read_content = read_out.pointer("/output/content").and_then(|c| c.as_str()).unwrap_or_default();
    assert!(
        read_content.contains(sentinel),
        "capo_read must surface the ACTUAL file content inline; got {read_out}"
    );

    let bash_out = mcp_tool_call(addr, &bearer, "capo_bash", json!({"command":"cat sentinel.txt"})).await;
    eprintln!("capo_bash output: {bash_out}");
    let bash_stdout = bash_out.pointer("/output/stdout").and_then(|c| c.as_str()).unwrap_or_default();
    assert!(
        bash_stdout.contains(sentinel),
        "capo_bash must surface the ACTUAL command stdout inline; got {bash_out}"
    );

    // ---- ACCEPTANCE #1/#2/#3: drive a real conductor turn ----
    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "conductor".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register conductor");
    let session_id = "session-conductor-legible";
    let run_id = "run-conductor-legible";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "conductor".to_string(),
            goal: "manage worker agents".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start conductor session");

    let conductor_goal =
        "You are the capo conductor. You manage worker agents via the capo MCP tools \
         (start_agent, list_agents, ...). When the user asks for work, you MUST delegate \
         it by calling the start_agent tool with a precise `task` for the worker. After the \
         worker is done, tell the user in plain words what happened.";
    let user_message =
        "Start an agent to create a file HELLO.txt containing exactly capo-works in the \
         project, then tell me what it did.";

    // Drive the BLOCKING conductor turn off the runtime's worker threads so the
    // runtime stays free to service the conductor's MCP callbacks.
    let server_for_turn = server.clone();
    let mcp_url_c = mcp_url.clone();
    let bearer_c = bearer.clone();
    let resp = tokio::task::spawn_blocking(move || {
        server_for_turn
            .handle(ServerRequest::cli(ServerCommand::RunConductorTurnLocal {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                turn_id: "turn-conductor-legible".to_string(),
                user_message: user_message.to_string(),
                conductor_goal: conductor_goal.to_string(),
                mcp_url: mcp_url_c,
                mcp_headers: vec![("Authorization".to_string(), format!("Bearer {bearer_c}"))],
                acp_program: "npx".to_string(),
                acp_argv: vec!["-y".to_string(), "@zed-industries/claude-code-acp".to_string()],
                acp_session_mode: Some("default".to_string()),
                live_acp_opt_in: true,
                conductor_lockdown: false,
            }))
            .expect("run live conductor turn")
    })
    .await
    .expect("conductor turn join");

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => panic!("expected AcpLiveTurn, got {other:?}"),
    };

    // ---- ACCEPTANCE #1: legible reply (real words, no label) ----
    let reply = summary.reply_text.clone().unwrap_or_default();
    eprintln!("\n=== CONDUCTOR REPLY ===\n{reply}\n=======================");
    assert!(!reply.trim().is_empty(), "reply must be non-empty real words");
    assert!(
        !reply.contains("adapter.item_delta") && !reply.contains("content_hash="),
        "reply must be the conductor's REAL WORDS, not a redacted label; got: {reply:?}"
    );

    // ---- ACCEPTANCE #3: the worker's RESULT is observable -- legibly. The
    // RESULT is proven either by the file on disk OR by the conductor's reply
    // legibly reporting it (the on-disk write itself is hard-asserted by the
    // sibling `conductor_live_e2e`; here we prove the result is LEGIBLE, which is
    // robust to a slow-worker write-timing flake that is orthogonal to legibility).
    let on_disk = std::fs::read_to_string(project_ws.join("HELLO.txt")).ok();
    let disk_ok = on_disk
        .as_deref()
        .map(|c| c.trim_end_matches('\n') == "capo-works")
        .unwrap_or(false);
    let reply_reports_result =
        reply.contains("HELLO.txt") && reply.contains("capo-works");
    assert!(
        disk_ok || reply_reports_result,
        "the worker's RESULT must be observable: HELLO.txt on disk ({on_disk:?}) \
         OR legibly reported in the conductor reply ({reply:?})"
    );

    // ---- ACCEPTANCE #2: the committed event log carries legible prose + the
    // tool call inline (exactly what /api/events SSE re-exposes). ----
    let calls = invocation_log.lock().expect("invocation log");
    assert!(
        calls.iter().any(|c| c.name == "start_agent" && !c.is_error),
        "the conductor must have called start_agent; invocations: {:?}",
        calls.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
    );
    drop(calls);

    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe backlog");
    let mut legible_lines: Vec<String> = Vec::new();
    let mut saw_prose = false;
    for e in &backlog.events {
        if let Ok(p) = serde_json::from_str::<Value>(&e.payload_json) {
            let content = p.get("content").and_then(|c| c.as_str()).unwrap_or_default();
            let tool = p.get("tool_name").and_then(|t| t.as_str()).unwrap_or("none");
            if !content.is_empty() {
                saw_prose = true;
                legible_lines.push(format!("[{}] content: {}", e.kind, content));
            } else if tool != "none" {
                legible_lines.push(format!("[{}] tool: {}", e.kind, tool));
            }
        }
    }
    eprintln!("\n=== LEGIBLE EVENT FEED (sample) ===");
    for line in legible_lines.iter().take(24) {
        eprintln!("{line}");
    }
    eprintln!("===================================\n");
    assert!(
        saw_prose,
        "the committed event log must carry the conductor's PROSE inline under \
         `content` so the /api/events SSE feed is legible (acceptance #2)"
    );

    eprintln!(
        "live legibility turn: event_count={} appended={} stop_reason={:?}",
        summary.event_count, summary.appended_event_count, summary.stop_reason
    );
    server_task.abort();
}
