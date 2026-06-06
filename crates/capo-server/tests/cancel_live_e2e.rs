//! COOPERATIVE CANCEL (B2): the LIVE end-to-end proof that an
//! `InterruptAgent`/`StopAgent` command cooperatively cancels an in-flight live
//! ACP worker turn.
//!
//! Flow:
//!   1. Register an `acp` agent + StartSession (binds session_id + run_id to the
//!      agent, the key `InterruptAgent` resolves and the in-flight registry uses).
//!   2. Spawn a long-running `RunAcpLiveTurnLocal` worker turn on a BACKGROUND
//!      thread (a goal that keeps the agent emitting `session/update`s so the
//!      between-frames cancel check fires promptly).
//!   3. From the test thread, issue `InterruptAgent`. This flips the registered
//!      cancel flag; the worker's wire pump observes it between frames, sends a
//!      best-effort `session/cancel` notification, and the turn terminates.
//!   4. Assert the turn's terminal `stop_reason == "cancelled"`.
//!
//! GATING: `#[ignore]`d AND returns early unless `CAPO_E2E_LIVE_CANCEL=1` (slow:
//! a real `npx @zed-industries/claude-code-acp` + nested `claude` launch over the
//! subscription). Run with:
//!
//! ```text
//! export CAPO_E2E_LIVE_CANCEL=1 \
//!   CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1
//! cargo test -p capo-server --test cancel_live_e2e -- --ignored --nocapture
//! ```
//!
//! NOTE on granularity: cooperative cancel is observed BETWEEN frames (or at the
//! per-read deadline). The worker goal below deliberately asks for ongoing output
//! so the flag is observed promptly; a worker wedged inside a single blocking
//! read would only observe cancel at the next frame or the read timeout.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use capo_core::ProjectId;
use capo_server::{
    CapoServer, ServerCommand, ServerRequest, ServerResponsePayload,
};

fn live_gate_on() -> bool {
    std::env::var("CAPO_E2E_LIVE_CANCEL").as_deref() == Ok("1")
}

#[test]
#[ignore = "live: spawns a real npx @zed-industries/claude-code-acp + claude worker over the \
            subscription; set CAPO_E2E_LIVE_CANCEL=1 (and the live ACP env gate) to run"]
fn interrupt_agent_cooperatively_cancels_a_live_worker_turn() {
    if !live_gate_on() {
        eprintln!(
            "skipping live cancel E2E: CAPO_E2E_LIVE_CANCEL != 1 (this test only runs when \
             explicitly opted in)"
        );
        return;
    }

    // SAFETY: this is the only test in this dedicated live binary.
    unsafe {
        std::env::set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1");
        std::env::set_var("CAPO_SERVER_RUN_ACP_LIVE", "1");
    }

    let root = capo_tmptest::TempRoot::new("capo-cancel-live-e2e");
    let server = CapoServer::open(ProjectId::new("project-capo"), root.path()).expect("server");

    // The worker runs in the server's project-dir ACP workspace; the real bridge
    // prefers a git repo (Claude Code expects a project root), so pre-create +
    // git-init it.
    let project_ws = root.join("acp").join("workspace");
    std::fs::create_dir_all(&project_ws).expect("project workspace dir");
    let git_init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project_ws)
        .status()
        .expect("git init");
    assert!(git_init.success(), "git init must succeed");

    // Register + start an acp worker session (binds session_id + run_id to the
    // agent, which InterruptAgent resolves and the in-flight registry keys on).
    server
        .handle(ServerRequest::cli(ServerCommand::RegisterAgent {
            name: "worker".to_string(),
            adapter: "acp".to_string(),
        }))
        .expect("register worker");

    let session_id = "session-cancel-live";
    let run_id = "run-cancel-live";
    server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: "worker".to_string(),
            goal: "do a long task".to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some(run_id.to_string()),
        }))
        .expect("start worker session");

    // Spawn the (blocking) live worker turn on a background thread. The goal keeps
    // the agent producing ongoing output so the between-frames cancel check fires.
    let turn_server = server.clone();
    let ws = project_ws.to_string_lossy().to_string();
    let turn_done = Arc::new(AtomicBool::new(false));
    let turn_done_in_thread = Arc::clone(&turn_done);
    let handle = thread::spawn(move || {
        let resp = turn_server
            .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                goal: "Count slowly from 1 to 500, printing each number on its own \
                       line with a short explanation, and keep going until told to stop."
                    .to_string(),
                turn_id: "turn-cancel-live".to_string(),
                acp_program: "npx".to_string(),
                acp_argv: vec![
                    "-y".to_string(),
                    "@zed-industries/claude-code-acp".to_string(),
                ],
                workspace_root: Some(ws),
                live_acp_opt_in: true,
                acp_session_mode: Some("default".to_string()),
            }))
            .expect("run acp live turn");
        turn_done_in_thread.store(true, Ordering::Relaxed);
        resp
    });

    // Give the worker time to spawn and begin emitting frames, then interrupt.
    // (If it already finished, the interrupt is a no-op and the assert below
    // simply checks whatever terminal reason the turn returned.)
    thread::sleep(Duration::from_secs(8));
    server
        .handle(ServerRequest::cli(ServerCommand::InterruptAgent {
            agent_name: "worker".to_string(),
            reason: "operator requested cancel".to_string(),
        }))
        .expect("interrupt worker");

    let resp = handle.join().expect("worker thread joined");
    let ServerResponsePayload::AcpLiveTurn(summary) = resp.payload else {
        panic!("expected AcpLiveTurn response, got {:?}", resp.payload);
    };

    eprintln!(
        "live cancel: stop_reason={:?} events={} (turn_done_naturally={})",
        summary.stop_reason,
        summary.event_count,
        turn_done.load(Ordering::Relaxed),
    );
    assert_eq!(
        summary.stop_reason.as_deref(),
        Some("cancelled"),
        "the interrupted live turn must terminate with stop_reason=cancelled"
    );
}
