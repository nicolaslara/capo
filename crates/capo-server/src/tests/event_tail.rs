//! ST4 event-tail tests: `events_after` backlog, the broadcast-fed live tail,
//! the backlog-to-live seam (no gap, no duplicate), and the `Subscribe`/tail
//! wire shapes. All deterministic: events are produced by scripted server
//! commands (register/send/steer), never a live provider.

use capo_state::SqliteStateStore;

use super::*;
use crate::{
    EVENT_TAIL_METHOD, EventNotification, ServerEvent, ServerThread, ServerThreadItem,
    ServerThreadTurn, SubscriptionBacklog, jsonrpc_request_roundtrip, jsonrpc_response_roundtrip,
};

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

#[test]
fn subscribe_backlog_returns_only_events_after_the_watermark_in_order() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    // Three writes -> at least three committed events.
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "alpha".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "beta".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "gamma".to_string(),
        },
    );

    let sequences = all_sequences(&root);
    assert!(
        sequences.len() >= 3,
        "expected several events: {sequences:?}"
    );
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "log sequences must be strictly increasing: {sequences:?}"
    );

    // A subscribe from 0 catches up on the whole log, in order.
    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe from 0");
    assert_eq!(
        backlog
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences,
    );
    assert_eq!(backlog.from_sequence, 0);
    assert_eq!(backlog.next_sequence, *sequences.last().expect("nonempty"));

    // A subscribe from a mid-log watermark returns strictly-after events only.
    let watermark = sequences[0];
    let (mid, _mid_stream) = server.subscribe(None, watermark).expect("subscribe mid");
    assert!(
        mid.events.iter().all(|event| event.sequence > watermark),
        "backlog must contain only events strictly after the watermark"
    );
    assert_eq!(
        mid.events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences[1..].to_vec(),
    );

    // A subscribe at the tail returns an empty backlog and resumes from there.
    let tail = *sequences.last().expect("nonempty");
    let (at_tail, _tail_stream) = server.subscribe(None, tail).expect("subscribe at tail");
    assert!(at_tail.events.is_empty());
    assert_eq!(at_tail.next_sequence, tail);
}

#[test]
fn event_tail_has_no_gap_and_no_duplicate_across_the_backlog_to_live_seam() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    // Seed a backlog: register an agent and start a task (several events).
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "operator".to_string(),
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

    // Subscribe: this snapshots the backlog AND registers the live subscriber.
    let (backlog, mut stream) = server.subscribe(None, 0).expect("subscribe");
    let backlog_sequences: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert!(
        !backlog_sequences.is_empty(),
        "expected a non-empty seed backlog"
    );
    assert_eq!(stream.delivered_through(), backlog.next_sequence);

    // Commit more events AFTER subscribing: these must arrive live, with no gap
    // back to the backlog and no duplicate of any backlog event at the seam.
    server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "operator".to_string(),
            goal: "first live redirect".to_string(),
        }))
        .expect("steer 1");
    server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "operator".to_string(),
            goal: "second live redirect".to_string(),
        }))
        .expect("steer 2");

    let live: Vec<i64> = stream.next_batch().iter().map(|e| e.sequence).collect();
    assert!(!live.is_empty(), "expected live events after subscribing");

    // The full delivered stream is backlog followed by live.
    let mut delivered = backlog_sequences.clone();
    delivered.extend(live.iter().copied());

    // No duplicate: every delivered sequence is distinct.
    let mut unique = delivered.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        delivered.len(),
        "duplicate at the seam: {delivered:?}"
    );

    // No gap: the delivered sequences are exactly the committed log, in order.
    assert_eq!(
        delivered,
        all_sequences(&root),
        "gap or reorder at the seam"
    );

    // The seam is exact: the first live sequence is strictly greater than the
    // last backlog sequence (the backlog watermark), so nothing is re-sent.
    let last_backlog = *backlog_sequences.last().expect("nonempty backlog");
    assert!(
        live.iter().all(|seq| *seq > last_backlog),
        "a live event re-sent a backlog sequence: backlog<= {last_backlog}, live={live:?}"
    );

    // Draining again with no new commits yields nothing (the watermark holds).
    assert!(
        stream.next_batch().is_empty(),
        "no new events should be delivered without a new commit"
    );
}

#[test]
fn session_scoped_subscribe_tails_only_the_named_session() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "scoped".to_string(),
        },
    );
    let sent = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "scoped".to_string(),
            goal: "scoped goal".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent");
    };
    let session_id = run.session_id.to_string();

    // Session-scoped backlog: only events carrying this session id.
    let (backlog, mut stream) = server
        .subscribe(Some(session_id.clone()), 0)
        .expect("session subscribe");
    assert!(
        backlog
            .events
            .iter()
            .all(|event| event.session_id.as_deref() == Some(session_id.as_str())),
        "session-scoped backlog must contain only the named session's events"
    );
    assert_eq!(backlog.session_id.as_deref(), Some(session_id.as_str()));

    // A live event for THIS session is delivered; a registration (no session)
    // and a different agent's session are not.
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "other".to_string(),
        },
    );
    server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: "scoped".to_string(),
            goal: "scoped live redirect".to_string(),
        }))
        .expect("steer scoped");

    let live = stream.next_batch();
    assert!(
        live.iter()
            .all(|event| event.session_id.as_deref() == Some(session_id.as_str())),
        "session-scoped live tail leaked another session's events: {live:?}"
    );
    assert!(
        live.iter().any(|event| event.kind == "session.redirected"),
        "expected the scoped session's redirect event in the live tail"
    );
}

#[test]
fn subscribe_command_and_subscribed_payload_round_trip_on_the_wire() {
    // The typed `Subscribe` command maps onto a JSON-RPC request and back.
    let request = ServerRequest::cli(ServerCommand::Subscribe {
        session_id: Some("session-xyz".to_string()),
        from_sequence: 42,
    });
    let decoded = jsonrpc_request_roundtrip(&request);
    assert_eq!(decoded, request);

    // A `None` session id (global tail) round-trips as JSON null.
    let global = ServerRequest::cli(ServerCommand::Subscribe {
        session_id: None,
        from_sequence: 0,
    });
    assert_eq!(jsonrpc_request_roundtrip(&global), global);

    // The `Subscribed` backlog payload round-trips through a full response.
    let backlog = SubscriptionBacklog {
        session_id: Some("session-xyz".to_string()),
        from_sequence: 42,
        next_sequence: 44,
        events: vec![
            ServerEvent {
                sequence: 43,
                event_id: "event-a".to_string(),
                kind: "session.redirected".to_string(),
                actor: "local-user".to_string(),
                project_id: Some("project-capo".to_string()),
                task_id: None,
                agent_id: Some("agent-scoped".to_string()),
                session_id: Some("session-xyz".to_string()),
                run_id: Some("run-xyz".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: None,
                payload_json: "{\"goal\":\"x\"}".to_string(),
                redaction_state: "safe".to_string(),
            },
            ServerEvent {
                sequence: 44,
                event_id: "event-b".to_string(),
                kind: "server.request_handled".to_string(),
                actor: "local-user".to_string(),
                project_id: Some("project-capo".to_string()),
                task_id: None,
                agent_id: None,
                session_id: Some("session-xyz".to_string()),
                run_id: None,
                turn_id: None,
                item_id: Some("req-1".to_string()),
                payload_json: "{}".to_string(),
                redaction_state: "safe".to_string(),
            },
        ],
    };
    let response = ServerResponse {
        request_id: "server-subscribe-session-xyz-42".to_string(),
        client_id: "local-cli".to_string(),
        actor_id: "local-user".to_string(),
        input_origin: ServerInputOrigin::Cli,
        payload: ServerResponsePayload::Subscribed(backlog),
    };
    assert_eq!(jsonrpc_response_roundtrip(&response), response);
}

#[test]
fn read_thread_command_and_thread_payload_round_trip_on_the_wire() {
    // ST5: the typed `ReadThread` command maps onto a JSON-RPC request and back.
    let request = ServerRequest::cli(ServerCommand::ReadThread {
        session_id: "session-xyz".to_string(),
        from_sequence: 7,
    });
    assert_eq!(jsonrpc_request_roundtrip(&request), request);

    // The `Thread` payload round-trips through a full response, including a turn
    // with one item that has BOTH `item_ref` and `text` set and another that has
    // neither (the optional `None` fields, which the wire encodes as JSON null
    // and the client decode path must read back as `None`).
    let thread = ServerThread {
        session_id: "session-xyz".to_string(),
        from_sequence: 7,
        next_sequence: 11,
        turns: vec![ServerThreadTurn {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            first_sequence: 8,
            last_sequence: 11,
            items: vec![
                ServerThreadItem {
                    sequence: 8,
                    event_id: "event-out".to_string(),
                    kind: "output".to_string(),
                    event_kind: "session.summary_updated".to_string(),
                    item_ref: Some("item-1".to_string()),
                    text: Some("inspected state".to_string()),
                    redaction_state: "safe".to_string(),
                },
                ServerThreadItem {
                    sequence: 11,
                    event_id: "event-term".to_string(),
                    kind: "terminal".to_string(),
                    event_kind: "evidence.recorded".to_string(),
                    item_ref: None,
                    text: None,
                    redaction_state: "safe".to_string(),
                },
            ],
        }],
    };
    let response = ServerResponse {
        request_id: "server-read-thread-session-xyz-7".to_string(),
        client_id: "local-cli".to_string(),
        actor_id: "local-user".to_string(),
        input_origin: ServerInputOrigin::Cli,
        payload: ServerResponsePayload::Thread(thread),
    };
    assert_eq!(jsonrpc_response_roundtrip(&response), response);
}

#[test]
fn read_thread_projects_real_turn_events_into_the_server_thread_payload() {
    // ST5 integration: drive real turn-keyed events through the server boundary
    // (a deterministic adapter-fixture replay), then `ReadThread` and assert the
    // projected `ServerThread` -- proving the read_model -> wire contract on a
    // real append, not a fabricated payload.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );
    let session_id = "session-codex-local-1";
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Read a real thread through the server".to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session_id.to_string()),
            run_id: Some("run-codex-local-1".to_string()),
        },
    );
    server
        .handle(ServerRequest::local_cli(
            "replay-codex-for-thread",
            ServerCommand::ReplayAdapterFixture {
                adapter: "codex".to_string(),
                session_id: session_id.to_string(),
                run_id: "run-codex-local-1".to_string(),
                turn_id: "turn-codex-local-1".to_string(),
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                    .to_string(),
            },
        ))
        .expect("replay codex fixture");

    let response = handle(
        &server,
        ServerCommand::ReadThread {
            session_id: session_id.to_string(),
            from_sequence: 0,
        },
    );
    let ServerResponsePayload::Thread(thread) = response.payload else {
        panic!("expected a thread response payload");
    };
    assert_eq!(thread.session_id, session_id);
    assert_eq!(thread.from_sequence, 0);
    let turn = thread
        .turns
        .iter()
        .find(|turn| turn.turn_id == "turn-codex-local-1")
        .expect("the replayed turn is projected");
    // The fixture replay ends in a completed turn (`evidence.recorded`).
    assert_eq!(turn.status, "completed");
    // The replay projected summary + tool items; the actual reply text is only
    // hashed, so every item carries the composed-label / ref text, never empty.
    assert!(!turn.items.is_empty(), "the turn projects its items");
    assert!(turn.items.iter().any(|item| item.kind == "tool"));
    assert!(turn.items.iter().any(|item| item.kind == "output"));
    // The wire watermark composes with a Subscribe tail.
    assert!(thread.next_sequence >= turn.last_sequence);
}

#[test]
fn live_event_notification_frame_round_trips() {
    let event = ServerEvent {
        sequence: 7,
        event_id: "event-live".to_string(),
        kind: "session.summary_updated".to_string(),
        actor: "controller".to_string(),
        project_id: Some("project-capo".to_string()),
        task_id: None,
        agent_id: None,
        session_id: Some("session-live".to_string()),
        run_id: Some("run-live".to_string()),
        turn_id: Some("turn-9".to_string()),
        item_id: None,
        payload_json: "{\"summary\":\"x\"}".to_string(),
        redaction_state: "safe".to_string(),
    };

    // A live tail pushes a JSON-RPC notification (no id) carrying the event.
    let notification = EventNotification::for_event(&event);
    assert_eq!(notification.method, EVENT_TAIL_METHOD);
    let frame = notification.to_wire_frame();
    let parsed: serde_json::Value = serde_json::from_str(&frame).expect("frame is json");
    assert_eq!(
        parsed.get("jsonrpc").and_then(serde_json::Value::as_str),
        Some("2.0")
    );
    assert!(parsed.get("id").is_none(), "a notification carries no id");

    // The frame decodes back to the same typed event.
    let decoded = EventNotification::from_wire_frame(&frame).expect("decode notification");
    assert_eq!(decoded.decode_event().expect("decode event"), event);
}
