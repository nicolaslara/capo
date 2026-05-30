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
//!   completion model. Its equality-significant fields (`turn_id`,
//!   `stop_reason`, `summary_refs`, `observed_tool_refs`,
//!   `observed_terminal_event`) are derived purely from the batch -- the same
//!   source the projection consumes -- so a restart/replay that re-feeds the
//!   persisted batch reconstructs the identical outcome. The embedded
//!   [`AdapterReplayReport`] is the ONE exception: it counts events *appended*
//!   by this projection pass, so it is a volatile per-run diagnostic (it
//!   collapses to zero on an idempotent re-run) and must not be read for any
//!   replay-stable signal -- use the batch-derived fields instead.
//!
//! Scope honesty: the equality-significant outcome is re-derivable from the
//! turn's normalized batch (which the projection persists verbatim as
//! `payload_json` on each `event.adapter.replay.*` row). `run_turn` derives it
//! from the in-memory batch; a replay re-derives it by re-feeding that batch.
//! The `interrupt_turn`/`stop_turn` command paths are a deliberate exception:
//! they carry empty `summary_refs`/`observed_tool_refs` and a default `replay`
//! because the controller `interrupt`/`stop` commands produce a single
//! terminal `session.interrupted`/`session.stopped` event, not an adapter
//! batch; their replay-stable content is the `turn_id` + `stop_reason`, which
//! they persist onto the terminal event (see `interrupt_turn`/`stop_turn`).

use capo_adapters::AdapterTerminalOutcome;

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

    /// Map a terminal adapter event's outcome onto the loop's stop reason. This
    /// is the single place the adapter taxonomy crosses into the loop's
    /// vocabulary, so the projection path and the loop agree on what a terminal
    /// `adapter.turn_*` event means.
    const fn from_terminal_outcome(outcome: AdapterTerminalOutcome) -> Self {
        match outcome {
            AdapterTerminalOutcome::Completed => Self::Completed,
            AdapterTerminalOutcome::Failed => Self::Failed,
            AdapterTerminalOutcome::Interrupted => Self::Interrupted,
        }
    }
}

/// The outcome emitted at the end of one observe -> project -> emit cycle.
///
/// The equality-significant fields (`turn_id`, `stop_reason`, `summary_refs`,
/// `observed_tool_refs`, `observed_terminal_event`) are derived deterministically
/// from the turn's normalized batch, so the same batch reconstructs an identical
/// outcome on replay. `replay` is a volatile per-run diagnostic and is
/// deliberately excluded from that guarantee (see [`Self::replay`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFinished {
    /// The turn this outcome closes.
    pub turn_id: TurnId,
    /// Why the turn finished.
    pub stop_reason: TurnStopReason,
    /// `true` when the batch carried a terminal `adapter.turn_*` event
    /// (completed/failed/interrupted) rather than only making partial progress.
    /// Derived from the batch (not the append-counting replay report) so it is
    /// replay-stable.
    pub observed_terminal_event: bool,
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
    ///
    /// VOLATILE: these counts measure events *appended* by THIS projection pass,
    /// so an idempotent re-run (nothing newly appended) collapses them all to
    /// zero. This field is therefore a per-run diagnostic only and is NOT
    /// replay-stable; never read it for a terminal/summary/tool signal -- use the
    /// batch-derived fields above.
    pub replay: AdapterReplayReport,
}

impl TurnFinished {
    /// `true` when the turn reached a terminal adapter event in its batch
    /// (completed/failed/interrupted) rather than only making partial progress.
    pub fn observed_terminal_event(&self) -> bool {
        self.observed_terminal_event
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
        Ok(Self::derive_turn_finished(turn_id, adapter_events, replay))
    }

    /// Derive the [`TurnFinished`] outcome from an ALREADY-projected normalized
    /// batch plus the projection's replay report.
    ///
    /// RTL4 reuses this from the server's dispatch path: the existing dispatch
    /// pipeline (`PlanDispatch`/`PreflightLiveProvider` -> `GateDispatch` ->
    /// `RunDispatchLocal`/`RunLiveProviderLocal`) is the loop's single execution
    /// substrate, so once the dispatch run has ingested the batch through
    /// [`Self::apply_normalized_adapter_events_with_turn`], the loop ANNOTATES
    /// that run with a `TurnFinished` derived from the *same* batch -- it does
    /// not fork a second completion model. Keeping the derivation here means
    /// [`Self::run_turn`] and the dispatch path agree on the outcome by
    /// construction, because they call the same pure classifier.
    pub fn derive_turn_finished(
        turn_id: &TurnId,
        adapter_events: &[NormalizedAdapterEvent],
        replay: AdapterReplayReport,
    ) -> TurnFinished {
        finish_turn(turn_id, adapter_events, replay)
    }

    /// Reconstruct the equality-significant `TurnFinished` for a turn purely
    /// from the PERSISTED, turn-keyed event log (no in-memory batch).
    ///
    /// This is the event-sourced proof that the outcome is re-derivable from
    /// what was persisted: it reads the projected `event.adapter.replay.*` rows
    /// scoped to `turn_id` and re-derives `summary_refs`/`observed_tool_refs`/
    /// `stop_reason`/`observed_terminal_event` from the projected event KIND and
    /// the persisted item ref -- the same classification `finish_turn` makes
    /// from the live batch. The `replay` field is left at its default because it
    /// is the volatile append-count diagnostic, not replay-stable state.
    ///
    /// It backs the restart/replay tests that enforce the replay-stability
    /// invariant for both the fake handle and RTL5's `RealBoundaryController`
    /// (the production consumer reconstructs a turn outcome after a restart via
    /// `RealBoundaryController::core`). It is also the production derivation the
    /// server uses to annotate a live-SPAWN turn whose ingested stdout batch is
    /// not threaded back in memory (RTL12): the annotation is reconstructed from
    /// the persisted, turn-keyed event log instead, so the loop's `TurnFinished`
    /// stays an honest annotation of what the dispatch run actually projected.
    pub fn reconstruct_turn_finished(
        &self,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
    ) -> StateResult<TurnFinished> {
        let mut events = self
            .state
            .recent_events_for_session(&refs.session_id, 256)?;
        // recent_events_for_session returns ascending sequence order already.
        events.retain(|event| event.turn_id.as_deref() == Some(turn_id.as_str()));
        let mut summary_refs = Vec::new();
        let mut observed_tool_refs = Vec::new();
        let mut terminal_outcome = None;
        for event in &events {
            match event.kind.as_str() {
                "session.summary_updated" => {
                    summary_refs.push(persisted_turn_ref(event));
                }
                // The projected tool kinds; `tool.observation_recorded` shares
                // the same item ref so dedup keeps a single ref per tool.
                "tool.call_requested"
                | "tool.invocation_started"
                | "tool.call_completed"
                | "tool.observation_recorded" => {
                    let tool_ref = persisted_turn_ref(event);
                    if !observed_tool_refs.contains(&tool_ref) {
                        observed_tool_refs.push(tool_ref);
                    }
                }
                "evidence.recorded" => terminal_outcome = Some(AdapterTerminalOutcome::Completed),
                "session.interrupted" => {
                    terminal_outcome = Some(AdapterTerminalOutcome::Interrupted)
                }
                "run.exited" => terminal_outcome = Some(AdapterTerminalOutcome::Failed),
                _ => {}
            }
        }
        Ok(TurnFinished {
            turn_id: turn_id.clone(),
            stop_reason: terminal_outcome
                .map(TurnStopReason::from_terminal_outcome)
                .unwrap_or(TurnStopReason::Completed),
            observed_terminal_event: terminal_outcome.is_some(),
            summary_refs,
            observed_tool_refs,
            replay: AdapterReplayReport::default(),
        })
    }

    /// Map the controller `interrupt` command onto the loop: terminate the
    /// in-flight turn and emit a [`TurnFinished`] with
    /// [`TurnStopReason::Interrupted`].
    ///
    /// This drives the existing [`Self::interrupt`] command (which records the
    /// `session.interrupted` event and updates the read models) so the loop has
    /// one interrupt path, then annotates it with the turn outcome. The turn id
    /// is keyed onto the persisted `session.interrupted` event so the outcome's
    /// `turn_id` is honored by event-log/projection queries, not cosmetic.
    pub fn interrupt_turn(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.interrupt_with_turn(registration, refs, reason, Some(turn_id))?;
        Ok(command_turn_finished(turn_id, TurnStopReason::Interrupted))
    }

    /// Map the controller `stop` command onto the loop: stop the run and emit a
    /// [`TurnFinished`] with [`TurnStopReason::Stopped`]. The turn id is keyed
    /// onto the persisted `session.stopped` event so the outcome is locatable by
    /// turn.
    pub fn stop_turn(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.stop_with_turn(registration, refs, reason, Some(turn_id))?;
        Ok(command_turn_finished(turn_id, TurnStopReason::Stopped))
    }
}

/// Build the `TurnFinished` for a controller command (`interrupt`/`stop`) path.
///
/// These paths drive a single terminal `session.interrupted`/`session.stopped`
/// event rather than an adapter batch, so they carry no summary/tool refs and a
/// default (volatile) replay report; the replay-stable content is the keyed
/// `turn_id` + `stop_reason`, persisted onto the terminal event.
fn command_turn_finished(turn_id: &TurnId, stop_reason: TurnStopReason) -> TurnFinished {
    TurnFinished {
        turn_id: turn_id.clone(),
        stop_reason,
        observed_terminal_event: true,
        summary_refs: Vec::new(),
        observed_tool_refs: Vec::new(),
        replay: AdapterReplayReport::default(),
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
    let mut terminal_outcome = None;
    // Classify each event through the SAME taxonomy the projection path uses
    // (NormalizedAdapterEvent::is_summary_event / is_tool_event /
    // terminal_outcome), so the outcome's refs/stop_reason cannot drift from
    // what was actually projected.
    for event in adapter_events {
        if event.is_summary_event() {
            summary_refs.push(turn_ref_for(event));
        } else if event.is_tool_event() {
            let tool_ref = turn_ref_for(event);
            if !observed_tool_refs.contains(&tool_ref) {
                observed_tool_refs.push(tool_ref);
            }
        } else if let Some(outcome) = event.terminal_outcome() {
            terminal_outcome = Some(outcome);
        }
    }
    TurnFinished {
        turn_id: turn_id.clone(),
        // A batch with no terminal adapter event is treated as a completed
        // single observe->project cycle for phase 1: the loop emitted, there is
        // nothing in flight, and the read models reflect the projected items.
        stop_reason: terminal_outcome
            .map(TurnStopReason::from_terminal_outcome)
            .unwrap_or(TurnStopReason::Completed),
        // Replay-stable: derived from the batch, not the append-counting replay
        // report (which is zero on an idempotent re-run).
        observed_terminal_event: terminal_outcome.is_some(),
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

/// The same stable ref as [`turn_ref_for`], reconstructed from a PERSISTED event
/// row. `item_id` holds the adapter's `external_item_ref`; the timeline key and
/// raw event hash fall-backs are recovered from the persisted `payload_json`
/// (written by `adapter_event_payload_json`), so a row with no `external_item_ref`
/// reconstructs the identical ref `finish_turn` would have produced.
fn persisted_turn_ref(event: &EventRecord) -> String {
    event
        .item_id
        .clone()
        .or_else(|| payload_field(&event.payload_json, "timeline_key"))
        .or_else(|| payload_field(&event.payload_json, "raw_event_hash"))
        .unwrap_or_else(|| event.payload_json.clone())
}

/// Extract a flat `"name":"value"` string field from the controller's
/// hand-built adapter-replay payload JSON. Returns `None` for the sentinel
/// `"none"` value the payload writer uses for absent optionals.
fn payload_field(payload_json: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\":\"");
    let start = payload_json.find(&needle)? + needle.len();
    let rest = &payload_json[start..];
    let end = rest.find('"')?;
    let value = &rest[..end];
    if value == "none" {
        None
    } else {
        Some(value.to_string())
    }
}
