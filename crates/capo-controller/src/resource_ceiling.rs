//! RTL7: the per-run resource ceiling and its controller-enforced abort.
//!
//! `real-turn-loop` ships a live workspace-write adapter in phase 1 while the
//! full goal model and its budget land later in `goal-autonomy`. That leaves a
//! live model editing a real repo with no bound on how long it runs, how many
//! turns it takes, or how much provider spend it accrues. The RTL safety floor
//! therefore carries a minimal per-run ceiling the moment the first live write
//! does: **max turns**, **max wall-clock**, and a hard **token/cost** ceiling.
//!
//! Three invariants keep this honest:
//!
//! - The ceiling is enforced by the CONTROLLER, in the loop. [`run_turn_within_ceiling`]
//!   accounts each turn against the ceiling BEFORE it projects, so a run that
//!   has already hit `max_turns` never projects another turn -- it aborts.
//!   [`RunResourceCeiling::breach`] is the single pure classifier the loop and
//!   the live arm (RTL9) both consult, so they cannot drift on what "over the
//!   ceiling" means.
//! - Exceeding any ceiling emits a durable `run.aborted` event (with a `Run`
//!   projection of status `aborted`) and, on the live path, stops the run
//!   through the RTL6 hard kill. The `run.aborted` event is idempotent on
//!   `(run_id, breach)` and its projection rebuilds identically on replay, so an
//!   aborted run stays aborted after a restart.
//! - The wall-clock ceiling wires to the EXISTING live-provider timeout path
//!   (`wait_running_with_timeout`): a live-provider task derives its
//!   `timeout_seconds` from the ceiling, so the live Codex path always runs
//!   inside an active ceiling, never without one.
//!
//! Scope: this ceiling is a strict SUBSET of `goal-autonomy`'s `GoalBudget`.
//! `goal-autonomy` extends this enforcement floor (durable goal budget,
//! continuation accounting, auditor) rather than replacing it -- the run-level
//! floor here is the bound that holds even before a goal model exists.
//!
//! [`run_turn_within_ceiling`]: FakeBoundaryController::run_turn_within_ceiling

use std::time::Duration;

use super::*;

/// The per-run resource ceiling: the bound a single run may not exceed.
///
/// Every field is independently enforced; the first one a run trips aborts it.
/// A `None` field means that dimension is unbounded for this run, but a ceiling
/// is never absent on a live-provider path -- [`Self::for_live_provider`] is the
/// constructor that path uses, and it always sets a wall-clock bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunResourceCeiling {
    /// The maximum number of turns this run may take. `None` = unbounded.
    pub max_turns: Option<u32>,
    /// The maximum wall-clock duration this run may take. `None` = unbounded.
    /// The live-provider path wires this to `wait_running_with_timeout` so the
    /// process is killed at the deadline.
    pub max_wall_clock: Option<Duration>,
    /// The hard token/cost ceiling for this run, in provider cost units (e.g.
    /// tokens). `None` = unbounded.
    pub max_token_cost: Option<u64>,
}

impl RunResourceCeiling {
    /// An unbounded ceiling. Used by deterministic paths that opt out of every
    /// bound; a live-provider path must NOT use this (see
    /// [`Self::for_live_provider`]).
    pub const fn unbounded() -> Self {
        Self {
            max_turns: None,
            max_wall_clock: None,
            max_token_cost: None,
        }
    }

    /// A ceiling bounding only the number of turns.
    pub const fn max_turns(max_turns: u32) -> Self {
        Self {
            max_turns: Some(max_turns),
            max_wall_clock: None,
            max_token_cost: None,
        }
    }

    /// The ceiling a live-provider task runs inside.
    ///
    /// A live-provider run always carries a wall-clock bound (it is wired to the
    /// runtime timeout), so this constructor never produces an unbounded
    /// wall-clock. The live Codex path (RTL9) constructs its ceiling here and
    /// derives `timeout_seconds` from [`Self::wall_clock_timeout_seconds`].
    pub const fn for_live_provider(
        max_turns: u32,
        max_wall_clock: Duration,
        max_token_cost: u64,
    ) -> Self {
        Self {
            max_turns: Some(max_turns),
            max_wall_clock: Some(max_wall_clock),
            max_token_cost: Some(max_token_cost),
        }
    }

    /// Whether this ceiling bounds wall-clock time -- the invariant a live
    /// provider task must satisfy ("never run without an active ceiling").
    pub const fn bounds_wall_clock(&self) -> bool {
        self.max_wall_clock.is_some()
    }

    /// The wall-clock timeout in whole seconds for the live-provider runtime
    /// wait, clamped to at least one second so a tiny ceiling cannot become a
    /// zero (i.e. immediate) timeout. Returns `None` for an unbounded ceiling.
    pub fn wall_clock_timeout_seconds(&self) -> Option<u64> {
        self.max_wall_clock.map(|d| d.as_secs().max(1))
    }

    /// Classify usage against the ceiling: the single pure decision the loop and
    /// the live arm both consult. Returns the FIRST breach in a fixed priority
    /// order (turns, then wall-clock, then token/cost) so the abort reason is
    /// deterministic for a given usage.
    pub fn breach(&self, usage: RunResourceUsage) -> Option<CeilingBreach> {
        if let Some(max) = self.max_turns
            && usage.turns_taken > max
        {
            return Some(CeilingBreach::MaxTurns {
                limit: max,
                observed: usage.turns_taken,
            });
        }
        if let Some(max) = self.max_wall_clock
            && usage.wall_clock_elapsed > max
        {
            return Some(CeilingBreach::WallClock {
                limit: max,
                observed: usage.wall_clock_elapsed,
            });
        }
        if let Some(max) = self.max_token_cost
            && usage.token_cost > max
        {
            return Some(CeilingBreach::TokenCost {
                limit: max,
                observed: usage.token_cost,
            });
        }
        None
    }
}

/// The accumulated resource usage of a run, accounted by the controller as the
/// loop drives turns.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunResourceUsage {
    /// Turns taken so far (the turn about to run is counted before it projects).
    pub turns_taken: u32,
    /// Wall-clock elapsed so far.
    pub wall_clock_elapsed: Duration,
    /// Token/cost accrued so far, in provider cost units.
    pub token_cost: u64,
}

/// Which ceiling a run exceeded. Carries the limit and the observed value so the
/// `run.aborted` payload records exactly what was tripped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CeilingBreach {
    MaxTurns { limit: u32, observed: u32 },
    WallClock { limit: Duration, observed: Duration },
    TokenCost { limit: u64, observed: u64 },
}

impl CeilingBreach {
    /// A stable machine code for the breached dimension, used in the event
    /// idempotency key and payload reason code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MaxTurns { .. } => "max_turns_exceeded",
            Self::WallClock { .. } => "max_wall_clock_exceeded",
            Self::TokenCost { .. } => "max_token_cost_exceeded",
        }
    }

    fn limit_value(&self) -> u64 {
        match self {
            Self::MaxTurns { limit, .. } => u64::from(*limit),
            Self::WallClock { limit, .. } => limit.as_secs(),
            Self::TokenCost { limit, .. } => *limit,
        }
    }

    fn observed_value(&self) -> u64 {
        match self {
            Self::MaxTurns { observed, .. } => u64::from(*observed),
            Self::WallClock { observed, .. } => observed.as_secs(),
            Self::TokenCost { observed, .. } => *observed,
        }
    }
}

/// What the loop did with a turn that was accounted against the ceiling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CeilingTurnOutcome {
    /// The turn was within the ceiling: it projected and produced this outcome.
    Completed(TurnFinished),
    /// The turn would have exceeded the ceiling: it was NOT projected; the run
    /// aborted with this breach and a `run.aborted` event was recorded.
    Aborted(CeilingBreach),
}

impl CeilingTurnOutcome {
    /// The turn outcome when within the ceiling, or `None` when the run aborted.
    pub fn finished(&self) -> Option<&TurnFinished> {
        match self {
            Self::Completed(finished) => Some(finished),
            Self::Aborted(_) => None,
        }
    }

    /// The breach when the run aborted, or `None` when the turn completed.
    pub fn breach(&self) -> Option<CeilingBreach> {
        match self {
            Self::Completed(_) => None,
            Self::Aborted(breach) => Some(*breach),
        }
    }
}

impl FakeBoundaryController {
    /// Run one loop turn under an active per-run ceiling.
    ///
    /// The ceiling is enforced BEFORE the turn projects: `usage_before` is the
    /// usage accrued prior to this turn, and the turn about to run is counted as
    /// one additional turn plus `turn_token_cost`. If that projected usage trips
    /// the ceiling, the run aborts -- the turn is never projected, a durable
    /// `run.aborted` event is recorded, and [`CeilingTurnOutcome::Aborted`] is
    /// returned. Otherwise the turn projects through the existing
    /// [`Self::run_turn`] path and [`CeilingTurnOutcome::Completed`] is returned.
    ///
    /// This is the controller-enforced abort the RTL7 acceptance requires: a
    /// scripted run that exceeds `max_turns` aborts with a `run.aborted` event
    /// and no further turns are projected.
    pub fn run_turn_within_ceiling(
        &self,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        adapter_events: &[NormalizedAdapterEvent],
        ceiling: &RunResourceCeiling,
        usage_before: RunResourceUsage,
        turn_token_cost: u64,
    ) -> StateResult<CeilingTurnOutcome> {
        let projected_usage = RunResourceUsage {
            turns_taken: usage_before.turns_taken.saturating_add(1),
            wall_clock_elapsed: usage_before.wall_clock_elapsed,
            token_cost: usage_before.token_cost.saturating_add(turn_token_cost),
        };
        if let Some(breach) = ceiling.breach(projected_usage) {
            // Abort BEFORE projecting: a run over the ceiling never appends
            // another turn's read models.
            self.abort_run_for_ceiling(refs, turn_id, breach)?;
            return Ok(CeilingTurnOutcome::Aborted(breach));
        }
        let finished = self.run_turn(refs, turn_id, adapter_events)?;
        Ok(CeilingTurnOutcome::Completed(finished))
    }

    /// Record a controller-enforced abort: append a durable `run.aborted` event
    /// and mark the run's projection `aborted`.
    ///
    /// On the deterministic path this is the whole abort (there is no live
    /// process). On the live path the caller pairs this with the RTL6 hard kill
    /// of the process group; the event recorded here is the run-level truth that
    /// the run was aborted by the ceiling -- distinct from the RTL6
    /// `run.hard_killed` (the emergency stop) and the RTL10 orphan recovery.
    ///
    /// Idempotent on `(run_id, breach.code())`: re-recording the same breach
    /// appends nothing new, so a restart/replay leaves the run aborted exactly
    /// once. The `Run` projection of status `aborted` rebuilds identically from
    /// the persisted event, so the run stays aborted after a rebuild.
    pub fn abort_run_for_ceiling(
        &self,
        refs: &FakeRunRefs,
        turn_id: &TurnId,
        breach: CeilingBreach,
    ) -> StateResult<()> {
        let mut event = scoped_event(
            &format!(
                "event-run-aborted-{}-{}",
                refs.run_id,
                stable_hash(format!("{}:{}", refs.run_id, breach.code()).as_bytes())
            ),
            EventKind::RunAborted,
            &self.project_id,
            &refs.task_id,
            &refs.agent_id,
            &refs.session_id,
            &refs.run_id,
        )
        .with_turn(turn_id.as_str());
        event.item_id = Some(refs.run_id.to_string());
        event.idempotency_key = Some(format!(
            "run-aborted:{}:{}:{}",
            self.project_id,
            refs.run_id,
            breach.code()
        ));
        let payload = format!(
            "{{\"reason_code\":\"{}\",\"limit\":{},\"observed\":{},\"status\":\"aborted\",\"enforced_by\":\"controller_resource_ceiling\"}}",
            breach.code(),
            breach.limit_value(),
            breach.observed_value(),
        );
        self.state.append_event(
            event.with_payload(payload),
            &[ProjectionRecord::Run(RunProjection {
                run_id: refs.run_id.clone(),
                session_id: refs.session_id.clone(),
                status: "aborted".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )?;
        Ok(())
    }
}
