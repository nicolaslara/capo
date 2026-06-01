//! DP4: real-Claude chat reachable END-TO-END through the running server.
//!
//! Mirrors `codex_chat.rs` for the SECOND real provider: a user can
//! `capo server agent register --adapter claude` and then `SendTask` against
//! that agent and get REAL Claude `stream-json` output back -- routed by the
//! agent's binding through [`capo_adapters::ClaudeCodeLiveAdapter`], not a fake
//! summary.
//!
//! The deterministic test drives the SAME server path the CLI uses over a real
//! loopback TCP transport, with a deterministic absolute-path `claude` STUB
//! pinned via `CAPO_CLAUDE_BIN` and the Claude live gate opened. It asserts the
//! chat summary that flows back from `SendTask` is the STUB's parsed Claude
//! assistant text -- NOT the fake-adapter summary -- and that a fake-bound agent
//! on the SAME server still routes through the fake adapter.
//!
//! Fail-closed-fast is proven too: a claude-bound agent's chat with the gate OFF
//! returns an IMMEDIATE typed error, fast, never spawning or blocking the server.

use super::*;

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Serializes the process-global env mutation (`CAPO_CLAUDE_BIN` + the two
/// live-provider opt-in gates) these tests perform, so concurrent test threads
/// never observe a half-set gate.
static CLAUDE_CHAT_ENV_LOCK: Mutex<()> = Mutex::new(());

const PREFLIGHT_GATE_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
const CLAUDE_RUN_GATE_ENV: &str = "CAPO_SERVER_RUN_CLAUDE_LIVE";
const CLAUDE_BIN_ENV: &str = "CAPO_CLAUDE_BIN";

/// The fixed text the deterministic `claude` stub emits as its assistant message.
const CLAUDE_STUB_CHAT_SUMMARY: &str = "CLAUDE_STUB_E2E_CHAT_SUMMARY";

/// Write an executable absolute-path `claude` STUB that streams a fixed
/// `stream-json` turn (a `system` start, an `assistant` message, a `result`) to
/// stdout. The runtime spawns with `env_clear()`, so the stub uses ONLY POSIX
/// builtins and reads its fixture from an absolute path. Returns the stub path.
#[cfg(unix)]
fn write_claude_chat_stub(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).expect("stub dir");
    let fixture = dir.join("claude-chat-output.jsonl");
    let fixture_jsonl = format!(
        "{{\"type\":\"system\",\"session_id\":\"claude-e2e-sess\"}}\n\
{{\"type\":\"assistant\",\"session_id\":\"claude-e2e-sess\",\"message\":{{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{CLAUDE_STUB_CHAT_SUMMARY}\"}}]}}}}\n\
{{\"type\":\"result\",\"session_id\":\"claude-e2e-sess\",\"subtype\":\"success\"}}\n"
    );
    std::fs::write(&fixture, fixture_jsonl).expect("write fixture");
    let stub = dir.join("claude-chat-stub.sh");
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{}'\n",
        fixture.display()
    );
    std::fs::write(&stub, script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

fn open_live_gate() {
    unsafe {
        std::env::set_var(PREFLIGHT_GATE_ENV, "1");
        std::env::set_var(CLAUDE_RUN_GATE_ENV, "1");
    }
}

fn close_live_gate() {
    unsafe {
        std::env::remove_var(PREFLIGHT_GATE_ENV);
        std::env::remove_var(CLAUDE_RUN_GATE_ENV);
    }
}

fn set_claude_bin(path: &str) {
    unsafe {
        std::env::set_var(CLAUDE_BIN_ENV, path);
    }
}

fn clear_claude_bin() {
    unsafe {
        std::env::remove_var(CLAUDE_BIN_ENV);
    }
}

fn send(address: std::net::SocketAddr, request_id: &str, command: ServerCommand) -> ServerResponse {
    send_tcp(address, &ServerRequest::local_cli(request_id, command)).expect("send over tcp")
}

/// DP4 DETERMINISTIC END-TO-END: a claude-bound agent's `SendTask` chat output
/// flows back from the REAL server as the STUB's parsed Claude text -- NOT a
/// fake summary -- while a fake-bound agent on the SAME server still routes
/// through the fake adapter.
#[cfg(unix)]
#[test]
fn claude_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server() {
    let _guard: MutexGuard<'_, ()> = CLAUDE_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let root = temp_root();
    let stub = write_claude_chat_stub(&root.join("stub"));

    set_claude_bin(&stub);
    open_live_gate();

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    // register-claude (1) + send-claude (1) + status-claude (1) +
    // register-fake (1) + send-fake (1) = 5.
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(5),
        )
        .expect("serve")
    });

    // 1) Register a CLAUDE-bound agent through the running server.
    let registered = send(
        address,
        "e2e-register-claude",
        ServerCommand::RegisterAgent {
            name: "claude-chat".to_string(),
            adapter: "claude".to_string(),
        },
    );
    assert_agent_registered(&registered, "claude-chat");

    // 2) SendTask: the chat turn drives the REAL Claude stub through the bound
    //    adapter. The external_session_ref is the claude-live binding's session
    //    ref -- proof the claude adapter ran, not the fake adapter.
    let sent = send(
        address,
        "e2e-send-claude",
        ServerCommand::SendTask {
            agent_name: "claude-chat".to_string(),
            goal: "Edit the workspace through real Claude".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent for claude-chat");
    };
    assert_eq!(
        run.external_session_ref, "claude-live-session-claude-chat",
        "claude-bound chat must use the real Claude adapter session ref, not the fake one"
    );
    assert_ne!(run.external_session_ref, "fake-adapter-session-claude-chat");

    // 3) AgentStatus: the persisted session summary is the STUB's parsed Claude
    //    assistant text -- the load-bearing proof real (stub) chat output flowed
    //    back, NOT a fake summary.
    let status = send(
        address,
        "e2e-status-claude",
        ServerCommand::AgentStatus {
            agent_name: "claude-chat".to_string(),
        },
    );
    let ServerResponsePayload::AgentStatus(agent) = status.payload else {
        panic!("expected agent status for claude-chat");
    };
    let session = agent
        .session
        .expect("claude-chat must have an active session");
    assert_eq!(
        session.latest_summary.as_deref(),
        Some(CLAUDE_STUB_CHAT_SUMMARY),
        "claude-bound chat summary must be the REAL stub output, not a fake summary"
    );

    // 4) A FAKE-bound agent on the SAME server still routes through the FAKE
    //    adapter -- binding is per-agent, Claude is not a global default.
    let registered_fake = send(
        address,
        "e2e-register-fake",
        ServerCommand::RegisterAgent {
            name: "fake-chat".to_string(),
            adapter: "fake".to_string(),
        },
    );
    assert_agent_registered(&registered_fake, "fake-chat");
    let sent_fake = send(
        address,
        "e2e-send-fake",
        ServerCommand::SendTask {
            agent_name: "fake-chat".to_string(),
            goal: "Stay fake".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run_fake) = sent_fake.payload else {
        panic!("expected task sent for fake-chat");
    };
    assert_eq!(
        run_fake.external_session_ref, "fake-adapter-session-fake-chat",
        "a fake-bound agent must keep the fake adapter session ref"
    );

    assert_eq!(server_thread.join().expect("server thread"), 5);

    close_live_gate();
    clear_claude_bin();
}

/// DP4 FAIL-CLOSED-FAST END-TO-END: with the Claude live gate OFF, a
/// claude-bound agent's `SendTask` through the running server returns an
/// IMMEDIATE typed error, fast, never spawning the claude program (pinned to a
/// non-existent path) nor blocking.
#[cfg(unix)]
#[test]
fn claude_bound_chat_fails_closed_fast_end_to_end_when_gate_is_off() {
    let _guard: MutexGuard<'_, ()> = CLAUDE_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    close_live_gate();
    set_claude_bin("/nonexistent/claude-must-never-spawn");

    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(2),
        )
        .expect("serve")
    });

    send(
        address,
        "e2e-fc-register",
        ServerCommand::RegisterAgent {
            name: "claude-chat".to_string(),
            adapter: "claude".to_string(),
        },
    );

    let started = Instant::now();
    let error = send_tcp(
        address,
        &ServerRequest::local_cli(
            "e2e-fc-send",
            ServerCommand::SendTask {
                agent_name: "claude-chat".to_string(),
                goal: "This must fail closed".to_string(),
                scenario: "default".to_string(),
            },
        ),
    )
    .expect_err("claude-bound chat must fail closed when the gate is off");
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "fail-closed chat must return fast (no spawn/wait), took {elapsed:?}"
    );
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("fail-closed") || rendered.contains("CodexLiveChat"),
        "the error must be the typed live-chat fail-closed error, got: {rendered}"
    );

    assert_eq!(server_thread.join().expect("server thread"), 2);

    clear_claude_bin();
}

/// DP4 LIVE OPT-IN SMOKE: register a claude agent and send a trivial goal through
/// the REAL running server with BOTH `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and
/// `CAPO_SERVER_RUN_CLAUDE_LIVE=1`; assert real Claude output flows back.
///
/// `#[ignore]`d and gated on the explicit env opt-in (DP11 lives here); it skips
/// cleanly when the gates are unset or `claude` is unavailable, so it is never
/// fatal for operators who have not opted in.
#[test]
#[ignore = "live Claude chat smoke: set CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CLAUDE_LIVE=1"]
fn claude_live_chat_smoke() {
    let _guard: MutexGuard<'_, ()> = CLAUDE_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let preflight = std::env::var(PREFLIGHT_GATE_ENV).as_deref() == Ok("1");
    let run = std::env::var(CLAUDE_RUN_GATE_ENV).as_deref() == Ok("1");
    if !(preflight && run) {
        eprintln!(
            "skipping live Claude chat smoke: set {PREFLIGHT_GATE_ENV}=1 {CLAUDE_RUN_GATE_ENV}=1 to run it"
        );
        return;
    }
    if std::env::var_os(CLAUDE_BIN_ENV).is_none()
        && std::process::Command::new("claude")
            .arg("--version")
            .output()
            .map(|out| !out.status.success())
            .unwrap_or(true)
    {
        eprintln!("skipping live Claude chat smoke: `claude` is not available on PATH");
        return;
    }

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
        .expect("serve")
    });

    send(
        address,
        "smoke-register",
        ServerCommand::RegisterAgent {
            name: "claude-live".to_string(),
            adapter: "claude".to_string(),
        },
    );
    let sent = send(
        address,
        "smoke-send",
        ServerCommand::SendTask {
            agent_name: "claude-live".to_string(),
            goal: "Reply with the single word: capo".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run_refs) = sent.payload else {
        panic!("expected task sent for claude-live");
    };
    assert_eq!(
        run_refs.external_session_ref, "claude-live-session-claude-live",
        "the live claude chat must run through the real Claude adapter binding"
    );

    let status = send(
        address,
        "smoke-status",
        ServerCommand::AgentStatus {
            agent_name: "claude-live".to_string(),
        },
    );
    let ServerResponsePayload::AgentStatus(agent) = status.payload else {
        panic!("expected agent status for claude-live");
    };
    let session = agent.session.expect("claude-live must have a session");
    let summary = session
        .latest_summary
        .expect("a live claude chat turn must produce a summary");
    assert!(
        !summary.is_empty(),
        "the live claude chat summary must be real output"
    );

    assert_eq!(server_thread.join().expect("server thread"), 3);
}
