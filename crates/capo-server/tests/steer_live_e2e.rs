//! LIVE STEERING: the end-to-end proof that `steer_agent` continues a PERSISTENT
//! live ACP worker session — cancel the in-flight prompt and re-`prompt` the SAME
//! session (the ACP spec's multi-turn continuation), no new `session/new`.
//!
//! Flow:
//!   1. Register an `acp` agent + StartSession (binds the session_id/run_id the
//!      `SteerAgent` command resolves and the steer registry keys on).
//!   2. Spawn a PERSISTENT `RunAcpLiveTurnLocal` worker turn (steer_window > 0) on
//!      a background thread. Initial goal: write `alpha.txt`.
//!   3. From the test thread, after the initial turn lands, issue `SteerAgent`
//!      with a follow-up goal: write `bravo.txt`. This flips the cancel flag and
//!      delivers the steer; the worker actor re-prompts the SAME session.
//!   4. `StopAgent` finalizes the session; join the thread.
//!   5. Assert `bravo.txt` exists — the steered (second) prompt ran on the one
//!      persistent session (the adapter does `session/new` exactly ONCE).
//!
//! GATING: `#[ignore]`d AND returns early unless `CAPO_E2E_LIVE_STEER=1` (slow: a
//! real `npx @zed-industries/claude-code-acp` + nested `claude` over the
//! subscription). Run with:
//!
//! ```text
//! export CAPO_E2E_LIVE_STEER=1 \
//!   CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1
//! cargo test -p capo-server --test steer_live_e2e -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use capo_core::ProjectId;
use capo_server::{CapoServer, ServerCommand, ServerRequest, ServerResponsePayload};

fn live_gate_on() -> bool {
    std::env::var("CAPO_E2E_LIVE_STEER").as_deref() == Ok("1")
}

#[test]
#[ignore = "live: spawns a real npx @zed-industries/claude-code-acp + claude worker over the \
            subscription; set CAPO_E2E_LIVE_STEER=1 (and the live ACP env gate) to run"]
fn steer_agent_continues_a_persistent_live_worker_session() {
    if !live_gate_on() {
        eprintln!(
            "skipping live steer E2E: CAPO_E2E_LIVE_STEER != 1 (this test only runs when \
             explicitly opted in)"
        );
        return;
    }

    // SAFETY: this is the only test in this dedicated live binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-steer-live-e2e");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The real bridge prefers a git repo project root.
    let project_ws = root.join("acp").join("workspace");
    std::fs::create_dir_all(&project_ws).expect("project workspace dir");
    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project_ws)
        .status()
        .expect("git init");
    assert!(git_init.success(), "git init must succeed");

    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "worker".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register worker");

    let session_id = "session-steer-live";
    let run_id = "run-steer-live";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "worker".to_string(),
            goal: "persistent steerable task".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start worker session");

    // Spawn the PERSISTENT (steer_window > 0) live worker turn. Initial goal:
    // write alpha.txt. The session then stays open for steers.
    let turn_server = server.clone();
    let ws = project_ws.to_string_lossy().to_string();
    let turn_done = Arc::new(AtomicBool::new(false));
    let turn_done_in_thread = Arc::clone(&turn_done);
    let handle = thread::spawn(move || {
        let resp = turn_server
            .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                goal: "Use your Write tool to create a file named `alpha.txt` in the current \
                       directory containing exactly the single word ALPHA. Then stop and wait."
                    .to_string(),
                turn_id: "turn-steer-live".to_string(),
                acp_program: "npx".to_string(),
                acp_argv: vec!["-y".to_string(), "@zed-industries/claude-code-acp".to_string()],
                workspace_root: Some(ws),
                live_acp_opt_in: true,
                acp_session_mode: Some("default".to_string()),
                mcp_url: None,
                mcp_headers: vec![],
                // Persistent + steerable: keep the session alive long enough to
                // receive the steer below.
                steer_window_secs: 120,
            }))
            .expect("run acp live turn");
        turn_done_in_thread.store(true, Ordering::Relaxed);
        resp
    });

    // Let the initial turn spawn + write alpha.txt, then STEER the SAME session
    // with a follow-up goal (write bravo.txt). SteerAgent flips the cancel flag
    // and delivers the steer; the actor re-prompts the persistent session.
    thread::sleep(Duration::from_secs(25));
    eprintln!("live steer: issuing SteerAgent follow-up (write bravo.txt)…");
    server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "worker".to_string(),
            goal: "Now use your Write tool to create a file named `bravo.txt` in the current \
                   directory containing exactly the single word BRAVO. Then stop and wait."
                .to_string(),
        }))
        .expect("steer worker");

    // Give the steered prompt time to run, then stop the session so the actor
    // finalizes promptly (instead of waiting out the steer window).
    thread::sleep(Duration::from_secs(35));
    eprintln!("live steer: issuing StopAgent to finalize…");
    server
        .handle(ServerRequest::cli(ServerCommand::StopAgent {
            agent_name: "worker".to_string(),
            reason: "steer smoke complete".to_string(),
        }))
        .expect("stop worker");

    let resp = handle.join().expect("worker thread joined");
    let ServerResponsePayload::AcpLiveTurn(summary) = resp.payload else {
        panic!("expected AcpLiveTurn response, got {:?}", resp.payload);
    };

    let alpha = project_ws.join("alpha.txt");
    let bravo = project_ws.join("bravo.txt");
    let alpha_exists = alpha.exists();
    let bravo_exists = bravo.exists();
    eprintln!(
        "live steer: stop_reason={:?} events={} alpha.txt={} bravo.txt={} (turn_done={})",
        summary.stop_reason,
        summary.event_count,
        alpha_exists,
        bravo_exists,
        turn_done.load(Ordering::Relaxed),
    );

    // The steered (second) prompt MUST have run on the persistent session: the
    // adapter performs `session/new` exactly once, so bravo.txt existing proves a
    // follow-up prompt continued the SAME live session.
    assert!(
        bravo_exists,
        "the steered follow-up prompt must have run on the persistent session \
         (bravo.txt should exist). alpha.txt={alpha_exists}"
    );
}
