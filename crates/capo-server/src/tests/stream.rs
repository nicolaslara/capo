//! ST11 deterministic stream wire-snapshot tests: a fake/scripted agent drives
//! committed events through the persistent connection and the tests assert the
//! exact JSON-RPC frames/notifications a client observes for a scripted turn.
//!
//! All deterministic, no live provider:
//!
//! - The exact-frame tests run an in-process `serve_tcp_with_handler` with a
//!   [`ScriptedTailHandler`] backed by a real `SqliteStateStore` the test appends
//!   scripted events to. The handler's `subscribe` opens a real broadcast
//!   subscription + backlog (the same discipline `CapoServer::subscribe` uses), so
//!   the live tail is gap- and dup-free, and the test asserts the verbatim
//!   `subscribed` response frame plus each live `event` notification frame.
//! - The end-to-end client-seam test runs a real `serve_tcp` server and uses the
//!   public `subscribe_tcp` client: subscribe-with-backlog, then live tail as a
//!   real write-bearing command commits an event, then an in-band cancel that ends
//!   the tail, then redaction-on-emit over the live wire.
//! - The restart/replay test proves the thread projection and a `from_sequence`
//!   subscriber resume identically after a server restart.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use capo_state::{EventKind, NewEvent, RedactionState, SqliteStateStore};

use super::*;
use crate::event_tail::EventStream;
use crate::transport::{RequestHandler, ServeConfig, serve_tcp_with_handler};
use crate::{
    CancellationToken, ServerEvent, SubscriptionBacklog, TransportError, send_tcp, subscribe_tcp,
};

const POLL_DEADLINE: Duration = Duration::from_secs(5);
const TAIL_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A scripted, no-live-provider handler for the exact-frame tail tests. It owns a
/// real `SqliteStateStore` so the tests can append scripted committed events and
/// observe the exact frames the live tail pump emits. The `subscribe` method
/// mirrors `CapoServer::subscribe` (subscribe to the broadcast, then snapshot the
/// backlog) so the seam is gap-free and duplicate-free, exactly like production.
struct ScriptedTailHandler {
    store: Arc<SqliteStateStore>,
    interrupts: Arc<Mutex<Vec<(String, String)>>>,
}

impl ScriptedTailHandler {
    fn new(store: Arc<SqliteStateStore>) -> Self {
        Self {
            store,
            interrupts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn interrupt_log(&self) -> Arc<Mutex<Vec<(String, String)>>> {
        Arc::clone(&self.interrupts)
    }
}

impl RequestHandler for ScriptedTailHandler {
    fn handle(
        &self,
        request: ServerRequest,
        _cancel: &CancellationToken,
    ) -> Result<ServerResponse, TransportError> {
        // The scripted handler only needs to serve `Subscribe` (routed through
        // `subscribe`/the tail) for these tests; any other request is a protocol
        // error so a test that sends one fails loudly rather than silently.
        Err(TransportError::Protocol(format!(
            "scripted tail handler does not serve {:?}",
            request.command
        )))
    }

    fn subscribe(
        &self,
        session_id: Option<String>,
        from_sequence: i64,
    ) -> Result<(SubscriptionBacklog, EventStream), TransportError> {
        // Subscribe to the broadcast BEFORE snapshotting the backlog (the gap-free
        // discipline), then build the same backlog + live stream shape production
        // does.
        let subscription = self.store.event_broadcaster().subscribe();
        let records = match &session_id {
            Some(session) => self
                .store
                .events_after_for_session(
                    &capo_core::SessionId::new(session.clone()),
                    from_sequence,
                    4096,
                )
                .expect("backlog"),
            None => self
                .store
                .events_after(from_sequence, 4096)
                .expect("backlog"),
        };
        let next_sequence = records
            .last()
            .map(|record| record.sequence)
            .unwrap_or(from_sequence);
        let events = records.into_iter().map(ServerEvent::from_record).collect();
        let backlog = SubscriptionBacklog {
            session_id: session_id.clone(),
            from_sequence,
            next_sequence,
            events,
        };
        let stream = EventStream::new(subscription, next_sequence, session_id);
        Ok((backlog, stream))
    }

    fn supports_subscription(&self) -> bool {
        true
    }

    fn interrupt(&self, session_id: &str, reason: &str) {
        self.interrupts
            .lock()
            .expect("interrupt log")
            .push((session_id.to_string(), reason.to_string()));
    }
}

/// Append a scripted committed event to a store, returning its sequence. This is
/// the "fake agent" producing one item of a scripted turn.
fn script_event(
    store: &SqliteStateStore,
    event_id: &str,
    kind: EventKind,
    session_id: &str,
    turn_id: &str,
    payload: &str,
    redaction_state: RedactionState,
) -> i64 {
    let mut event = NewEvent::new(event_id, kind, "fake-agent");
    event.project_id = Some(ProjectId::new("project-capo"));
    event.session_id = Some(SessionId::new(session_id));
    event.turn_id = Some(turn_id.to_string());
    event.payload_json = payload.to_string();
    event.redaction_state = redaction_state;
    store
        .append_event(event, &[])
        .expect("append scripted event")
}

/// A running scripted in-process tail server: its loopback address, the handler's
/// interrupt log, and the bounded accept loop's join handle. The test must drop
/// every client connection so the server drains and `server` joins.
struct ScriptedTailServer {
    address: String,
    interrupts: Arc<Mutex<Vec<(String, String)>>>,
    server: thread::JoinHandle<usize>,
}

/// Start a scripted in-process tail server. `max_connections` sizes the bounded
/// accept loop; the test must drop every client connection so the server drains
/// and the join returns.
fn start_scripted_tail_server(
    store: Arc<SqliteStateStore>,
    max_connections: usize,
) -> ScriptedTailServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address").to_string();
    let handler = Arc::new(ScriptedTailHandler::new(store));
    let interrupts = handler.interrupt_log();
    let server = thread::spawn(move || {
        serve_tcp_with_handler(
            listener,
            handler,
            Some(max_connections),
            ServeConfig::default(),
        )
        .expect("serve scripted tail")
    });
    ScriptedTailServer {
        address,
        interrupts,
        server,
    }
}

/// Poll `probe` until it returns `Some` or the deadline elapses.
fn poll_until<T>(probe: impl Fn() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if start.elapsed() >= POLL_DEADLINE {
            return None;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Read the next live `event` notification frame off a [`subscribe_tcp`] stream,
/// returning the VERBATIM wire line the server emitted (so a test can assert the
/// exact byte shape) plus the decoded event. Bounded by a read timeout so a hung
/// tail fails the test.
fn next_tail_frame(stream: &mut crate::SubscribeStream) -> (String, ServerEvent) {
    stream
        .set_read_timeout(Some(TAIL_READ_TIMEOUT))
        .expect("set tail read timeout");
    stream
        .next_event_frame()
        .expect("read live tail frame")
        .expect("a live event, not EOF")
}

#[test]
fn scripted_turn_streams_exact_backlog_and_live_event_frames() {
    // A fake agent scripts a multi-item turn: a backlog item committed BEFORE the
    // subscribe, then two live items committed AFTER. The test asserts the exact
    // `subscribed` backlog frame and the exact `event` notification frames the
    // live tail emits, with no gap and no duplicate at the seam.
    let root = temp_root();
    let store = Arc::new(SqliteStateStore::open(&root).expect("store"));
    let session = "session-scripted-turn";
    let turn = "turn-scripted-1";

    // One committed event before the subscribe -> the catch-up backlog.
    let backlog_seq = script_event(
        &store,
        "event-turn-started",
        EventKind::SessionSummaryUpdated,
        session,
        turn,
        "{\"text\":\"starting the scripted turn\"}",
        RedactionState::Safe,
    );

    let ScriptedTailServer {
        address, server, ..
    } = start_scripted_tail_server(Arc::clone(&store), 1);

    let (backlog, mut stream) =
        subscribe_tcp(&address, Some(session.to_string()), 0).expect("subscribe");

    // Exact backlog: one event, strictly the pre-subscribe one, and the resume
    // watermark is its sequence.
    assert_eq!(backlog.session_id.as_deref(), Some(session));
    assert_eq!(backlog.from_sequence, 0);
    assert_eq!(backlog.next_sequence, backlog_seq);
    assert_eq!(backlog.events.len(), 1);
    let backlog_event = &backlog.events[0];
    assert_eq!(backlog_event.event_id, "event-turn-started");
    assert_eq!(backlog_event.sequence, backlog_seq);
    assert_eq!(
        backlog_event.payload_json,
        "{\"text\":\"starting the scripted turn\"}"
    );

    // Now script two more turn items AFTER subscribing: they must arrive live, in
    // order, as `event` notifications strictly after the backlog watermark.
    let live_one = script_event(
        &store,
        "event-turn-output",
        EventKind::ToolObservationRecorded,
        session,
        turn,
        "{\"text\":\"scripted tool observation\"}",
        RedactionState::Safe,
    );
    let live_two = script_event(
        &store,
        "event-turn-finished",
        EventKind::EvidenceRecorded,
        session,
        turn,
        "{\"text\":\"scripted turn finished\"}",
        RedactionState::Safe,
    );
    assert!(live_one > backlog_seq && live_two > live_one);

    // First live frame: assert the EXACT JSON-RPC notification wire bytes the
    // server emitted. The `event` object keys are serialized in JSON sorted-key
    // order by the codec's `serde_json::Value`, which is what pins the byte shape.
    let (frame_one, event_one) = next_tail_frame(&mut stream);
    let expected_one = format!(
        concat!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"event\",\"params\":{{\"event\":{{",
            "\"actor\":\"fake-agent\",\"agent_id\":null,",
            "\"event_id\":\"event-turn-output\",\"item_id\":null,",
            "\"kind\":\"tool.observation_recorded\",",
            "\"payload_json\":\"{{\\\"text\\\":\\\"scripted tool observation\\\"}}\",",
            "\"project_id\":\"project-capo\",\"redaction_state\":\"safe\",",
            "\"run_id\":null,\"sequence\":{seq},\"session_id\":\"{session}\",",
            "\"task_id\":null,\"turn_id\":\"{turn}\"}}}}}}"
        ),
        seq = live_one,
        session = session,
        turn = turn,
    );
    assert_eq!(
        frame_one, expected_one,
        "first live tail frame must match the exact JSON-RPC notification shape"
    );
    assert_eq!(event_one.event_id, "event-turn-output");
    assert_eq!(event_one.sequence, live_one);

    // Second live frame: the turn-finished item, again strictly after.
    let (frame_two, event_two) = next_tail_frame(&mut stream);
    assert!(
        frame_two.contains("\"event_id\":\"event-turn-finished\""),
        "second live frame: {frame_two}"
    );
    assert!(frame_two.contains("\"kind\":\"evidence.recorded\""));
    assert_eq!(event_two.sequence, live_two);
    assert!(
        event_two.sequence > event_one.sequence,
        "live events arrive in strictly increasing sequence (no gap, no dup)"
    );

    // Close the tail so the server drains and the bounded accept loop returns.
    drop(stream);
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn subscribe_tail_has_no_gap_and_no_duplicate_at_the_backlog_to_live_seam_over_the_wire() {
    // The seam guarantee proven end-to-end over the socket: a subscribe from 0
    // catches up the whole backlog, and a burst of events committed right after
    // arrives live with every sequence delivered exactly once and in order.
    let root = temp_root();
    let store = Arc::new(SqliteStateStore::open(&root).expect("store"));
    let session = "session-seam";
    let turn = "turn-seam";

    let b1 = script_event(
        &store,
        "seam-b1",
        EventKind::SessionSummaryUpdated,
        session,
        turn,
        "{\"text\":\"b1\"}",
        RedactionState::Safe,
    );
    let b2 = script_event(
        &store,
        "seam-b2",
        EventKind::SessionSummaryUpdated,
        session,
        turn,
        "{\"text\":\"b2\"}",
        RedactionState::Safe,
    );

    let ScriptedTailServer {
        address, server, ..
    } = start_scripted_tail_server(Arc::clone(&store), 1);
    let (backlog, mut stream) =
        subscribe_tcp(&address, Some(session.to_string()), 0).expect("subscribe");
    let backlog_seqs: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert_eq!(backlog_seqs, vec![b1, b2]);

    let l1 = script_event(
        &store,
        "seam-l1",
        EventKind::ToolObservationRecorded,
        session,
        turn,
        "{\"text\":\"l1\"}",
        RedactionState::Safe,
    );
    let l2 = script_event(
        &store,
        "seam-l2",
        EventKind::EvidenceRecorded,
        session,
        turn,
        "{\"text\":\"l2\"}",
        RedactionState::Safe,
    );

    let (_f1, e1) = next_tail_frame(&mut stream);
    let (_f2, e2) = next_tail_frame(&mut stream);

    let mut delivered = backlog_seqs.clone();
    delivered.push(e1.sequence);
    delivered.push(e2.sequence);

    // No duplicate: every delivered sequence distinct.
    let mut unique = delivered.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        delivered.len(),
        "duplicate at the seam: {delivered:?}"
    );
    // No gap/reorder: exactly the committed sequence order.
    assert_eq!(
        delivered,
        vec![b1, b2, l1, l2],
        "gap or reorder at the seam"
    );
    // The first live sequence is strictly greater than the last backlog one.
    assert!(e1.sequence > b2);

    drop(stream);
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn in_band_cancel_on_the_subscribe_connection_ends_the_tail() {
    // ST11 cancel-over-tail: a `cancel` notification sent on the subscribe
    // connection (no request in flight, only the tail) stops the server tail and
    // emits the typed `cancelled` frame; the client sees the tail end.
    let root = temp_root();
    let store = Arc::new(SqliteStateStore::open(&root).expect("store"));
    let session = "session-cancel-tail";

    let ScriptedTailServer {
        address, server, ..
    } = start_scripted_tail_server(Arc::clone(&store), 1);

    // Subscribe over a raw connection so we control the exact frames we send and
    // read the raw `cancelled` error frame the tail-cancel emits. The subscribe
    // frame is built by hand so the test also pins the request wire shape.
    let mut conn = TcpStream::connect(&address).expect("connect");
    let subscribe_request_id = "stream-cancel-subscribe";
    let frame = serde_json::json!({
        "jsonrpc": "2.0",
        "id": subscribe_request_id,
        "method": "subscribe",
        "params": {
            "session_id": session,
            "from_sequence": 0,
            "origin": {
                "client_id": "local-cli",
                "actor_id": "local-user",
                "input_origin": "cli",
            }
        }
    })
    .to_string();
    write_line(&mut conn, &frame);
    // First frame back is the `subscribed` backlog response.
    let backlog_frame = read_line(&conn);
    let backlog_value: serde_json::Value =
        serde_json::from_str(backlog_frame.trim_end()).expect("subscribed frame is JSON-RPC");
    assert_eq!(
        backlog_value
            .get("result")
            .and_then(|r| r.get("payload"))
            .and_then(|p| p.get("type"))
            .and_then(serde_json::Value::as_str),
        Some("subscribed"),
        "first frame on a subscribe connection is the backlog: {backlog_frame}"
    );

    // Send an in-band cancel on the SAME connection.
    let cancel = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "cancel",
        "params": { "request_id": subscribe_request_id },
    })
    .to_string();
    write_line(&mut conn, &cancel);

    // The tail ends with a typed `cancelled` error frame.
    let cancelled = read_line(&conn);
    let cancelled_value: serde_json::Value =
        serde_json::from_str(cancelled.trim_end()).expect("cancel frame is JSON-RPC");
    assert!(
        cancelled_value.get("result").is_none(),
        "expected an error frame ending the tail, got: {cancelled}"
    );
    assert_eq!(
        cancelled_value
            .get("error")
            .and_then(|e| e.get("data"))
            .and_then(|d| d.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("cancelled"),
        "tail-cancel must carry the typed `cancelled` kind: {cancelled}"
    );

    conn.shutdown(Shutdown::Both).expect("shutdown");
    drop(conn);
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn mid_turn_interrupt_on_the_subscribe_connection_records_the_typed_abort() {
    // ST11 mid-turn interrupt over the stream: a typed `interrupt` notification
    // sent on the subscribe connection (the CLI Ctrl-C frame) drives the handler's
    // typed turn-aborted hook for the named session, so the thread projection can
    // render the interrupted turn -- distinct from a request-id `cancel`.
    let root = temp_root();
    let store = Arc::new(SqliteStateStore::open(&root).expect("store"));
    let session = "session-interrupt-tail";

    let ScriptedTailServer {
        address,
        interrupts,
        server,
    } = start_scripted_tail_server(Arc::clone(&store), 1);

    let mut conn = TcpStream::connect(&address).expect("connect");
    let frame = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "stream-interrupt-subscribe",
        "method": "subscribe",
        "params": {
            "session_id": session,
            "from_sequence": 0,
            "origin": {
                "client_id": "local-cli",
                "actor_id": "local-user",
                "input_origin": "cli",
            }
        }
    })
    .to_string();
    write_line(&mut conn, &frame);
    let backlog_frame = read_line(&conn);
    assert!(
        backlog_frame.contains("\"type\":\"subscribed\""),
        "first frame is the backlog: {backlog_frame}"
    );

    // Send the typed mid-turn interrupt on the SAME open connection.
    let interrupt = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "interrupt",
        "params": { "session_id": session, "reason": "operator ctrl-c" },
    })
    .to_string();
    write_line(&mut conn, &interrupt);

    // The typed turn-aborted hook fired with the session and reason (the
    // `session.interrupted` event the thread projection renders is recorded by the
    // production handler through `CapoServer::interrupt_session`).
    let recorded = poll_until(|| {
        let log = interrupts.lock().expect("interrupt log");
        log.first().cloned()
    });
    assert_eq!(
        recorded,
        Some((session.to_string(), "operator ctrl-c".to_string())),
        "the typed interrupt must drive RequestHandler::interrupt for the session"
    );

    conn.shutdown(Shutdown::Both).expect("shutdown");
    drop(conn);
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn live_tail_withholds_a_sensitive_event_body_and_never_emits_a_secret_raw() {
    // ST11 redaction-on-emit on the live tail: a scripted event classified
    // `ContainsSensitive` arrives on the live tail (the broadcast egress path) with
    // its body WITHHELD -- the secret cleartext never appears on the wire frame and
    // the egress classification is downgraded to `redacted`.
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let root = temp_root();
    let store = Arc::new(SqliteStateStore::open(&root).expect("store"));
    let session = "session-redact-tail";
    let turn = "turn-redact";

    let ScriptedTailServer {
        address, server, ..
    } = start_scripted_tail_server(Arc::clone(&store), 1);
    // Subscribe FIRST so the seeded secret travels the live broadcast tail.
    let (_backlog, mut stream) =
        subscribe_tcp(&address, Some(session.to_string()), 0).expect("subscribe");

    script_event(
        &store,
        "event-secret",
        EventKind::SessionSummaryUpdated,
        session,
        turn,
        &format!("{{\"key\":\"{secret}\"}}"),
        RedactionState::ContainsSensitive,
    );

    let (frame, event) = next_tail_frame(&mut stream);
    assert!(
        !frame.contains(secret),
        "secret leaked to the live tail wire: {frame}"
    );
    assert!(
        event
            .payload_json
            .contains(crate::WITHHELD_PAYLOAD_PLACEHOLDER),
        "sensitive body must be a withheld reference: {}",
        event.payload_json
    );
    assert_eq!(event.redaction_state, "redacted");

    drop(stream);
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn live_tail_streams_real_committed_events_from_write_bearing_commands() {
    // End-to-end with a REAL `CapoServer` over `serve_tcp`: a `subscribe_tcp` tail
    // observes the events a real write-bearing command commits (no scripted store).
    // This proves the production `CapoServerHandler::subscribe` path pumps the live
    // tail, not just the scripted handler.
    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address").to_string();
    let server_root = root.clone();
    // 1 subscribe connection + 1 register connection.
    let server = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(2),
        )
        .expect("serve tcp")
    });

    // Open the live tail first (global tail), then commit a real event.
    let (_backlog, mut stream) = subscribe_tcp(&address, None, 0).expect("subscribe");

    let registered = send_tcp(
        &address,
        &ServerRequest::local_cli(
            "stream-register",
            ServerCommand::RegisterAgent {
                name: "tail-agent".to_string(),
                adapter: "fake".to_string(),
            },
        ),
    )
    .expect("register over tcp");
    assert!(matches!(
        registered.payload,
        ServerResponsePayload::AgentRegistered(_)
    ));

    // The committed events fan out to the live tail. Read until we observe the
    // agent.registered event (there may be other committed events around it).
    stream
        .set_read_timeout(Some(TAIL_READ_TIMEOUT))
        .expect("set read timeout");
    let mut saw_registered = false;
    let deadline = Instant::now() + POLL_DEADLINE;
    while Instant::now() < deadline {
        match stream.next_event() {
            Ok(Some(event)) => {
                if event.kind == "agent.registered" {
                    saw_registered = true;
                    break;
                }
            }
            Ok(None) => break,
            Err(TransportError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => panic!("unexpected tail error: {error:?}"),
        }
    }
    assert!(
        saw_registered,
        "the live tail must deliver the agent.registered event committed by the real command"
    );

    drop(stream);
    assert_eq!(server.join().expect("server thread"), 2);
}

#[test]
fn thread_projection_and_subscriber_resume_identically_after_a_server_restart() {
    // ST11 restart/replay: commit a scripted multi-item turn, read the thread
    // projection and capture every committed event. Then "restart" (drop the
    // server, reopen the store on the same root) and prove (a) the thread
    // projection rebuilds byte-identically, and (b) a subscriber resuming from a
    // `from_sequence` watermark sees exactly the events strictly after it, in the
    // same order -- the durable log is the single source of truth across restart.
    let root = temp_root();
    let session = "session-restart";
    let turn = "turn-restart";

    // --- Before restart: commit a scripted turn through a CapoServer. ---
    let pre_restart_thread;
    let all_events_before;
    let resume_watermark;
    let events_after_watermark_before;
    {
        let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
        let store = server.state_for_test();
        script_event(
            store,
            "restart-1",
            EventKind::SessionSummaryUpdated,
            session,
            turn,
            "{\"text\":\"turn item one\"}",
            RedactionState::Safe,
        );
        let mid = script_event(
            store,
            "restart-2",
            EventKind::ToolObservationRecorded,
            session,
            turn,
            "{\"text\":\"turn item two\"}",
            RedactionState::Safe,
        );
        script_event(
            store,
            "restart-3",
            EventKind::EvidenceRecorded,
            session,
            turn,
            "{\"text\":\"turn item three\"}",
            RedactionState::Safe,
        );
        resume_watermark = mid;

        // Capture the thread projection before restart.
        let response = handle(
            &server,
            ServerCommand::ReadThread {
                session_id: session.to_string(),
                from_sequence: 0,
            },
        );
        let ServerResponsePayload::Thread(thread) = response.payload else {
            panic!("expected a thread payload");
        };
        pre_restart_thread = thread;

        // Capture every committed event and those strictly after the watermark.
        let (full_backlog, _stream) = server.subscribe(None, 0).expect("subscribe full");
        all_events_before = full_backlog.events;
        let (after, _after_stream) = server
            .subscribe(Some(session.to_string()), resume_watermark)
            .expect("subscribe after watermark");
        events_after_watermark_before = after.events;
        // Server dropped here: simulates a restart (the store reopens below).
    }

    // --- After restart: reopen on the SAME root. ---
    let restarted = CapoServer::open(ProjectId::new("project-capo"), &root).expect("reopen");

    // (a) The thread projection rebuilds identically from the durable log.
    let response = handle(
        &restarted,
        ServerCommand::ReadThread {
            session_id: session.to_string(),
            from_sequence: 0,
        },
    );
    let ServerResponsePayload::Thread(thread_after) = response.payload else {
        panic!("expected a thread payload after restart");
    };
    assert_eq!(
        thread_after, pre_restart_thread,
        "the thread projection must rebuild identically after a restart"
    );

    // (b) A subscriber resuming from the watermark sees exactly the same events,
    // in the same order, as before the restart.
    let (resumed, _resumed_stream) = restarted
        .subscribe(Some(session.to_string()), resume_watermark)
        .expect("resume subscribe after restart");
    assert_eq!(
        resumed.events, events_after_watermark_before,
        "a subscriber resuming from from_sequence must replay identically after restart"
    );
    // The resume is strictly after the watermark (no re-delivery of the watermark
    // event itself).
    assert!(
        resumed.events.iter().all(|e| e.sequence > resume_watermark),
        "resume must deliver only events strictly after the watermark"
    );

    // The full backlog after restart equals the full backlog before restart.
    let (full_after, _full_after_stream) = restarted.subscribe(None, 0).expect("full after");
    assert_eq!(
        full_after.events, all_events_before,
        "the full event log must replay identically after restart"
    );
}

/// Write a framed line to a raw connection (test helper).
fn write_line(stream: &mut TcpStream, frame: &str) {
    use std::io::Write;
    stream
        .write_all(frame.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .expect("write frame line");
}

/// Read one newline-terminated frame from a raw connection, with a read timeout
/// so a hung server fails the test rather than blocking forever.
fn read_line(stream: &TcpStream) -> String {
    use std::io::BufRead;
    stream
        .set_read_timeout(Some(TAIL_READ_TIMEOUT))
        .expect("set read timeout");
    let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read frame line");
    line
}
