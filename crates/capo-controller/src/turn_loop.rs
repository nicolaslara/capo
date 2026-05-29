//! The normalized turn-loop contract: observe -> project -> `TurnFinished`.
//!
//! RTL3 defines the single-turn substrate the real controller (RTL5) and goal
//! continuation (`goal-autonomy`) later drive. A turn opens, the adapter
//! produces normalized events, the controller projects them through the
//! existing [`FakeBoundaryController::apply_normalized_adapter_events_with_turn`]
//! path, and the loop emits a [`TurnFinished`] outcome carrying the stop reason,
//! the summary refs, and the observed tool refs.
//!
//! Three invariants keep this honest for phase 1:
//!
//! - It maps onto the EXISTING event kinds. The terminal normalized events
//!   (`adapter.turn_completed`/`adapter.turn_interrupted`/`adapter.turn_failed`)
//!   already project onto `evidence.recorded`/`session.interrupted`/`run.exited`,
//!   and item/tool events onto `session.summary_updated`/`tool.*`. The loop adds
//!   no parallel turn vocabulary and no new [`capo_state::EventKind`] (the
//!   ceiling/recovery kinds are RTL7/RTL10).
//! - It is pure and synchronous: one observe -> project -> emit cycle per turn,
//!   deterministic over a scripted [`NormalizedAdapterEvent`] batch. No streaming
//!   (that is `streaming-transport`).
//! - [`TurnFinished`] ANNOTATES the persisted events; it does not fork a second
//!   completion model. It is re-derivable from the batch, so a restart/replay
//!   that rebuilds projections reconstructs the identical outcome.

use super::*;

/// Why a turn finished.
///
/// The variants map directly onto the controller commands and the terminal
/// normalized adapter events, so `interrupt`/`stop` (the existing controller
/// commands) and a scripted `adapter.turn_*` event resolve to the same outcome
/// vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStopReason {
    /// The adapter reported `adapter.turn_completed` (normal completion) or the
    /// batch carried no terminal event but did make progress.
    Completed,
    /// The turn was interrupted: an `adapter.turn_interrupted` event, or the
    /// controller `interrupt` command terminating an in-flight turn.
    Interrupted,
    /// The turn was stopped by the controller `stop` command.
    Stopped,
    /// The adapter reported `adapter.turn_failed`.
    Failed,
}

impl TurnStopReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

/// The outcome emitted at the end of one observe -> project -> emit cycle.
///
/// Every field is derived deterministically from the projected normalized
/// batch, so the same batch (or the same rebuilt projections after a restart)
/// reconstructs an identical `TurnFinished`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFinished {
    /// The turn this outcome closes.
    pub turn_id: TurnId,
    /// Why the turn finished.
    pub stop_reason: TurnStopReason,
    /// Refs to the summary/item content the turn produced, in observation
    /// order. Each ref is the `external_item_ref` (or timeline key) of an
    /// `adapter.item_*`/`adapter.plan_replaced` event so the summary text never
    /// has to be re-persisted to be located.
    pub summary_refs: Vec<String>,
    /// Refs to the tools the turn observed, in observation order. Each ref is
    /// the `external_item_ref` (or timeline key) of an `adapter.tool_call_*`
    /// event.
    pub observed_tool_refs: Vec<String>,
    /// The replay report from projecting the batch (event/tool/summary/completed
    /// counts), reused unchanged from the existing projection path.
    pub replay: AdapterReplayReport,
}

impl TurnFinished {
    /// `true` when the turn reached a terminal adapter event in its batch
    /// (completed/failed/interrupted) rather than only making partial progress.
    pub fn observed_terminal_event(&self) -> bool {
        self.replay.completed_turn_count > 0
            || matches!(
                self.stop_reason,
                TurnStopReason::Failed | TurnStopReason::Interrupted
            )
    }
}

impl FakeBoundaryController {
    /// Run one turn of the loop: observe a normalized event batch, project it,
    /// and emit a [`TurnFinished`].
    ///
    /// This is the RTL3 contract. The batch is the adapter's normalized output
    /// for this turn (in phase 1, a scripted/mock batch; later, a real Codex
    /// round-trip). Projection reuses the existing
    /// [`Self::apply_normalized_adapter_events_with_turn`] path so the loop adds
    /// no second ingestion route, and the emitted outcome is derived purely from
    /// the batch so it is deterministic and replay-stable.
    pub fn run_turn(
        &self,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        adapter_events: &[NormalizedAdapterEvent],
    ) -> StateResult<TurnFinished> {
        // Observe + project: drive the existing normalized-ingestion path,
        // keying every projected event/artifact to this turn.
        let replay = self.apply_normalized_adapter_events_with_turn(
            refs,
            adapter_events,
            Some(turn_id.as_str()),
        )?;
        // Emit: derive the outcome from the same batch we just projected.
        Ok(finish_turn(turn_id, adapter_events, replay))
    }

    /// Map the controller `interrupt` command onto the loop: terminate the
    /// in-flight turn and emit a [`TurnFinished`] with
    /// [`TurnStopReason::Interrupted`].
    ///
    /// This drives the existing [`Self::interrupt`] command (which records the
    /// `session.interrupted` event and updates the read models) so the loop has
    /// one interrupt path, then annotates it with the turn outcome.
    pub fn interrupt_turn(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.interrupt(registration, refs, reason)?;
        Ok(TurnFinished {
            turn_id: turn_id.clone(),
            stop_reason: TurnStopReason::Interrupted,
            summary_refs: Vec::new(),
            observed_tool_refs: Vec::new(),
            replay: AdapterReplayReport::default(),
        })
    }

    /// Map the controller `stop` command onto the loop: stop the run and emit a
    /// [`TurnFinished`] with [`TurnStopReason::Stopped`].
    pub fn stop_turn(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.stop(registration, refs, reason)?;
        Ok(TurnFinished {
            turn_id: turn_id.clone(),
            stop_reason: TurnStopReason::Stopped,
            summary_refs: Vec::new(),
            observed_tool_refs: Vec::new(),
            replay: AdapterReplayReport::default(),
        })
    }
}

/// Derive the [`TurnFinished`] outcome from a projected batch.
///
/// Kept as a free function (over `&[NormalizedAdapterEvent]` + the replay
/// report) so it is pure and trivially re-runnable against a rebuilt batch in a
/// replay test.
fn finish_turn(
    turn_id: &TurnId,
    adapter_events: &[NormalizedAdapterEvent],
    replay: AdapterReplayReport,
) -> TurnFinished {
    let mut summary_refs = Vec::new();
    let mut observed_tool_refs = Vec::new();
    let mut stop_reason = None;
    for event in adapter_events {
        match event.kind.as_str() {
            "adapter.item_completed" | "adapter.item_delta" | "adapter.plan_replaced" => {
                summary_refs.push(turn_ref_for(event));
            }
            "adapter.tool_call_requested"
            | "adapter.tool_call_started"
            | "adapter.tool_call_completed"
            | "adapter.tool_call_failed" => {
                let tool_ref = turn_ref_for(event);
                if !observed_tool_refs.contains(&tool_ref) {
                    observed_tool_refs.push(tool_ref);
                }
            }
            "adapter.turn_completed" => stop_reason = Some(TurnStopReason::Completed),
            "adapter.turn_failed" => stop_reason = Some(TurnStopReason::Failed),
            "adapter.turn_interrupted" => stop_reason = Some(TurnStopReason::Interrupted),
            _ => {}
        }
    }
    TurnFinished {
        turn_id: turn_id.clone(),
        // A batch with no terminal adapter event is treated as a completed
        // single observe->project cycle for phase 1: the loop emitted, there is
        // nothing in flight, and the read models reflect the projected items.
        stop_reason: stop_reason.unwrap_or(TurnStopReason::Completed),
        summary_refs,
        observed_tool_refs,
        replay,
    }
}

/// The stable ref identifying an event's item on the turn timeline: prefer the
/// adapter's `external_item_ref`, fall back to the timeline key, then the raw
/// event hash, so a ref is always available and deterministic.
fn turn_ref_for(event: &NormalizedAdapterEvent) -> String {
    event
        .external_item_ref
        .clone()
        .or_else(|| event.timeline_key.clone())
        .unwrap_or_else(|| event.raw_event_hash.clone())
}
