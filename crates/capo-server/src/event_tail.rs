//! The server-side event tail (ST4): catch-up backlog + live broadcast.
//!
//! A `Subscribe { session_id, from_sequence }` is served in two phases that
//! together give a gap-free, duplicate-free forward read of the append-only
//! event log:
//!
//! 1. **Backlog** -- every committed event strictly after `from_sequence`
//!    (optionally filtered to one session), read once via
//!    `capo_state::SqliteStateStore::events_after`.
//! 2. **Live tail** -- newly-committed events fanned out by the store's
//!    broadcast hub *after* their write commits.
//!
//! The ordering that makes the seam sound is: the [`EventStream`] subscribes to
//! the broadcast **before** the backlog snapshot is read (see
//! `CapoServer::subscribe`). So an event committed between the snapshot and the
//! first live poll is captured by the subscription rather than lost (no gap),
//! and any event that appears in *both* the backlog and the live buffer is
//! dropped on the live side by the per-stream watermark (no duplicate): the
//! stream only ever yields an event whose `sequence` is strictly greater than
//! the highest sequence it has already delivered.

use capo_state::EventSubscription;

use crate::ServerEvent;

/// A live tail over committed events, paired with the catch-up backlog by
/// `CapoServer::subscribe`. It holds the broadcast subscription and the
/// per-stream delivery watermark (the highest sequence delivered so far, seeded
/// from the backlog's `next_sequence`), plus an optional session filter.
///
/// Dropping the stream unsubscribes it from the broadcast (the store prunes the
/// stale sender on its next publish).
#[derive(Debug)]
pub struct EventStream {
    subscription: EventSubscription,
    /// Highest sequence delivered so far, across backlog + live. The live side
    /// only yields events with `sequence > delivered_through`, which is exactly
    /// the seam dedupe: a live event already covered by the backlog is dropped.
    delivered_through: i64,
    /// When `Some`, the tail yields only events for this session id, matching the
    /// session filter the backlog was read with.
    session_filter: Option<String>,
}

impl EventStream {
    pub(crate) fn new(
        subscription: EventSubscription,
        delivered_through: i64,
        session_filter: Option<String>,
    ) -> Self {
        Self {
            subscription,
            delivered_through,
            session_filter,
        }
    }

    /// The highest sequence this stream has delivered so far (backlog + live).
    /// A reconnecting subscriber resumes a fresh `Subscribe` from this value.
    pub fn delivered_through(&self) -> i64 {
        self.delivered_through
    }

    /// Drain every committed event buffered for this subscriber, applying the
    /// seam dedupe and the session filter, and advance the watermark. Never
    /// blocks: a tail with nothing pending returns an empty `Vec`.
    ///
    /// Events are returned in commit (sequence) order. An event at or below the
    /// current watermark is a duplicate already delivered via the backlog (or an
    /// earlier poll) and is dropped; an event for a different session (when a
    /// filter is set) is skipped without advancing the watermark past it.
    pub fn next_batch(&mut self) -> Vec<ServerEvent> {
        let mut batch = Vec::new();
        for record in self.subscription.drain_pending() {
            // Seam dedupe: never re-deliver an event the backlog already carried
            // (or an earlier live batch delivered).
            if record.sequence <= self.delivered_through {
                continue;
            }
            // Session filter: a live event for another session is not part of
            // this tail. The global delivery watermark still advances (we have
            // observed this sequence) so a later matching event is not blocked.
            let session_matches = match &self.session_filter {
                Some(filter) => record
                    .session_id
                    .as_ref()
                    .map(|id| id.as_str() == filter)
                    .unwrap_or(false),
                None => true,
            };
            self.delivered_through = record.sequence;
            if session_matches {
                batch.push(ServerEvent::from_record(record));
            }
        }
        batch
    }
}
