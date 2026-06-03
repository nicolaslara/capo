//! DT4b: runner buffered-event reconciliation (spool + idempotent replay) at the
//! `capo-server` seam.
//!
//! DT4a proves the watermark resume for events ALREADY COMMITTED to the log. DT4b is
//! the SEPARATE mechanism (DT-D2): the `runtime.*` events a runner produced WHILE the
//! server leg was DOWN are buffered in a runner-side
//! [`capo_runtime::RunnerEventSpool`] and replayed on reattach to the single-writer
//! server. The spool itself is proven in `capo-runtime`'s unit suite; these tests
//! pin the SERVER half of the contract the DT4b section names, and they exercise the
//! PRODUCTION transport seam — `ServerCommand::ReplayRunnerEvents` through
//! `CapoServer::handle()` — NOT a `state_for_test()` backdoor:
//!
//! - replayed spooled events are appended by the SINGLE WRITER and a tailing client
//!   sees each EXACTLY ONCE (idempotency-key dedupe), with no duplicate run;
//! - a replay that re-sends an already-appended event is a NO-OP, and the resulting
//!   committed sequence is CONTIGUOUS;
//! - a spooled frame carries no seeded secret marker before it crosses the leg, and
//!   the property holds onto the committed, client-visible event;
//! - the server RE-VALIDATES each replay frame at the seam (a non-runtime kind or an
//!   unscrubbed classification is refused before any append), so a raw-TCP caller
//!   cannot inject through the replay path.
//!
//! All deterministic: the disconnect/reconnect is the spool's own state transition
//! (`mark_disconnected` / `drain_for_replay`), NOT a wall-clock drop, and every event
//! is produced synchronously. The replay flows through the same `handle()` request
//! path a reconnecting runner uses over JSON-RPC, so the test exercises the actual
//! production seam end-to-end (runner produces → spools → reconnects → submits over
//! the command transport → single writer appends → tailing client sees once).

use capo_core::{ProjectId, SessionId};
use capo_runtime::{RunnerEventSpool, SpoolAdmission, SpooledRuntimeEvent};
use capo_state::{EventKind, SqliteStateStore};

use super::temp_root;
use crate::{
    CapoServer, RunnerReplayFrame, ServerCommand, ServerRequest, ServerResponse,
    ServerResponsePayload,
};

/// Every committed sequence in the store for a session, in order, read from the log.
fn session_sequences(root: &std::path::Path, session: &SessionId) -> Vec<i64> {
    SqliteStateStore::open(root)
        .expect("state")
        .events_after_for_session(session, 0, 100_000)
        .expect("events_after_for_session")
        .into_iter()
        .map(|event| event.sequence)
        .collect()
}

/// Count how many times an `event_id` appears in the committed log for a session.
fn occurrences(root: &std::path::Path, session: &SessionId, event_id: &str) -> usize {
    SqliteStateStore::open(root)
        .expect("state")
        .events_after_for_session(session, 0, 100_000)
        .expect("events_after_for_session")
        .into_iter()
        .filter(|event| event.event_id == event_id)
        .count()
}

/// Build the production replay command from a runner's drained spool, exactly as a
/// reconnecting runner does (`SpooledRuntimeEvent` -> wire `RunnerReplayFrame`).
fn replay_command(drained: &[SpooledRuntimeEvent]) -> ServerCommand {
    ServerCommand::ReplayRunnerEvents {
        frames: drained.iter().map(RunnerReplayFrame::from).collect(),
    }
}

/// Submit a `ReplayRunnerEvents` command over the SAME `handle()` request path a
/// reconnecting runner uses over JSON-RPC, returning the per-frame committed
/// sequences (the single writer's assignment / dedupe outcome).
fn replay_through_server(server: &CapoServer, command: ServerCommand) -> Vec<i64> {
    let response: ServerResponse = server
        .handle(ServerRequest::cli(command))
        .expect("replay handled");
    let ServerResponsePayload::RunnerEventsReplayed(summary) = response.payload else {
        panic!("expected RunnerEventsReplayed response");
    };
    summary.appended_sequences
}

#[test]
fn spooled_runner_events_replay_through_the_single_writer_exactly_once() {
    // A runner produces three `runtime.*` events while its server leg is DOWN. They
    // are spooled, then replayed on reattach through the PRODUCTION transport seam
    // (`ServerCommand::ReplayRunnerEvents` -> `CapoServer::handle()` -> single
    // writer). A client tailing the log sees each EXACTLY ONCE, in order, no dup.
    let root = temp_root();
    let project = ProjectId::new("project-capo");
    let session = SessionId::new("session-dt4b-replay");
    let server = CapoServer::open(project.clone(), &root).expect("server");

    let mut spool = RunnerEventSpool::new(16);
    spool.mark_disconnected();
    for i in 0..3 {
        let admission = spool.offer(
            format!("runner-evt-{i}"),
            EventKind::RuntimeRemoteOutputDelta,
            session.clone(),
            // The stable idempotency key the server dedupes on.
            format!("runtime.remote_output_delta:run-dt4b:{i}"),
            &format!("{{\"offset\":{i},\"text\":\"delta {i}\"}}"),
        );
        assert_eq!(admission, Some(SpoolAdmission::Buffered));
    }

    // --- REATTACH: drain the spool and replay each event to the single writer over
    // the existing command transport (the production seam, not a test backdoor).
    let replayed = spool.drain_for_replay();
    assert_eq!(replayed.len(), 3);
    assert!(spool.is_connected(), "drain reconnects the leg");
    let sequences = replay_through_server(&server, replay_command(&replayed));
    assert_eq!(
        sequences.len(),
        3,
        "the single writer assigned each a sequence"
    );
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "replayed frames are committed in strictly increasing order: {sequences:?}"
    );

    // --- A tailing client sees each spooled event EXACTLY ONCE, in order.
    let (backlog, _stream) = server
        .subscribe(Some(session.as_str().to_string()), 0)
        .expect("subscribe");
    let seen: Vec<&str> = backlog
        .events
        .iter()
        .filter(|e| e.event_id.starts_with("runner-evt-"))
        .map(|e| e.event_id.as_str())
        .collect();
    assert_eq!(
        seen,
        vec!["runner-evt-0", "runner-evt-1", "runner-evt-2"],
        "the client must see each replayed event exactly once, in production order"
    );
    for i in 0..3 {
        assert_eq!(
            occurrences(&root, &session, &format!("runner-evt-{i}")),
            1,
            "no duplicate of runner-evt-{i} in the committed log"
        );
    }
}

#[test]
fn re_replaying_an_already_appended_event_is_a_no_op_and_the_sequence_stays_contiguous() {
    // A reattach can re-send events a PRIOR (partially-successful) replay already
    // appended. The single writer's `(project_id, idempotency_key)` dedupe makes
    // that a no-op: no duplicate event, no duplicate run, and the committed
    // sequence stays contiguous. Both replays go through the PRODUCTION seam.
    let root = temp_root();
    let project = ProjectId::new("project-capo");
    let session = SessionId::new("session-dt4b-idem");
    let server = CapoServer::open(project.clone(), &root).expect("server");

    let mut spool = RunnerEventSpool::new(16);
    spool.mark_disconnected();
    for i in 0..2 {
        spool.offer(
            format!("idem-evt-{i}"),
            EventKind::RuntimeRemoteOutputDelta,
            session.clone(),
            format!("runtime.remote_output_delta:run-idem:{i}"),
            &format!("{{\"offset\":{i}}}"),
        );
    }
    let replayed = spool.drain_for_replay();

    // First replay through the server: appends both. Record each returned sequence.
    let first_seqs = replay_through_server(&server, replay_command(&replayed));
    assert_eq!(first_seqs.len(), 2);

    // SECOND replay of the SAME drained events (a retried reattach), again over the
    // production seam. The dedupe returns the EXISTING sequence for each — no rows.
    let second_seqs = replay_through_server(&server, replay_command(&replayed));
    assert_eq!(
        second_seqs, first_seqs,
        "re-replaying already-appended frames must return their existing sequences (no-op)"
    );

    // Each event appears exactly once; the session's committed sequence is
    // strictly increasing and contiguous-as-read (no gap introduced by the dedupe).
    for i in 0..2 {
        assert_eq!(
            occurrences(&root, &session, &format!("idem-evt-{i}")),
            1,
            "idem-evt-{i} must appear exactly once despite the double replay"
        );
    }
    let seqs = session_sequences(&root, &session);
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "the committed session sequence must stay strictly increasing: {seqs:?}"
    );
}

#[test]
fn a_replayed_spooled_frame_carries_no_seeded_secret() {
    // DT4b: a spooled frame contains no seeded secret marker — and the property
    // holds end-to-end onto the appended, client-visible event committed through
    // the production replay seam. Even a payload that arrives with a credential is
    // scrubbed by the spool before replay, and the server's egress backstop guards
    // the tail.
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let root = temp_root();
    let project = ProjectId::new("project-capo");
    let session = SessionId::new("session-dt4b-secret");
    let server = CapoServer::open(project.clone(), &root).expect("server");

    let mut spool = RunnerEventSpool::new(8);
    spool.mark_disconnected();
    spool.offer(
        "secret-evt",
        EventKind::RuntimeRemoteOutputDelta,
        session.clone(),
        "runtime.remote_output_delta:run-secret:0",
        &format!("{{\"text\":\"token={secret}\"}}"),
    );
    let replayed = spool.drain_for_replay();
    assert!(
        !replayed[0].payload_json.contains(secret),
        "the spooled frame must not carry the seeded secret: {}",
        replayed[0].payload_json
    );

    replay_through_server(&server, replay_command(&replayed));

    let (backlog, _stream) = server
        .subscribe(Some(session.as_str().to_string()), 0)
        .expect("subscribe");
    let appended = backlog
        .events
        .iter()
        .find(|e| e.event_id == "secret-evt")
        .expect("replayed event in backlog");
    assert!(
        !appended.payload_json.contains(secret),
        "the committed, client-visible event must not carry the secret: {}",
        appended.payload_json
    );
}

#[test]
fn the_replay_seam_refuses_a_non_runtime_kind_before_appending() {
    // The replay command can arrive from a remote runner speaking JSON-RPC
    // directly, so the server RE-VALIDATES each frame at the seam: a wire `kind`
    // that is not a `runtime.remote_*` kind is refused BEFORE any append, and
    // nothing reaches the log. This is the replay analogue of the
    // `RegisterRuntimeTarget` re-validation — the single writer never appends a
    // kind the closed runtime vocabulary does not own through the replay path.
    let root = temp_root();
    let project = ProjectId::new("project-capo");
    let session = SessionId::new("session-dt4b-reject");
    let server = CapoServer::open(project.clone(), &root).expect("server");

    // A frame whose kind is a real EventKind but NOT a runtime.remote_* kind.
    let forbidden = ServerCommand::ReplayRunnerEvents {
        frames: vec![RunnerReplayFrame {
            event_id: "forbidden-evt".to_string(),
            kind: EventKind::AgentRegistered.as_str().to_string(),
            session_id: session.as_str().to_string(),
            idempotency_key: "runtime.remote_output_delta:run-x:0".to_string(),
            payload_json: "{}".to_string(),
            redaction_state: "safe".to_string(),
        }],
    };
    let result = server.handle(ServerRequest::cli(forbidden));
    assert!(
        matches!(
            result,
            Err(crate::ServerError::InvalidRunnerReplayFrame { field: "kind", .. })
        ),
        "a non-runtime replay kind must be refused at the seam: {result:?}"
    );

    // And an unknown wire kind is likewise refused.
    let unknown = ServerCommand::ReplayRunnerEvents {
        frames: vec![RunnerReplayFrame {
            event_id: "unknown-evt".to_string(),
            kind: "runtime.totally_made_up".to_string(),
            session_id: session.as_str().to_string(),
            idempotency_key: "runtime.remote_output_delta:run-x:1".to_string(),
            payload_json: "{}".to_string(),
            redaction_state: "safe".to_string(),
        }],
    };
    assert!(
        matches!(
            server.handle(ServerRequest::cli(unknown)),
            Err(crate::ServerError::InvalidRunnerReplayFrame { field: "kind", .. })
        ),
        "an unknown replay kind must be refused at the seam"
    );

    // Nothing was appended: the session's log is empty.
    assert_eq!(
        occurrences(&root, &session, "forbidden-evt"),
        0,
        "a refused replay frame must never reach the authoritative log"
    );
    assert_eq!(occurrences(&root, &session, "unknown-evt"), 0);
}

#[test]
fn the_replay_seam_refuses_an_unscrubbed_redaction_classification() {
    // A replayed frame must carry a persistable (`safe`/`redacted`) classification.
    // A frame that arrives classified `contains_sensitive` (an unscrubbed frame) is
    // refused at the seam rather than committed, so the replay path can never
    // append an unscrubbed event into the authoritative log.
    let root = temp_root();
    let project = ProjectId::new("project-capo");
    let session = SessionId::new("session-dt4b-unscrubbed");
    let server = CapoServer::open(project.clone(), &root).expect("server");

    let unscrubbed = ServerCommand::ReplayRunnerEvents {
        frames: vec![RunnerReplayFrame {
            event_id: "unscrubbed-evt".to_string(),
            kind: EventKind::RuntimeRemoteOutputDelta.as_str().to_string(),
            session_id: session.as_str().to_string(),
            idempotency_key: "runtime.remote_output_delta:run-y:0".to_string(),
            payload_json: "{}".to_string(),
            redaction_state: "contains_sensitive".to_string(),
        }],
    };
    let result = server.handle(ServerRequest::cli(unscrubbed));
    assert!(
        matches!(
            result,
            Err(crate::ServerError::InvalidRunnerReplayFrame {
                field: "redaction_state",
                ..
            })
        ),
        "an unscrubbed replay frame must be refused at the seam: {result:?}"
    );
    assert_eq!(
        occurrences(&root, &session, "unscrubbed-evt"),
        0,
        "a refused unscrubbed frame must never reach the authoritative log"
    );
}
