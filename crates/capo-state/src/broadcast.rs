//! In-process event broadcast for the streaming-transport event tail (ST4).
//!
//! The append-only event log stays authoritative: a write commits a row, and
//! only *after* the transaction commits is the committed [`EventRecord`] fanned
//! out to every live subscriber here. Subscribers are how a long-lived tail
//! receives newly-committed events without polling the store, complementing the
//! catch-up backlog read ([`SqliteStateStore::events_after`]).
//!
//! ## Why a hand-rolled `mpsc` fan-out and not `tokio::sync::broadcast`
//!
//! capo-state is deliberately reactor-free: its writes are synchronous SQLite
//! transactions and its deterministic tests must not require a tokio runtime
//! (see `workpads/streaming-transport/knowledge.md`). A `tokio::sync::broadcast`
//! channel would drag a reactor dependency into the store layer for no gain. The
//! fan-out here is a small list of per-subscriber `std::sync::mpsc` senders: a
//! publish clones the committed record to each live sender and prunes any whose
//! receiver was dropped. Each subscriber owns an unbounded `Receiver`, so a
//! committed event is never lost to a full buffer; back-pressure is the
//! subscriber's to apply by draining.
//!
//! ## Ordering guarantee the seam relies on
//!
//! Publish runs strictly after `transaction.commit()`, so a subscriber that
//! subscribed *before* taking its `events_after(from_sequence)` backlog snapshot
//! cannot miss an event: any event committed after the snapshot is delivered
//! live, and any event already in the backlog that also arrives live is filtered
//! at the seam by sequence. That subscribe-then-backlog discipline (implemented
//! by the transport's `Subscribe`) is what makes the tail gap-free and
//! duplicate-free.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::EventRecord;

/// Fan-out hub for committed events. Cloning shares the same underlying
/// subscriber list (it holds an `Arc` internally via the store), so every clone
/// of a [`crate::SqliteStateStore`] publishes to and subscribes from the same
/// hub.
#[derive(Debug, Default)]
pub struct EventBroadcaster {
    subscribers: Mutex<Vec<Sender<EventRecord>>>,
}

impl EventBroadcaster {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register a new subscriber and return its receiver end. The subscriber is
    /// dropped from the fan-out automatically the first time a publish finds its
    /// receiver gone, so a caller that drops its [`EventSubscription`] needs no
    /// explicit unsubscribe.
    pub fn subscribe(&self) -> EventSubscription {
        let (tx, rx) = mpsc::channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(tx);
        EventSubscription { receiver: rx }
    }

    /// Fan a committed event out to every live subscriber, pruning any whose
    /// receiver has been dropped. Called by the store *after* the write commits.
    pub(crate) fn publish(&self, event: &EventRecord) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Retain only senders whose receiver is still alive; `send` errors iff
        // the receiver was dropped, so this both delivers and garbage-collects.
        subscribers.retain(|sender| sender.send(event.clone()).is_ok());
    }

    /// The number of live subscribers, used by deterministic tests to assert the
    /// fan-out prunes dropped receivers.
    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// A live subscription to committed events. Holds the receiver end of the
/// fan-out; drop it to unsubscribe (the next publish prunes the stale sender).
#[derive(Debug)]
pub struct EventSubscription {
    receiver: Receiver<EventRecord>,
}

impl EventSubscription {
    /// Drain every committed event already buffered for this subscriber without
    /// blocking. Returns them in commit (sequence) order.
    ///
    /// This is the non-blocking primitive the transport's `Subscribe` uses to
    /// fold buffered live events in after the backlog read: it never waits, so a
    /// subscriber with nothing pending gets an empty `Vec`.
    pub fn drain_pending(&self) -> Vec<EventRecord> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }

    /// Borrow the underlying receiver for callers that want to block on the next
    /// event (e.g. a live tail loop). Kept narrow so the channel type stays an
    /// implementation detail.
    pub fn receiver(&self) -> &Receiver<EventRecord> {
        &self.receiver
    }
}
