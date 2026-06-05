//! SLICE-A / Layer 1: boot the in-process STATELESS HTTP MCP server on a
//! 127.0.0.1 ephemeral port and drive it over real HTTP JSON-RPC.
//!
//! Asserts the stateless MCP wire contract and the capo-tools dispatch:
//!   - `GET /mcp` -> 405 (the statelessness switch claude-code-acp requires).
//!   - `initialize` negotiates a protocol version + advertises tools capability.
//!   - `tools/list` advertises `start_agent` + `list_agents` (and the rest).
//!   - `tools/call start_agent` triggers a worker ACP turn that writes the
//!     OBSERVED file into the confined workspace (via the DETERMINISTIC `/bin/sh`
//!     ACP stub — NOT the live `npx` bridge; this layer stays deterministic).
//!   - `tools/call list_agents` then returns the registered worker agent.
//!
//! The worker turn is driven through the same `RunAcpLiveTurnLocal` server seam
//! `acp_dispatch_smoke.rs` exercises, so the live ACP env gate is opened here.

use capo_core::ProjectId;
use capo_server::{AcpWorkerToolConfig, CapoServer, McpState, acp_mcp_router};
use serde_json::{Value, json};

/// The same `/bin/sh` ACP write-stub `acp_dispatch_smoke.rs` uses: on
/// `session/prompt` it writes a file into its confined cwd and finalizes
/// `end_turn`.
fn write_file_acp_agent_stub(dir: &std::path::Path, out_file: &str, contents: &str) -> String {
    std::fs::create_dir_all(dir).expect("stub dir");
    let stub = dir.join("acp-write-agent-stub.sh");
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
      printf '%s' "{contents}" > "{out_file}"
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

/// A minimal raw HTTP/1.1 client: send one POST /mcp (or GET) and read the
/// status line + body. Avoids pulling in a heavy HTTP client dep for the test.
async fn http_request(
    addr: std::net::SocketAddr,
    method: &str,
    bearer: &str,
    json_body: Option<&Value>,
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let body = json_body.map(|v| v.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Accept: application/json, text/event-stream\r\n"
    );
    if !bearer.is_empty() {
        req.push_str(&format!("Authorization: Bearer {bearer}\r\n"));
    }
    if json_body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    stream.write_all(req.as_bytes()).await.expect("write");
    stream.flush().await.expect("flush");

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status code");
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn mcp_http_server_advertises_and_dispatches_capo_tools() {
    // Open the live ACP gate the RunAcpLiveTurnLocal seam self-checks (the worker
    // turn the start_agent tool drives goes through that seam). SAFETY: this is
    // the only test in this binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-mcp-http-l1");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    let workspace = root.join("acp-ws");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let stub_dir = root.join("acp-stub");
    let program = write_file_acp_agent_stub(&stub_dir, "out.txt", "hi from acp");

    let bearer = "test-bearer-token".to_string();
    let worker = AcpWorkerToolConfig {
        acp_program: program,
        acp_argv: Vec::new(),
        default_workspace_root: Some(workspace.to_string_lossy().to_string()),
        acp_session_mode: None, // deterministic stub path
    };
    let state = McpState::new(server.clone(), worker, bearer.clone());
    let app = acp_mcp_router(state);

    // Boot on a 127.0.0.1 ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    // GET /mcp -> 405 (statelessness switch).
    let (status, _) = http_request(addr, "GET", &bearer, None).await;
    assert_eq!(status, 405, "GET /mcp must be 405 Method Not Allowed");

    // initialize.
    let init = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "x"}}
    });
    let (status, body) = http_request(addr, "POST", &bearer, Some(&init)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("initialize json");
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(v["result"]["capabilities"]["tools"]["listChanged"], false);

    // notifications/initialized -> 202.
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let (status, _) = http_request(addr, "POST", &bearer, Some(&note)).await;
    assert_eq!(status, 202, "notification must be 202 Accepted");

    // bad auth -> 401.
    let (status, _) = http_request(addr, "POST", "wrong-token", Some(&init)).await;
    assert_eq!(status, 401, "bad bearer token must be 401");

    // tools/list advertises start_agent + list_agents (and the rest).
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let (status, body) = http_request(addr, "POST", &bearer, Some(&list)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("tools/list json");
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("tool name"))
        .collect();
    assert!(names.contains(&"start_agent"), "tools/list must advertise start_agent; got {names:?}");
    assert!(names.contains(&"list_agents"), "tools/list must advertise list_agents; got {names:?}");
    assert!(names.contains(&"review_agent"));
    assert!(names.contains(&"steer_agent"));
    assert!(names.contains(&"set_mode"));

    // tools/call start_agent -> drives a worker ACP turn that writes the file.
    let call = json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "start_agent", "arguments": {"task": "write a file", "name": "acp-worker"}}
    });
    let (status, body) = http_request(addr, "POST", &bearer, Some(&call)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("tools/call json");
    assert_eq!(v["result"]["isError"], false, "start_agent must succeed: {body}");
    let result_text = v["result"]["content"][0]["text"].as_str().expect("result text");
    let summary: Value = serde_json::from_str(result_text).expect("start_agent summary json");
    assert_eq!(summary["stop_reason"], "end_turn", "worker turn must finalize end_turn");
    assert_eq!(summary["name"], "acp-worker");

    // OBSERVED file the worker wrote into the confined workspace.
    let written = std::fs::read_to_string(workspace.join("out.txt"))
        .expect("worker must have written the observed file into the confined workspace");
    assert_eq!(written, "hi from acp");

    // tools/call list_agents -> the registered worker is present.
    let call = json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "list_agents", "arguments": {}}
    });
    let (status, body) = http_request(addr, "POST", &bearer, Some(&call)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("list_agents json");
    assert_eq!(v["result"]["isError"], false);
    let result_text = v["result"]["content"][0]["text"].as_str().expect("list text");
    let listing: Value = serde_json::from_str(result_text).expect("list_agents summary json");
    let agents = listing["agents"].as_array().expect("agents array");
    assert!(
        agents.iter().any(|a| a["name"] == "acp-worker"),
        "list_agents must include the registered worker; got {listing}"
    );

    // unknown method -> JSON-RPC -32601.
    let bad = json!({"jsonrpc": "2.0", "id": 9, "method": "bogus/method", "params": {}});
    let (status, body) = http_request(addr, "POST", &bearer, Some(&bad)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("error json");
    assert_eq!(v["error"]["code"], -32601, "unknown method must be -32601");

    server_task.abort();
}

/// `start_agent {detached:true}` returns IMMEDIATELY with status:running (the
/// depth-discipline responsiveness contract — the conductor is not blocked on the
/// worker turn), and the worker turn still runs on a background thread and
/// produces the OBSERVED file. Deterministic (/bin/sh stub worker, no live bridge).
#[tokio::test]
async fn start_agent_detached_returns_running_and_writes_in_background() {
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }
    let root = capo_tmptest::TempRoot::new("capo-mcp-http-detached");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");
    let workspace = root.join("acp-ws");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let program = write_file_acp_agent_stub(&root.join("acp-stub"), "out.txt", "hi from acp");

    let bearer = "test-bearer-token".to_string();
    let worker = AcpWorkerToolConfig {
        acp_program: program,
        acp_argv: Vec::new(),
        default_workspace_root: Some(workspace.to_string_lossy().to_string()),
        acp_session_mode: None,
    };
    let app = acp_mcp_router(McpState::new(server.clone(), worker, bearer.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_task = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve"); });

    // The observed file must NOT exist before the call.
    assert!(!workspace.join("out.txt").exists());

    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "start_agent", "arguments": {"task": "write a file", "name": "w", "detached": true}}
    });
    let (status, body) = http_request(addr, "POST", &bearer, Some(&call)).await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["result"]["isError"], false, "detached start must succeed: {body}");
    let text = v["result"]["content"][0]["text"].as_str().expect("text");
    let summary: Value = serde_json::from_str(text).expect("summary json");
    // The defining contract: it returned status:running (NOT a completed turn with
    // a stop_reason) — i.e. the conductor was not blocked on the worker turn.
    assert_eq!(summary["status"], "running", "detached returns running: {summary}");
    assert_eq!(summary["detached"], true);
    assert!(summary.get("stop_reason").is_none(), "detached must not carry a completed turn outcome");

    // The background worker thread still drives the turn and writes the file.
    let mut wrote = false;
    for _ in 0..50 {
        if std::fs::read_to_string(workspace.join("out.txt")).ok().as_deref() == Some("hi from acp") {
            wrote = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(wrote, "the detached worker must write the observed file in the background within 5s");

    server_task.abort();
}
