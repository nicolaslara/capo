//! AI2: real-Codex chat reachable END-TO-END through the running server.
//!
//! These tests prove the production route the workpad's AI2 item was missing: a
//! user can `capo server agent register --adapter codex` and then `SendTask` /
//! `SteerAgent` against that agent and get REAL Codex output back -- routed by the
//! agent's binding through [`capo_adapters::CodexLiveAdapter`], not a fake summary.
//!
//! The deterministic test drives the SAME server path the CLI uses
//! ([`serve_tcp`] -> [`CapoServer::open`] -> per-request [`CapoServer::handle`])
//! over a real loopback TCP transport, with a deterministic absolute-path `codex`
//! STUB pinned via `CAPO_CODEX_BIN` and the live-provider gate opened. It asserts
//! the chat summary that flows back from `SendTask` is the STUB's parsed Codex
//! `agent_message` text -- NOT the fake-adapter summary -- and that a FAKE-bound
//! agent on the SAME server still routes through the fake adapter.
//!
//! Fail-closed-fast is proven too: a codex-bound agent's chat with the gate OFF
//! returns an IMMEDIATE typed error, fast, never spawning or blocking the server.
//!
//! The live opt-in smoke (`#[ignore]` + the explicit env gates) sends a trivial
//! goal to a codex-bound agent through the real server and asserts real Codex
//! output; it skips cleanly when the gates are unset or `codex` is unavailable.

use super::*;

use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Serializes the process-global env mutation (`CAPO_CODEX_BIN` + the two
/// live-provider opt-in gates) these tests perform, so concurrent test threads
/// never observe a half-set gate. Mirrors the controller crate's
/// `CODEX_LIVE_CHAT_ENV_LOCK`.
static CODEX_CHAT_ENV_LOCK: Mutex<()> = Mutex::new(());

const PREFLIGHT_GATE_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
const RUN_GATE_ENV: &str = "CAPO_SERVER_RUN_CODEX_LIVE";
const CODEX_BIN_ENV: &str = "CAPO_CODEX_BIN";

/// The fixed text the deterministic `codex` stub emits as its `agent_message`.
const CODEX_STUB_CHAT_SUMMARY: &str = "CODEX_STUB_E2E_CHAT_SUMMARY";

/// Write an executable absolute-path `codex` STUB that streams a fixed JSONL turn
/// (an `agent_message` + `turn.completed`) to stdout. The runtime spawns with
/// `env_clear()`, so the stub uses ONLY POSIX builtins (`read`/`printf`) and reads
/// its fixture from an absolute path. Returns the absolute stub path.
#[cfg(unix)]
fn write_codex_chat_stub(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).expect("stub dir");
    let fixture = dir.join("codex-chat-output.jsonl");
    let fixture_jsonl = format!(
        "{{\"type\":\"thread.started\",\"thread_id\":\"codex-e2e-thread\"}}\n\
{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item-1\",\"type\":\"agent_message\",\"text\":\"{CODEX_STUB_CHAT_SUMMARY}\"}}}}\n\
{{\"type\":\"turn.completed\"}}\n"
    );
    std::fs::write(&fixture, fixture_jsonl).expect("write fixture");
    let stub = dir.join("codex-chat-stub.sh");
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

/// Set the live-provider opt-in gate (both env vars) for the duration of `guard`.
/// SAFETY: every caller holds [`CODEX_CHAT_ENV_LOCK`] for the call's lifetime.
fn open_live_gate() {
    unsafe {
        std::env::set_var(PREFLIGHT_GATE_ENV, "1");
        std::env::set_var(RUN_GATE_ENV, "1");
    }
}

fn close_live_gate() {
    unsafe {
        std::env::remove_var(PREFLIGHT_GATE_ENV);
        std::env::remove_var(RUN_GATE_ENV);
    }
}

fn set_codex_bin(path: &str) {
    unsafe {
        std::env::set_var(CODEX_BIN_ENV, path);
    }
}

fn clear_codex_bin() {
    unsafe {
        std::env::remove_var(CODEX_BIN_ENV);
    }
}

/// Send `command` to the running server at `address` over the real TCP transport.
fn send(address: std::net::SocketAddr, request_id: &str, command: ServerCommand) -> ServerResponse {
    send_tcp(address, &ServerRequest::local_cli(request_id, command)).expect("send over tcp")
}

/// AI2 DETERMINISTIC END-TO-END: a codex-bound agent's `SendTask` chat output
/// flows back from the REAL server (`serve_tcp`/`CapoServer`) as the STUB's parsed
/// Codex text -- NOT a fake summary -- while a fake-bound agent on the SAME server
/// still routes through the fake adapter.
#[cfg(unix)]
#[test]
fn codex_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server() {
    let _guard: MutexGuard<'_, ()> = CODEX_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let root = temp_root();
    let stub = write_codex_chat_stub(&root.join("stub"));

    // The server reads `CAPO_CODEX_BIN` at `CapoServer::open` time (inside the
    // serve_tcp thread) and the live gate at chat time. Set both BEFORE serving.
    set_codex_bin(&stub);
    open_live_gate();

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    // The bounded server is sized to the exact number of connections this flow
    // opens (one connection per request): register-codex (1) + send-codex (1) +
    // status-codex (1) + register-fake (1) + send-fake (1) = 5.
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(5),
        )
        .expect("serve")
    });

    // 1) Register a CODEX-bound agent through the running server.
    let registered = send(
        address,
        "e2e-register-codex",
        ServerCommand::RegisterAgent {
            name: "codex-chat".to_string(),
            adapter: "codex".to_string(),
        },
    );
    assert_agent_registered(&registered, "codex-chat");

    // 2) SendTask: the chat turn drives the REAL Codex stub through the bound
    //    adapter. The returned external_session_ref is the codex-live binding's
    //    session ref -- proof the codex adapter ran, not the fake adapter.
    let sent = send(
        address,
        "e2e-send-codex",
        ServerCommand::SendTask {
            agent_name: "codex-chat".to_string(),
            goal: "Summarize the workpad through real Codex".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent for codex-chat");
    };
    assert_eq!(
        run.external_session_ref, "codex-live-chat-session-codex-chat",
        "codex-bound chat must use the real Codex adapter session ref, not the fake one"
    );
    assert_ne!(run.external_session_ref, "fake-adapter-session-codex-chat");

    // 3) AgentStatus: the persisted session summary is the STUB's parsed Codex
    //    agent_message text -- the load-bearing proof that REAL (stub) chat output
    //    flowed back, NOT a fake summary.
    let status = send(
        address,
        "e2e-status-codex",
        ServerCommand::AgentStatus {
            agent_name: "codex-chat".to_string(),
        },
    );
    let ServerResponsePayload::AgentStatus(agent) = status.payload else {
        panic!("expected agent status for codex-chat");
    };
    let session = agent
        .session
        .expect("codex-chat must have an active session");
    assert_eq!(
        session.latest_summary.as_deref(),
        Some(CODEX_STUB_CHAT_SUMMARY),
        "codex-bound chat summary must be the REAL stub output, not a fake summary"
    );
    assert_ne!(
        session.latest_summary.as_deref(),
        Some(
            "Fake adapter processed goal for codex-chat: Summarize the workpad through real Codex"
        )
    );

    // 4) A FAKE-bound agent on the SAME server still routes through the FAKE
    //    adapter -- binding is per-agent, Codex is not a global default.
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
    clear_codex_bin();
}

/// AI2 FAIL-CLOSED-FAST END-TO-END: with the live-provider gate OFF, a codex-bound
/// agent's `SendTask` through the running server returns an IMMEDIATE typed error
/// (the wire `unsupported`-free `codex live chat is fail-closed` message), fast,
/// never spawning the codex program (pinned to a non-existent path) nor blocking.
#[cfg(unix)]
#[test]
fn codex_bound_chat_fails_closed_fast_end_to_end_when_gate_is_off() {
    let _guard: MutexGuard<'_, ()> = CODEX_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Gate OFF, and pin codex to a program that must NEVER be spawned.
    close_live_gate();
    set_codex_bin("/nonexistent/codex-must-never-spawn");

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
            name: "codex-chat".to_string(),
            adapter: "codex".to_string(),
        },
    );

    // The codex-bound chat must FAIL CLOSED FAST: a typed transport error,
    // returned well under a second (no spawn, no wait, no hang).
    let started = Instant::now();
    let error = send_tcp(
        address,
        &ServerRequest::local_cli(
            "e2e-fc-send",
            ServerCommand::SendTask {
                agent_name: "codex-chat".to_string(),
                goal: "This must fail closed".to_string(),
                scenario: "default".to_string(),
            },
        ),
    )
    .expect_err("codex-bound chat must fail closed when the gate is off");
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "fail-closed chat must return fast (no spawn/wait), took {elapsed:?}"
    );
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("fail-closed") || rendered.contains("CodexLiveChat"),
        "the error must be the typed Codex live-chat fail-closed error, got: {rendered}"
    );

    assert_eq!(server_thread.join().expect("server thread"), 2);

    clear_codex_bin();
}

/// AI2 LIVE OPT-IN SMOKE: register a codex agent and send a trivial goal through
/// the REAL running server with BOTH `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and
/// `CAPO_SERVER_RUN_CODEX_LIVE=1`; assert real Codex output flows back.
///
/// `#[ignore]`d and gated on the explicit env opt-in; it skips cleanly (passing)
/// when the gates are unset or `codex` is unavailable, so it is never fatal for
/// operators who have not opted in.
///
/// Run with:
///   `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 \`
///   `  cargo test -p capo-server -- --ignored codex_live_chat_smoke`
#[test]
#[ignore = "live Codex chat smoke: set CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1"]
fn codex_live_chat_smoke() {
    let _guard: MutexGuard<'_, ()> = CODEX_CHAT_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let preflight = std::env::var(PREFLIGHT_GATE_ENV).as_deref() == Ok("1");
    let run = std::env::var(RUN_GATE_ENV).as_deref() == Ok("1");
    if !(preflight && run) {
        eprintln!(
            "skipping live Codex chat smoke: set {PREFLIGHT_GATE_ENV}=1 {RUN_GATE_ENV}=1 to run it"
        );
        return;
    }
    // No `CAPO_CODEX_BIN` override: resolve the real `codex` from PATH. If it is
    // not installed, skip cleanly (this opt-in smoke is never fatal for unavailable
    // codex).
    if std::env::var_os(CODEX_BIN_ENV).is_none()
        && std::process::Command::new("codex")
            .arg("--version")
            .output()
            .map(|out| !out.status.success())
            .unwrap_or(true)
    {
        eprintln!("skipping live Codex chat smoke: `codex` is not available on PATH");
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
            name: "codex-live".to_string(),
            adapter: "codex".to_string(),
        },
    );
    let sent = send(
        address,
        "smoke-send",
        ServerCommand::SendTask {
            agent_name: "codex-live".to_string(),
            goal: "Reply with the single word: capo".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run_refs) = sent.payload else {
        panic!("expected task sent for codex-live");
    };
    assert_eq!(
        run_refs.external_session_ref, "codex-live-chat-session-codex-live",
        "the live codex chat must run through the real Codex adapter binding"
    );

    let status = send(
        address,
        "smoke-status",
        ServerCommand::AgentStatus {
            agent_name: "codex-live".to_string(),
        },
    );
    let ServerResponsePayload::AgentStatus(agent) = status.payload else {
        panic!("expected agent status for codex-live");
    };
    let session = agent.session.expect("codex-live must have a session");
    let summary = session
        .latest_summary
        .expect("a live codex chat turn must produce a summary");
    assert!(
        !summary.is_empty()
            && summary
                != "Fake adapter processed goal for codex-live: Reply with the single word: capo",
        "the live codex chat summary must be real Codex output, got: {summary:?}"
    );

    assert_eq!(server_thread.join().expect("server thread"), 3);
}
