//! DT4a streaming-resume tests: the `Subscribe { from_sequence }` event tail
//! resumes across a connectivity drop on the client (or runner) leg with NO gap
//! and NO duplicate, reusing the EXACT in-tree seam guarantee
//! (`EventStream::delivered_through` watermark; live events strictly greater than
//! the watermark) and the `streaming-transport` restart-resume property.
//!
//! These are the deterministic-first tests the DT4a section requires. They use a
//! DETERMINISTIC drop seam -- dropping the in-process `EventStream` and opening a
//! fresh `Subscribe { from_sequence = delivered_through }`, exactly what a
//! reconnecting `subscribe_tcp` client does after its connection dies -- and NO
//! wall-clock sleeps: every event is produced by scripted server commands, never
//! a live provider, and continuity is asserted by sequence arithmetic.
//!
//! Scope honesty (per the workpad): this covers resume of events ALREADY COMMITTED
//! to the server's log. Events a runner BUFFERED while disconnected are out of
//! scope here and are reconciled by DT4b.

use capo_state::SqliteStateStore;

use super::*;
use crate::ServerResponsePayload;

/// Every committed sequence in the store, in order, read straight from the log.
fn all_sequences(root: &std::path::Path) -> Vec<i64> {
    SqliteStateStore::open(root)
        .expect("state")
        .events_after(0, 100_000)
        .expect("events_after(0)")
        .into_iter()
        .map(|event| event.sequence)
        .collect()
}

/// Drive a write-bearing command and return the committed sequences it added,
/// so a test can seed a deterministic backlog without a live provider.
fn steer(server: &CapoServer, agent: &str, goal: &str) {
    server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: agent.to_string(),
            goal: goal.to_string(),
        }))
        .expect("steer");
}

/// Assert a delivered sequence is a gap-free, duplicate-free forward read of the
/// full committed log: strictly increasing, contiguous, and exactly the log.
fn assert_no_gap_no_dupe(delivered: &[i64], full_log: &[i64]) {
    // No duplicate: every delivered sequence is distinct.
    let mut unique = delivered.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        delivered.len(),
        "duplicate sequence across the drop/resume seam: {delivered:?}"
    );
    // Strictly increasing across the seam (no reorder).
    assert!(
        delivered.windows(2).all(|pair| pair[0] < pair[1]),
        "delivered sequence is not strictly increasing across the seam: {delivered:?}"
    );
    // No gap: the union of pre-drop + post-resume equals the full committed log.
    assert_eq!(
        delivered, full_log,
        "the union of pre-drop and post-resume events must equal the full committed log"
    );
}

#[test]
fn client_drop_mid_stream_resumes_from_watermark_with_no_gap_and_no_dupe() {
    // A client tails the log, is force-dropped mid-stream (the in-process
    // `EventStream` is dropped, exactly as a dead `subscribe_tcp` connection),
    // and reconnects with `from_sequence = delivered_through`. The union of the
    // pre-drop and post-resume events equals the full committed sequence with no
    // gap and no duplicate.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "operator".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "operator".to_string(),
            goal: "seed the backlog".to_string(),
            scenario: "default".to_string(),
        },
    );

    // --- First connection: catch up on the backlog, then take SOME live events.
    let (backlog, mut stream) = server.subscribe(None, 0).expect("subscribe from 0");
    let mut delivered: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert!(!delivered.is_empty(), "expected a non-empty seed backlog");
    assert_eq!(stream.delivered_through(), backlog.next_sequence);

    // Commit two live events; deliver them on the first connection.
    steer(&server, "operator", "first live redirect");
    steer(&server, "operator", "second live redirect");
    let live_before_drop: Vec<i64> = stream.next_batch().iter().map(|e| e.sequence).collect();
    assert!(
        !live_before_drop.is_empty(),
        "expected live events before the drop"
    );
    delivered.extend(live_before_drop.iter().copied());

    // The watermark the reconnecting client will resume from.
    let resume_from = stream.delivered_through();

    // --- DETERMINISTIC DROP: the connection dies mid-stream. Commit MORE events
    // while the client is disconnected (these are the ones a naive resume could
    // gap or duplicate).
    drop(stream);
    steer(&server, "operator", "redirect during the drop A");
    steer(&server, "operator", "redirect during the drop B");

    // --- RESUME: a fresh Subscribe from the last delivered watermark.
    let (resume_backlog, mut resumed) = server
        .subscribe(None, resume_from)
        .expect("resume subscribe from delivered_through");
    // The resumed backlog re-delivers every committed event STRICTLY AFTER the
    // watermark and NONE at or below it -- the exact seam guarantee.
    assert!(
        resume_backlog
            .events
            .iter()
            .all(|e| e.sequence > resume_from),
        "resume re-delivered an event at or below the watermark: {resume_from}"
    );
    delivered.extend(resume_backlog.events.iter().map(|e| e.sequence));

    // Anything committed after the resume snapshot arrives live on the new stream
    // (none here, but draining proves no spurious re-delivery).
    delivered.extend(resumed.next_batch().iter().map(|e| e.sequence));

    assert_no_gap_no_dupe(&delivered, &all_sequences(&root));
}

#[test]
fn resume_is_durable_across_a_server_restart_with_identical_continuation() {
    // The reconnect is durable across a server restart too: after the server
    // restarts and rebuilds read models from the durable event log, a client
    // resuming from `from_sequence` sees the IDENTICAL continuation (ST11
    // restart-resume), with no gap and no duplicate back to the pre-restart tail.
    let root = temp_root();
    let resume_from;
    let mut delivered: Vec<i64>;
    let continuation_before_restart;
    {
        let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
        handle(
            &server,
            ServerCommand::RegisterAgent {
                name: "operator".to_string(),
                adapter: "fake".to_string(),
            },
        );
        handle(
            &server,
            ServerCommand::SendTask {
                agent_name: "operator".to_string(),
                goal: "seed before restart".to_string(),
                scenario: "default".to_string(),
            },
        );

        let (backlog, mut stream) = server.subscribe(None, 0).expect("subscribe");
        delivered = backlog.events.iter().map(|e| e.sequence).collect();
        steer(&server, "operator", "pre-restart live");
        delivered.extend(stream.next_batch().iter().map(|e| e.sequence));
        resume_from = stream.delivered_through();

        // Commit events that fall AFTER the resume watermark BEFORE capturing the
        // pre-restart continuation, so the captured continuation is NON-EMPTY and
        // the cross-restart prefix equality below is a real assertion, not a
        // vacuous `[..0] == [][..]` (finding 2). These are the events whose
        // identical re-delivery after a restart is the whole point of the test.
        steer(&server, "operator", "committed near restart A");
        steer(&server, "operator", "committed near restart B");

        // Capture what a resume WOULD see on the same (pre-restart) server, so we
        // can prove the post-restart resume is byte-identical over this overlap.
        let (pre, _pre_stream) = server.subscribe(None, resume_from).expect("pre-resume");
        continuation_before_restart = pre.events.clone();
        assert!(
            !continuation_before_restart.is_empty(),
            "test setup error: the pre-restart continuation must be non-empty so the \
             cross-restart prefix equality is a real assertion"
        );
        // Server dropped here: the store reopens below on the same root.
    }

    // --- Restart: reopen on the SAME root; read models rebuild from the log.
    let restarted = CapoServer::open(ProjectId::new("project-capo"), &root).expect("reopen");
    let (resumed_backlog, mut resumed) = restarted
        .subscribe(None, resume_from)
        .expect("resume after restart");

    // The continuation a resuming subscriber sees AFTER the restart is identical
    // to what it would have seen before (where they overlap), and re-delivers
    // only events strictly after the watermark.
    assert!(
        resumed_backlog
            .events
            .iter()
            .all(|e| e.sequence > resume_from),
        "post-restart resume re-delivered an event at or below the watermark"
    );
    let overlap_len = continuation_before_restart.len();
    assert!(
        overlap_len > 0,
        "test setup error: overlap must be non-trivial or the prefix equality proves nothing"
    );
    assert_eq!(
        &resumed_backlog.events[..overlap_len],
        &continuation_before_restart[..],
        "the resume continuation must be identical across the restart"
    );

    delivered.extend(resumed_backlog.events.iter().map(|e| e.sequence));
    delivered.extend(resumed.next_batch().iter().map(|e| e.sequence));
    assert_no_gap_no_dupe(&delivered, &all_sequences(&root));
}

#[test]
fn a_stale_from_sequence_is_served_the_full_backlog_after_that_point() {
    // A reconnect that presents a STALE `from_sequence` (well behind the head) is
    // served correctly: it re-delivers the entire backlog strictly after that
    // point, contiguous and duplicate-free. This is the "reconnect with an old
    // cursor" case, distinct from the ahead-of-log rejection below.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "operator".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "operator".to_string(),
            goal: "seed".to_string(),
            scenario: "default".to_string(),
        },
    );
    steer(&server, "operator", "more A");
    steer(&server, "operator", "more B");

    let full = all_sequences(&root);
    assert!(full.len() >= 3, "expected several events: {full:?}");

    // A stale cursor pinned at the very first committed sequence.
    let stale = full[0];
    let (backlog, _stream) = server.subscribe(None, stale).expect("stale resume");
    let resumed: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert_eq!(
        resumed,
        full[1..].to_vec(),
        "a stale cursor must re-serve exactly the backlog strictly after it"
    );
    assert!(
        resumed.iter().all(|seq| *seq > stale),
        "no event at or below the stale watermark may be re-served"
    );
}

#[test]
fn a_from_sequence_ahead_of_the_log_is_rejected_as_invalid() {
    // A `from_sequence` STRICTLY AHEAD of the committed log head cannot be served
    // (no such continuation exists) and is rejected as invalid -- never silently
    // returned as an empty backlog that would mask a client cursor bug. The exact
    // head, and head+anything, are the boundary.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "operator".to_string(),
            adapter: "fake".to_string(),
        },
    );

    let head = *all_sequences(&root).last().expect("at least one event");

    // At the head: valid (an empty backlog, resuming exactly at the tail).
    let (at_head, _stream) = server.subscribe(None, head).expect("subscribe at head");
    assert!(at_head.events.is_empty());
    assert_eq!(at_head.next_sequence, head);

    // Ahead of the head: rejected as invalid.
    let ahead = head + 1;
    let error = server
        .subscribe(None, ahead)
        .expect_err("subscribe ahead of the log must be rejected");
    match error {
        ServerError::SubscribeFromSequenceAheadOfLog {
            from_sequence,
            latest_sequence,
        } => {
            assert_eq!(from_sequence, ahead);
            assert_eq!(latest_sequence, head);
        }
        other => panic!("expected SubscribeFromSequenceAheadOfLog, got {other:?}"),
    }

    // Far ahead is rejected the same way (not just off-by-one).
    assert!(matches!(
        server.subscribe(None, head + 10_000),
        Err(ServerError::SubscribeFromSequenceAheadOfLog { .. })
    ));
}

#[test]
fn empty_log_accepts_a_zero_cursor_but_rejects_a_positive_one() {
    // Boundary: with an EMPTY log the head is 0. A fresh subscriber resumes from
    // 0 (valid, empty backlog); any positive cursor is ahead of the (empty) log
    // and is rejected.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    let (backlog, _stream) = server
        .subscribe(None, 0)
        .expect("subscribe from 0 on empty log");
    assert!(backlog.events.is_empty());
    assert_eq!(backlog.next_sequence, 0);

    assert!(matches!(
        server.subscribe(None, 1),
        Err(ServerError::SubscribeFromSequenceAheadOfLog {
            from_sequence: 1,
            latest_sequence: 0,
        })
    ));
}

#[test]
fn resume_rides_the_committed_event_broadcast_not_a_readmodel_snapshot() {
    // Scope guard: resume rides the discrete committed-event backlog/broadcast,
    // NOT a periodic read-model dump. We assert this structurally: the resumed
    // backlog carries DISCRETE EventRecords (each with its own sequence/event_id),
    // is strictly ordered by sequence, and the resumed live stream's
    // `delivered_through` advances only by committed-event sequences.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "operator".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "operator".to_string(),
            goal: "seed".to_string(),
            scenario: "default".to_string(),
        },
    );

    let (backlog, stream) = server.subscribe(None, 0).expect("subscribe");
    // Each backlog entry is a discrete committed event with a unique id and a
    // strictly-increasing sequence -- not a single coalesced snapshot blob.
    let ids: std::collections::HashSet<&str> =
        backlog.events.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(
        ids.len(),
        backlog.events.len(),
        "backlog must be discrete committed events, not a coalesced snapshot"
    );
    assert!(
        backlog
            .events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence),
        "backlog events must be in strict commit order"
    );
    // The watermark equals the last committed sequence (a per-event cursor), not
    // a wall-clock or snapshot-version stamp.
    assert_eq!(
        stream.delivered_through(),
        backlog.events.last().map(|e| e.sequence).unwrap_or(0),
    );
}

/// The three-role resume contract holds over the REAL `subscribe_tcp` client
/// (not just the in-process `EventStream`): a TCP tail is dropped mid-stream and
/// a fresh `subscribe_tcp` from `delivered_through` continues with no gap/dupe.
///
/// DETERMINISTIC TIMING (finding 3): there is NO wall-clock read timeout and NO
/// `Instant`/deadline drain loop. Every event the test asserts on is committed by
/// a synchronous `send_tcp` BEFORE the `subscribe_tcp` that must see it, so the
/// event arrives in the catch-up BACKLOG (a synchronous request/response read),
/// not on the live tail. The watermark to resume from is the backlog's
/// `next_sequence`. The seam is the deterministic connection DROP, never a sleep.
#[test]
fn subscribe_tcp_client_resumes_over_a_real_connection_drop() {
    use std::net::TcpListener;
    use std::thread;

    use crate::{send_tcp, serve_tcp, subscribe_tcp};

    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address").to_string();
    let server_root = root.clone();
    // 1 register + 1 first-tail + 1 send + 1 resume-tail = 4 connections.
    let server = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(4),
        )
        .expect("serve")
    });

    // --- Commit the seed event FIRST (synchronous request/response), so the
    // first tail observes it in its catch-up backlog -- no live-tail draining.
    let registered = send_tcp(
        &address,
        &ServerRequest::local_cli(
            "dt4a-register",
            ServerCommand::RegisterAgent {
                name: "tail-agent".to_string(),
                adapter: "fake".to_string(),
            },
        ),
    )
    .expect("register");
    assert!(matches!(
        registered.payload,
        ServerResponsePayload::AgentRegistered(_)
    ));

    // --- First connection: tail from 0; the backlog already holds the committed
    // agent.registered. The watermark to resume from is the backlog head.
    let (backlog, stream) = subscribe_tcp(&address, None, 0).expect("subscribe");
    let mut delivered: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert!(
        !delivered.is_empty(),
        "the first tail backlog must carry the pre-committed agent.registered"
    );
    let resume_from = backlog.next_sequence;

    // --- DETERMINISTIC DROP: close the TCP tail, commit MORE while disconnected.
    drop(stream);
    let sent = send_tcp(
        &address,
        &ServerRequest::local_cli(
            "dt4a-send",
            ServerCommand::SendTask {
                agent_name: "tail-agent".to_string(),
                goal: "task committed during the drop".to_string(),
                scenario: "default".to_string(),
            },
        ),
    )
    .expect("send task");
    assert!(matches!(sent.payload, ServerResponsePayload::TaskSent(_)));

    // --- RESUME: fresh subscribe_tcp from the watermark; backlog carries the
    // events committed during the drop, strictly after the watermark.
    let (resume_backlog, resumed) =
        subscribe_tcp(&address, None, resume_from).expect("resume subscribe");
    assert!(
        resume_backlog
            .events
            .iter()
            .all(|e| e.sequence > resume_from),
        "the resume backlog must re-deliver only events strictly after the watermark"
    );
    delivered.extend(resume_backlog.events.iter().map(|e| e.sequence));

    // No gap, no dupe across the real connection drop.
    let full = all_sequences(&root);
    // `delivered` is the union of pre-drop tail + post-resume backlog; it equals
    // the committed log up to the resume backlog's head.
    let mut unique = delivered.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), delivered.len(), "duplicate across the drop");
    assert!(
        delivered.windows(2).all(|pair| pair[0] < pair[1]),
        "delivered not strictly increasing across the drop: {delivered:?}"
    );
    let head_after_resume = *delivered.iter().max().expect("nonempty");
    let expected: Vec<i64> = full
        .into_iter()
        .filter(|s| *s <= head_after_resume)
        .collect();
    assert_eq!(
        delivered, expected,
        "the union of pre-drop tail and post-resume backlog must be the gap-free committed log"
    );

    // Drop the resume tail so its server-side connection thread sees EOF and the
    // bounded accept loop can drain and return promptly (no idle-timeout wait).
    drop(resumed);
    server.join().expect("server thread");
}

/// DT4a (finding 6): the ahead-of-log rejection holds OVER THE WIRE, not just in
/// process. A `subscribe_tcp` with `from_sequence = head + 1` returns the typed
/// `TransportError::Remote { kind: "subscribe_from_sequence_ahead_of_log" }` over
/// the real TCP connection, so a typed client can branch on the exact kind.
#[test]
fn subscribe_tcp_ahead_of_log_is_rejected_over_the_wire() {
    use std::net::TcpListener;
    use std::thread;

    use crate::{TransportError, send_tcp, serve_tcp, subscribe_tcp};

    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address").to_string();
    let server_root = root.clone();
    // 1 register + 1 ahead-subscribe (rejected) + 1 at-head subscribe = 3.
    let server = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(3),
        )
        .expect("serve")
    });

    // Commit one event so the log head is positive.
    send_tcp(
        &address,
        &ServerRequest::local_cli(
            "dt4a-ahead-register",
            ServerCommand::RegisterAgent {
                name: "ahead-agent".to_string(),
                adapter: "fake".to_string(),
            },
        ),
    )
    .expect("register");
    let head = *all_sequences(&root).last().expect("at least one event");

    // Ahead of the head: rejected over the wire with the exact typed kind.
    let error = subscribe_tcp(&address, None, head + 1)
        .err()
        .expect("subscribe ahead of the log must be rejected over the wire");
    match error {
        TransportError::Remote { kind, message } => {
            assert_eq!(kind, "subscribe_from_sequence_ahead_of_log");
            assert!(
                message.contains(&(head + 1).to_string()),
                "the wire message should name the offending cursor: {message}"
            );
        }
        other => {
            panic!("expected Remote{{ kind: subscribe_from_sequence_ahead_of_log }}, got {other:?}")
        }
    }

    // At the head: still valid over the wire (an empty backlog).
    let (at_head, stream) =
        subscribe_tcp(&address, None, head).expect("subscribe at head over tcp");
    assert!(at_head.events.is_empty());
    drop(stream);
    server.join().expect("server thread");
}

/// DT4a (finding 1): the resume guarantee holds across a transport drop on the
/// RUNNER leg, not only the client leg. Per DT-D1 the runner is "a special client
/// that owns processes" and resumes by the SAME `Subscribe { from_sequence }`
/// sequence watermark -- it holds no authoritative state to lose. A runner-side
/// subscriber that tails its session's runtime events is force-dropped mid-stream
/// and re-subscribes from `delivered_through`; the union of pre-drop and
/// post-resume events for that session is gap-free and duplicate-free.
#[test]
fn runner_leg_drop_resumes_from_watermark_with_no_gap_and_no_dupe() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "runner-agent".to_string(),
            adapter: "fake".to_string(),
        },
    );
    // A session the runner leg tails (runner liveness affects this session's
    // process truth). SendTask opens the session and seeds runtime events.
    let task = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "runner-agent".to_string(),
            goal: "runner seeds the backlog".to_string(),
            scenario: "default".to_string(),
        },
    );
    let session_id = match task.payload {
        ServerResponsePayload::TaskSent(ref sent) => sent.session_id.as_str().to_string(),
        ref other => panic!("expected TaskSent, got {other:?}"),
    };

    // --- Runner-side subscription: SESSION-SCOPED tail from 0 (the runner leg
    // tails the session it owns the process for), then take some live events.
    let (backlog, mut runner_stream) = server
        .subscribe(Some(session_id.clone()), 0)
        .expect("runner subscribe from 0");
    let mut delivered: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert!(
        !delivered.is_empty(),
        "expected a non-empty session backlog for the runner leg"
    );

    steer(&server, "runner-agent", "first runner-leg redirect");
    let live_before_drop: Vec<i64> = runner_stream
        .next_batch()
        .iter()
        .map(|e| e.sequence)
        .collect();
    delivered.extend(live_before_drop.iter().copied());
    let resume_from = runner_stream.delivered_through();

    // --- DETERMINISTIC DROP of the RUNNER leg: drop the runner's EventStream
    // (exactly as a dead runner<->server connection), commit MORE while the
    // runner is disconnected.
    drop(runner_stream);
    steer(&server, "runner-agent", "redirect during runner drop A");
    steer(&server, "runner-agent", "redirect during runner drop B");

    // --- RUNNER RE-SUBSCRIBE from delivered_through (the watermark), session
    // scoped exactly as before. No gap, no dupe for this session's events.
    let (resume_backlog, mut resumed) = server
        .subscribe(Some(session_id.clone()), resume_from)
        .expect("runner resume subscribe from delivered_through");
    assert!(
        resume_backlog
            .events
            .iter()
            .all(|e| e.sequence > resume_from),
        "runner resume re-delivered an event at or below the watermark: {resume_from}"
    );
    delivered.extend(resume_backlog.events.iter().map(|e| e.sequence));
    delivered.extend(resumed.next_batch().iter().map(|e| e.sequence));

    // The runner leg's union must equal the FULL set of this session's committed
    // events -- gap-free, duplicate-free -- the same watermark guarantee the
    // client leg gets, proving the resume holds on either leg.
    let session_log: Vec<i64> = SqliteStateStore::open(&root)
        .expect("state")
        .events_after_for_session(&capo_core::SessionId::new(session_id), 0, 100_000)
        .expect("session events")
        .into_iter()
        .map(|event| event.sequence)
        .collect();
    assert_no_gap_no_dupe(&delivered, &session_log);
}
