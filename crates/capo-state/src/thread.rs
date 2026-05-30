//! The multi-turn conversation thread read model (ST5).
//!
//! A daily-driver chat surface needs an ordered multi-turn thread, not the
//! single `latest_summary` the dashboard polls today. This module reconstructs
//! that thread as a *projected read model*: a pure function over the
//! append-only event log ([`EventRecord`]s in ascending sequence order). The
//! event log stays authoritative; the thread is derived from it and is never
//! client-owned state -- a client renders the projection and never authors turn
//! ordering.
//!
//! ## Replay-stability
//!
//! [`SessionThread::project`] is a pure fold over the events it is handed, so
//! re-feeding the same (or a superset of the same) persisted events rebuilds the
//! identical thread. The store query [`crate::SqliteStateStore::session_thread`]
//! reads the events from SQLite and calls this same fold, so a thread read after
//! a restart reconstructs identically from the durable log -- matching the
//! rebuildable-read-models rule in `workpads/architecture/state-model.md`.
//!
//! ## Idempotency
//!
//! Turns are keyed by `turn_id` so distinct turns never collapse onto one
//! another, and items are de-duplicated by `event_id` within the fold, so a
//! duplicated/replayed event (the same `event_id` appearing twice in the input)
//! contributes exactly one item -- the same thread a single, un-replayed
//! sequence would produce.
//!
//! ## Composability with `Subscribe`
//!
//! The projection records the highest sequence it folded
//! ([`SessionThread::next_sequence`]). A caller reads a thread incrementally by
//! supplying a `since_sequence` watermark (see
//! [`crate::SqliteStateStore::session_thread`], built on the same
//! `events_after_for_session` forward read the ST4 `Subscribe` backlog uses), so
//! the thread query and the live event tail resume from the same watermark with
//! no gap and no duplicate at the seam.

use capo_core::SessionId;

use crate::EventRecord;

/// How a turn ended, derived from its terminal event. `InProgress` means the
/// turn produced items but the projected events carry no terminal marker yet
/// (the turn is still streaming or the batch was partial).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadTurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Stopped,
    Failed,
}

impl ThreadTurnStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

/// The role of a single item on the turn timeline. The kinds map directly onto
/// the projected event kinds the turn loop and adapter replay already emit, so
/// the thread adds no parallel event vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadItemKind {
    /// Incremental assistant output / summary text (`session.summary_updated`).
    Output,
    /// A tool observation (`tool.*`).
    Tool,
    /// A terminal annotation for the turn (`evidence.recorded`,
    /// `session.interrupted`, `session.stopped`, `run.exited`). Kept as an item
    /// so the render can show the turn closing, and consumed to set the turn's
    /// [`ThreadTurnStatus`].
    Terminal,
}

impl ThreadItemKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Tool => "tool",
            Self::Terminal => "terminal",
        }
    }
}

/// One item on a turn's timeline, projected from a single committed event.
///
/// `text` is the human-facing content the render shows. It is derived from the
/// event's `payload_json` summary/blocker text when present, falling back to the
/// item ref so an item is always locatable; it never re-persists content that is
/// only referenced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadItem {
    pub sequence: i64,
    pub event_id: String,
    pub kind: ThreadItemKind,
    /// The projected event kind string (e.g. `session.summary_updated`).
    pub event_kind: String,
    pub item_ref: Option<String>,
    pub text: Option<String>,
    pub redaction_state: String,
}

/// One turn of the conversation: an ordered list of items keyed by `turn_id`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadTurn {
    pub turn_id: String,
    pub status: ThreadTurnStatus,
    pub first_sequence: i64,
    pub last_sequence: i64,
    pub items: Vec<ThreadItem>,
}

/// The projected multi-turn thread for a session.
///
/// `turns` are in first-seen order (the order each `turn_id` first appears in
/// the event log, which is ascending sequence). `next_sequence` is the highest
/// event sequence folded into this projection (or the caller's `since_sequence`
/// when no events matched); a caller reads the thread incrementally by resuming
/// a later query from it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionThread {
    pub session_id: SessionId,
    pub since_sequence: i64,
    pub next_sequence: i64,
    pub turns: Vec<ThreadTurn>,
}

impl SessionThread {
    /// Project a thread from committed events, in ascending sequence order.
    ///
    /// Pure and deterministic: the same events fold to the same thread, so a
    /// replay re-feeding the persisted events rebuilds it identically. Events
    /// are de-duplicated by `event_id` (idempotent re-feed) and grouped by
    /// `turn_id`; an event with no `turn_id` is not part of any conversation
    /// turn and is skipped (it is session/run lifecycle bookkeeping, not turn
    /// content). `since_sequence` is the watermark the events were read after,
    /// carried through so the result is self-describing for an incremental read.
    pub fn project(session_id: SessionId, since_sequence: i64, events: &[EventRecord]) -> Self {
        let mut turns: Vec<ThreadTurn> = Vec::new();
        let mut seen_event_ids: Vec<String> = Vec::new();
        let mut next_sequence = since_sequence;

        for event in events {
            if event.sequence > next_sequence {
                next_sequence = event.sequence;
            }
            // Idempotent re-feed: a duplicate event_id contributes once.
            if seen_event_ids.iter().any(|id| id == &event.event_id) {
                continue;
            }
            let Some(item) = project_item(event) else {
                continue;
            };
            // A turn-keyed event with no turn id is session/run lifecycle
            // bookkeeping, not conversation content.
            let Some(turn_id) = event.turn_id.clone() else {
                continue;
            };
            seen_event_ids.push(event.event_id.clone());

            let terminal_status = terminal_status_for(&event.kind);
            let turn = match turns.iter_mut().find(|turn| turn.turn_id == turn_id) {
                Some(turn) => turn,
                None => {
                    turns.push(ThreadTurn {
                        turn_id: turn_id.clone(),
                        status: ThreadTurnStatus::InProgress,
                        first_sequence: event.sequence,
                        last_sequence: event.sequence,
                        items: Vec::new(),
                    });
                    turns.last_mut().expect("turn just pushed is present")
                }
            };
            turn.last_sequence = event.sequence;
            if let Some(status) = terminal_status {
                turn.status = status;
            }
            turn.items.push(item);
        }

        Self {
            session_id,
            since_sequence,
            next_sequence,
            turns,
        }
    }

    /// Total item count across every turn (a convenience for renders/tests).
    pub fn item_count(&self) -> usize {
        self.turns.iter().map(|turn| turn.items.len()).sum()
    }
}

/// Classify a projected event into a thread item, or `None` when the event kind
/// is not conversation content (lifecycle/bookkeeping kinds the thread ignores).
fn project_item(event: &EventRecord) -> Option<ThreadItem> {
    let kind = match event.kind.as_str() {
        "session.summary_updated" => ThreadItemKind::Output,
        "tool.call_requested"
        | "tool.invocation_started"
        | "tool.call_completed"
        | "tool.observation_recorded"
        | "tool.result_delivered" => ThreadItemKind::Tool,
        "evidence.recorded" | "session.interrupted" | "session.stopped" | "run.exited" => {
            ThreadItemKind::Terminal
        }
        _ => return None,
    };
    Some(ThreadItem {
        sequence: event.sequence,
        event_id: event.event_id.clone(),
        kind,
        event_kind: event.kind.clone(),
        item_ref: event.item_id.clone(),
        text: item_text(event),
        redaction_state: event.redaction_state.clone(),
    })
}

/// Map a terminal event kind onto the turn status it sets, or `None` for a
/// non-terminal item. A turn with no terminal event stays `InProgress`.
fn terminal_status_for(kind: &str) -> Option<ThreadTurnStatus> {
    match kind {
        "evidence.recorded" => Some(ThreadTurnStatus::Completed),
        "session.interrupted" => Some(ThreadTurnStatus::Interrupted),
        "session.stopped" => Some(ThreadTurnStatus::Stopped),
        "run.exited" => Some(ThreadTurnStatus::Failed),
        _ => None,
    }
}

/// Extract the human-facing text for an item from its `payload_json`.
///
/// Different append paths shape their payloads differently, so this reads
/// whichever text-bearing field is present without re-deriving content:
///
/// - A path that stores a rendered line uses `latest_summary` / `detail` /
///   `latest_blocker` (e.g. the controller's session projection summary text).
/// - The adapter-replay path stores structured refs (`tool_name`, `status`,
///   `normalized_kind`, `content_hash`) rather than a prose line; when no prose
///   field is present we compose a stable one-line label from those so a tool
///   item still renders meaningfully.
///
/// Returns `None` only when the payload carries no usable field, leaving the
/// render to fall back to the item ref / event kind.
fn item_text(event: &EventRecord) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(&event.payload_json).ok()?;
    let object = value.as_object()?;
    for key in ["latest_summary", "detail", "latest_blocker", "message"] {
        if let Some(text) = object.get(key).and_then(serde_json::Value::as_str)
            && !text.is_empty()
        {
            return Some(text.to_string());
        }
    }
    // Adapter-replay shape: compose a one-line label from the structured refs.
    let field = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.is_empty() && *text != "none")
            .map(ToString::to_string)
    };
    let tool = field("tool_name");
    let normalized = field("normalized_kind");
    let status = field("status");
    match (tool, normalized, status) {
        (Some(tool), _, Some(status)) => Some(format!("{tool} ({status})")),
        (Some(tool), _, None) => Some(tool),
        (None, Some(normalized), Some(status)) => Some(format!("{normalized} ({status})")),
        (None, Some(normalized), None) => Some(normalized),
        (None, None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a turn-keyed event row as the append-only log would persist it.
    fn event(
        sequence: i64,
        event_id: &str,
        kind: &str,
        turn_id: Option<&str>,
        payload_json: &str,
    ) -> EventRecord {
        EventRecord {
            sequence,
            event_id: event_id.to_string(),
            kind: kind.to_string(),
            actor: "test".to_string(),
            project_id: None,
            task_id: None,
            agent_id: None,
            session_id: Some(SessionId::new("session-thread")),
            run_id: None,
            turn_id: turn_id.map(ToString::to_string),
            item_id: None,
            payload_json: payload_json.to_string(),
            idempotency_key: None,
            redaction_state: "safe".to_string(),
        }
    }

    /// A scripted two-turn conversation: each turn opens with a summary item, a
    /// tool item, then a terminal item, plus a lifecycle event with no turn id
    /// that the thread must ignore.
    fn scripted_multi_turn() -> Vec<EventRecord> {
        vec![
            event(
                1,
                "e1",
                "session.summary_updated",
                Some("turn-a"),
                "{\"latest_summary\":\"first reply\"}",
            ),
            event(
                2,
                "e2",
                "tool.call_completed",
                Some("turn-a"),
                "{\"tool_name\":\"shell\",\"status\":\"completed\"}",
            ),
            event(
                3,
                "e3",
                "evidence.recorded",
                Some("turn-a"),
                "{\"detail\":\"turn done\"}",
            ),
            // A run-lifecycle event with no turn id: not conversation content.
            event(4, "e4", "run.started", None, "{}"),
            event(
                5,
                "e5",
                "session.summary_updated",
                Some("turn-b"),
                "{\"latest_summary\":\"second reply\"}",
            ),
            event(
                6,
                "e6",
                "session.interrupted",
                Some("turn-b"),
                "{\"detail\":\"stopped early\"}",
            ),
        ]
    }

    #[test]
    fn projects_multi_turn_thread_keyed_by_turn_id() {
        let events = scripted_multi_turn();
        let thread = SessionThread::project(SessionId::new("session-thread"), 0, &events);

        // Two distinct turns, in first-seen order, never collapsed onto one.
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].turn_id, "turn-a");
        assert_eq!(thread.turns[1].turn_id, "turn-b");

        // Per-turn items keyed under their turn (the run.started event is
        // skipped: no turn id, lifecycle bookkeeping).
        assert_eq!(thread.turns[0].items.len(), 3);
        assert_eq!(thread.turns[0].status, ThreadTurnStatus::Completed);
        assert_eq!(thread.turns[0].items[0].kind, ThreadItemKind::Output);
        assert_eq!(
            thread.turns[0].items[0].text.as_deref(),
            Some("first reply")
        );
        assert_eq!(thread.turns[0].items[1].kind, ThreadItemKind::Tool);
        assert_eq!(
            thread.turns[0].items[1].text.as_deref(),
            Some("shell (completed)")
        );

        assert_eq!(thread.turns[1].items.len(), 2);
        assert_eq!(thread.turns[1].status, ThreadTurnStatus::Interrupted);

        // Watermark is the highest folded sequence, composable with a tail.
        assert_eq!(thread.next_sequence, 6);
        assert_eq!(thread.item_count(), 5);
    }

    #[test]
    fn rebuilds_identically_from_the_same_event_log() {
        // Replay-stability: the same persisted events fold to the same thread.
        let events = scripted_multi_turn();
        let first = SessionThread::project(SessionId::new("session-thread"), 0, &events);
        let rebuilt = SessionThread::project(SessionId::new("session-thread"), 0, &events);
        assert_eq!(first, rebuilt);
    }

    #[test]
    fn duplicate_replayed_events_are_idempotent() {
        // A turn whose events are replayed (the same event_id appears twice)
        // contributes exactly one item per event: the duplicate-free thread.
        let base = scripted_multi_turn();
        let mut replayed = base.clone();
        replayed.extend(base.iter().cloned());
        replayed.sort_by_key(|event| (event.sequence, event.event_id.clone()));

        let from_clean = SessionThread::project(SessionId::new("session-thread"), 0, &base);
        let from_replayed = SessionThread::project(SessionId::new("session-thread"), 0, &replayed);
        assert_eq!(from_clean, from_replayed);
    }

    #[test]
    fn incremental_read_from_a_watermark_skips_earlier_turns() {
        // Reading after turn-a's last sequence yields only turn-b, with the
        // watermark carried through, so the read composes with a `Subscribe`
        // resuming from the same point.
        let events: Vec<EventRecord> = scripted_multi_turn()
            .into_iter()
            .filter(|event| event.sequence > 3)
            .collect();
        let thread = SessionThread::project(SessionId::new("session-thread"), 3, &events);
        assert_eq!(thread.since_sequence, 3);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].turn_id, "turn-b");
        assert_eq!(thread.next_sequence, 6);
    }
}
