//! DT4b: runner buffered-event reconciliation (spool + idempotent replay).
//!
//! DT4a proves the `Subscribe { from_sequence }` tail RESUMES across a drop for
//! events ALREADY COMMITTED to the server's log. This module builds the SEPARATE
//! mechanism DT-D2 names for the events a runner produced WHILE the runner<->server
//! leg was down: a runner-side SPOOL that buffers those `runtime.*` events and a
//! replay-on-reattach path that submits them to the server (the single writer) for
//! append.
//!
//! Why a spool is a REAL deliverable (and idempotency keys alone are not): the
//! in-tree `(project_id, idempotency_key)` dedupe at
//! [`capo_state::SqliteStateStore::append_event`] de-duplicates a re-SENT event, but
//! it does nothing for an event that was NEVER SENT because the leg was down. The
//! runner therefore has to RETAIN those events across the disconnect and re-offer
//! them on reattach. The spool is that retention; the server's idempotency-key
//! dedupe is what makes the replay EXACTLY ONCE (a reattach that re-sends an event
//! already appended on a prior, partially-successful replay is a no-op).
//!
//! Boundary discipline (DT-D2 / `knowledge.md`):
//! - The server stays the SINGLE authoritative writer. The spool never writes the
//!   log; it hands fully-formed [`NewEvent`]s (with a stable idempotency key) back to
//!   the caller, who submits them over the EXISTING transport for the server to
//!   append. The runner holds NO authoritative state.
//! - Each spooled frame is bounded and carries NO credential material: the DT3
//!   runner-side redaction runs before an event reaches the spool, and the spool
//!   defensively re-scans every payload on insert (a frame can never be the place a
//!   secret leaks across the reconnect).
//! - The spool is bounded (oldest-dropped, recorded) so a long outage cannot grow it
//!   without limit.

use capo_core::{ProjectId, SessionId};
use capo_state::{EventKind, NewEvent, RedactionState};

use crate::scan_credential_shapes;

/// DT4b: a single `runtime.*` event a runner produced while the server leg was down,
/// retained with the stable idempotency key the server dedupes on. The payload is
/// ALREADY redacted (DT3) and re-scanned on insert, so a spooled frame never carries
/// a credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpooledRuntimeEvent {
    /// A globally-unique id for THIS produced event (so two genuinely-distinct
    /// events are never collapsed even if their other fields coincide).
    pub event_id: String,
    /// The typed runtime event kind (a `runtime.*` kind). Typed rather than a raw
    /// string so the server append cannot be fed an arbitrary kind token.
    pub kind: EventKind,
    /// The session the runtime event pertains to (the runner leg tails its own
    /// session's process truth).
    pub session_id: SessionId,
    /// The stable idempotency key the server dedupes on. SAME logical event ->
    /// SAME key, so a re-sent event after a partial replay is a no-op append.
    pub idempotency_key: String,
    /// The redacted payload JSON (already scrubbed by DT3; re-scanned on insert).
    pub payload_json: String,
    /// The redaction classification after the spool's defensive re-scan.
    pub redaction_state: RedactionState,
}

impl SpooledRuntimeEvent {
    /// Build the append-ready [`NewEvent`] the caller submits to the server. The
    /// server's `append_event` dedupes on `(project_id, idempotency_key)`, so this is
    /// the unit of EXACTLY-ONCE replay.
    pub fn to_new_event(&self, project_id: &ProjectId, actor: &str) -> NewEvent {
        let mut event = NewEvent::new(self.event_id.clone(), self.kind, actor);
        event.project_id = Some(project_id.clone());
        event.session_id = Some(self.session_id.clone());
        event.idempotency_key = Some(self.idempotency_key.clone());
        event.payload_json = self.payload_json.clone();
        event.redaction_state = self.redaction_state;
        event
    }
}

/// DT4b: the outcome of offering one runtime event to the spool while disconnected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolAdmission {
    /// The event was buffered for replay on reattach.
    Buffered,
    /// The spool was at capacity; the OLDEST buffered event was dropped to make room
    /// for this one. Recorded (never silent) so a bounded-loss outage is auditable.
    BufferedEvictingOldest,
}

/// DT4b: a runner-side spool of `runtime.*` events produced while the runner<->server
/// leg is DOWN. While connected the runner submits events straight to the server; the
/// spool exists only for the disconnected window. On reattach [`Self::drain_for_replay`]
/// yields the buffered events in production order for the caller to replay over the
/// existing transport — the server's idempotency-key dedupe makes the replay
/// exactly-once.
///
/// The spool NEVER writes the authoritative log itself (the server is the single
/// writer) and is bounded so a long outage cannot grow it without limit.
#[derive(Debug)]
pub struct RunnerEventSpool {
    buffer: std::collections::VecDeque<SpooledRuntimeEvent>,
    capacity: usize,
    connected: bool,
    /// Count of events evicted (oldest-dropped) across the spool's life, for audit.
    evicted: u64,
}

impl RunnerEventSpool {
    /// A spool bounded to `capacity` buffered events, starting CONNECTED (the steady
    /// state — nothing is buffered while the leg is up). `capacity` is clamped to at
    /// least 1 so the bound is always meaningful.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: std::collections::VecDeque::new(),
            capacity: capacity.max(1),
            connected: true,
            evicted: 0,
        }
    }

    /// Mark the runner<->server leg DOWN. Events produced after this are buffered for
    /// replay rather than submitted directly. Idempotent.
    pub fn mark_disconnected(&mut self) {
        self.connected = false;
    }

    /// Whether the leg is currently connected (the steady state).
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Number of events currently buffered for replay.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Total events evicted (oldest-dropped) over the spool's life.
    pub fn evicted_count(&self) -> u64 {
        self.evicted
    }

    /// Offer a `runtime.*` event the runner produced WHILE DISCONNECTED to the spool.
    ///
    /// The payload is defensively re-scanned for credential shapes here (the DT3
    /// runner-side redaction already ran upstream; this guarantees a spooled frame
    /// can never be the place a secret leaks across the reconnect, satisfying the
    /// DT4b "spooled frame contains no seeded secret" criterion). When the spool is at
    /// capacity the OLDEST buffered event is dropped (bounded; recorded via
    /// [`SpoolAdmission::BufferedEvictingOldest`] and [`Self::evicted_count`]).
    ///
    /// Returns `None` when the leg is CONNECTED — there is nothing to spool; the
    /// caller submits the event to the server directly. This keeps the spool inert in
    /// the steady state (and, transitively, in the all-local default where the leg is
    /// never disconnected).
    pub fn offer(
        &mut self,
        event_id: impl Into<String>,
        kind: EventKind,
        session_id: SessionId,
        idempotency_key: impl Into<String>,
        payload_json: &str,
    ) -> Option<SpoolAdmission> {
        if self.connected {
            return None;
        }
        let (scrubbed, scrubbed_any) = scan_credential_shapes(payload_json);
        let redaction_state = if scrubbed_any {
            RedactionState::Redacted
        } else {
            RedactionState::Safe
        };
        let entry = SpooledRuntimeEvent {
            event_id: event_id.into(),
            kind,
            session_id,
            idempotency_key: idempotency_key.into(),
            payload_json: scrubbed,
            redaction_state,
        };
        let admission = if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
            self.evicted = self.evicted.saturating_add(1);
            SpoolAdmission::BufferedEvictingOldest
        } else {
            SpoolAdmission::Buffered
        };
        self.buffer.push_back(entry);
        Some(admission)
    }

    /// Mark the leg RECONNECTED and DRAIN the buffered events in production order for
    /// the caller to replay to the server (the single writer). After this the spool is
    /// empty and connected again; the steady state resumes.
    ///
    /// The caller submits each returned event over the existing transport; the
    /// server's `(project_id, idempotency_key)` dedupe makes the replay EXACTLY ONCE,
    /// so a reattach that re-sends an event already appended (a retried partial
    /// replay) produces no duplicate run and no duplicate event.
    pub fn drain_for_replay(&mut self) -> Vec<SpooledRuntimeEvent> {
        self.connected = true;
        self.buffer.drain(..).collect()
    }

    /// Non-draining view of the buffered events (for a status query / test
    /// inspection) without ending the disconnected window.
    pub fn peek(&self) -> impl Iterator<Item = &SpooledRuntimeEvent> {
        self.buffer.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::new("session-dt4b")
    }

    #[test]
    fn connected_spool_does_not_buffer_and_is_inert() {
        // Steady state: while connected the spool buffers nothing — the runner
        // submits straight to the server. This is what keeps it inert in the
        // all-local default (the leg is never disconnected).
        let mut spool = RunnerEventSpool::new(8);
        assert!(spool.is_connected());
        let admission = spool.offer(
            "evt-1",
            EventKind::RuntimeRemoteOutputDelta,
            session(),
            "runtime.remote_output_delta:run-1:0",
            "{\"offset\":0}",
        );
        assert_eq!(admission, None, "a connected spool must buffer nothing");
        assert_eq!(spool.buffered_len(), 0);
    }

    #[test]
    fn disconnected_events_are_buffered_in_production_order() {
        let mut spool = RunnerEventSpool::new(8);
        spool.mark_disconnected();
        for i in 0..3 {
            let admission = spool.offer(
                format!("evt-{i}"),
                EventKind::RuntimeRemoteOutputDelta,
                session(),
                format!("runtime.remote_output_delta:run-1:{i}"),
                &format!("{{\"offset\":{i}}}"),
            );
            assert_eq!(admission, Some(SpoolAdmission::Buffered));
        }
        let ids: Vec<&str> = spool.peek().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["evt-0", "evt-1", "evt-2"], "production order");

        let drained = spool.drain_for_replay();
        assert_eq!(drained.len(), 3);
        assert!(spool.is_connected(), "drain reconnects the leg");
        assert_eq!(spool.buffered_len(), 0, "drain empties the spool");
    }

    #[test]
    fn spool_is_bounded_and_evicts_oldest_recorded() {
        let mut spool = RunnerEventSpool::new(2);
        spool.mark_disconnected();
        assert_eq!(
            spool.offer(
                "a",
                EventKind::RuntimeRemoteOutputDelta,
                session(),
                "k:a",
                "{}"
            ),
            Some(SpoolAdmission::Buffered)
        );
        assert_eq!(
            spool.offer(
                "b",
                EventKind::RuntimeRemoteOutputDelta,
                session(),
                "k:b",
                "{}"
            ),
            Some(SpoolAdmission::Buffered)
        );
        // Third offer past capacity drops the OLDEST (a), recorded — not silent.
        assert_eq!(
            spool.offer(
                "c",
                EventKind::RuntimeRemoteOutputDelta,
                session(),
                "k:c",
                "{}"
            ),
            Some(SpoolAdmission::BufferedEvictingOldest)
        );
        assert_eq!(spool.buffered_len(), 2);
        assert_eq!(spool.evicted_count(), 1);
        let ids: Vec<&str> = spool.peek().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"], "oldest dropped, newest retained");
    }

    #[test]
    fn spooled_frame_scrubs_a_seeded_secret() {
        // DT4b: a spooled frame contains no seeded secret marker. Even if an
        // upstream pass somehow let a credential through, the spool's defensive
        // re-scan scrubs it before the frame is retained for replay.
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let mut spool = RunnerEventSpool::new(4);
        spool.mark_disconnected();
        let payload = format!("{{\"line\":\"key={secret}\"}}");
        spool.offer(
            "leak",
            EventKind::RuntimeRemoteOutputDelta,
            session(),
            "k:leak",
            &payload,
        );
        let entry = spool.peek().next().expect("buffered");
        assert!(
            !entry.payload_json.contains(secret),
            "spooled frame must not carry the seeded secret: {}",
            entry.payload_json
        );
        assert_eq!(entry.redaction_state, RedactionState::Redacted);
    }

    #[test]
    fn to_new_event_carries_the_idempotency_key_for_server_dedupe() {
        let mut spool = RunnerEventSpool::new(4);
        spool.mark_disconnected();
        spool.offer(
            "evt-x",
            EventKind::RuntimeRemoteStreamFinalized,
            session(),
            "runtime.remote_stream_finalized:run-1",
            "{\"reason\":\"eof\"}",
        );
        let entry = spool.peek().next().expect("buffered").clone();
        let new_event = entry.to_new_event(&ProjectId::new("project-capo"), "runner");
        assert_eq!(
            new_event.idempotency_key.as_deref(),
            Some("runtime.remote_stream_finalized:run-1"),
            "the idempotency key must survive onto the append-ready event"
        );
        assert_eq!(new_event.kind, EventKind::RuntimeRemoteStreamFinalized);
        assert_eq!(new_event.session_id, Some(session()));
    }
}
