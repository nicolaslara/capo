//! ST11 CLI integration: the operator `thread` command drives sequence-driven
//! incremental updates over the persistent connection (the `Subscribe`
//! event-tail contract), not a `latest_summary` snapshot poll.
//!
//! Deterministic, no live provider: a fake adapter agent produces real turn
//! events through the running server process, then the control REPL renders the
//! thread read model and the incremental tail of newly-committed events. A
//! repeated `thread` resumes from the per-session watermark, so it shows only what
//! is new since the prior read.
//!
//! These tests run the server with a large `--max-requests` and shut it down
//! explicitly (`kill`) rather than waiting for it to self-stop after a fixed
//! connection count: the `thread` command opens an indeterminate number of
//! connections (a `Subscribe` tail plus status/read-thread round-trips), so the
//! exact count is not pinned here -- the control output is the assertion surface.

use std::io::BufReader;

use super::support::*;

#[test]
fn control_thread_renders_incrementally_via_subscribe_over_the_persistent_connection() {
    let state_root = temp_root("control-thread-stream-state");
    let mut server = spawn_server(&state_root, 1_000);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "thread-agent",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(register.contains("server_agent_registered=true"));

    // A fake task creates a session and turn events to project a thread from.
    let send = capo([
        "server",
        "task",
        "send",
        "--agent",
        "thread-agent",
        "--goal",
        "Produce a turn to thread",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(send.contains("server_task_sent=true"));

    // First `thread`: renders the full projected thread from sequence 0 (the
    // structural read model). A second `thread` resumes from the watermark the
    // first advanced to, so with no new events it reports the incremental
    // no-new-events tail -- proving repeated reads are sequence-driven, not a
    // re-fetch of the whole snapshot each time.
    let script = "\
attach thread-agent
thread
thread
quit
";
    let output = capo_with_env_and_stdin(
        [
            "control",
            "--planner",
            "none",
            "--connect",
            &address,
            "--state",
            &state,
        ],
        [],
        script,
    );

    assert!(output.contains("Attached to thread-agent."));
    // The projected multi-turn thread is rendered (ST5 read model).
    assert!(
        output.contains("Thread"),
        "expected a rendered thread:\n{output}"
    );
    // The second `thread` resumed from the watermark and found nothing newer, so
    // it printed the incremental no-new-events tail (genuinely sequence-driven, not
    // a full snapshot re-render with no notion of "new since last read").
    assert!(
        output.contains("Streamed (no new events since last read)"),
        "expected the incremental no-new-events tail on the repeated read:\n{output}"
    );

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn control_thread_streams_new_events_committed_between_reads() {
    let state_root = temp_root("control-thread-stream-new-state");
    let mut server = spawn_server(&state_root, 1_000);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "thread-stream-agent",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(register.contains("server_agent_registered=true"));
    let send = capo([
        "server",
        "task",
        "send",
        "--agent",
        "thread-stream-agent",
        "--goal",
        "Initial turn",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(send.contains("server_task_sent=true"));

    // Read the thread once (advancing the watermark), then steer the agent (which
    // commits a `session.redirected` event strictly after the watermark), then
    // read the thread again: the second read must show the new event streamed
    // incrementally since the prior read.
    let script = "\
attach thread-stream-agent
thread
send Please continue the work
thread
quit
";
    let output = capo_with_env_and_stdin(
        [
            "control",
            "--planner",
            "none",
            "--connect",
            &address,
            "--state",
            &state,
        ],
        [],
        script,
    );

    assert!(output.contains("Attached to thread-stream-agent."));
    assert!(output.contains("Sent to thread-stream-agent"));
    // The repeated `thread` after the steer streamed at least one new event since
    // the prior read (the incremental tail), not a full re-snapshot.
    assert!(
        output.contains("Streamed (") && output.contains("new events)"),
        "expected newly-committed events on the incremental tail after a steer:\n{output}"
    );

    let _ = server.kill();
    let _ = server.wait();
}
