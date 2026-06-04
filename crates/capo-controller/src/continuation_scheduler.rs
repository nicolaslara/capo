//! GA4 (goal-orchestration GO8): the safe-boundary continuation scheduler.
//!
//! What this is and what it is NOT. The scheduler is a PURE state machine:
//! [`ContinuationScheduler::decide`] takes a snapshot of inputs
//! ([`SchedulerInputs`]) and returns one of `continue | pause | block |
//! budget-limit | no-progress-suppress` ([`ContinuationDecision`]). It performs
//! NO I/O, appends NO event, and holds NO state of its own -- the same inputs
//! always yield the same decision, so every branch is exhaustively testable
//! without a database or a live provider. The controller-side wiring
//! ([`FakeBoundaryController::evaluate_continuation`] /
//! [`FakeBoundaryController::evaluate_and_record_continuation`]) is the only thing
//! that touches persisted state: it reads the goal/continuation projections,
//! folds them into [`SchedulerInputs`], calls the pure decision, and (for the
//! recording variant) durably records the decision through the GA1
//! `goal.continuation_decision_recorded` event + [`GoalContinuationProjection`],
//! aborting the run on a budget breach through the existing RTL7
//! [`FakeBoundaryController::abort_run_for_ceiling`] path.
//!
//! Opt-in only (GO8 + Non-Goals). Automatic continuation is NEVER on by default.
//! The scheduler will only ever return `continue` when [`SchedulerInputs::enabled`]
//! is set -- the explicit operator/config enablement. With `enabled = false` the
//! scheduler short-circuits to `pause` with reason `not_enabled`, so the
//! safe-boundary path cannot run unattended unless an operator turned it on.
//!
//! Safe-boundary preconditions (GO8). A goal may be continued ONLY at a safe
//! boundary: the goal is active, the runtime and session are idle, no user input
//! is queued, no permission is pending, the capability profile is still valid,
//! budget is available, there is no recent no-progress suppression, AND no
//! conflicting `safety-gates` workspace lock is held by another writer. The lock
//! check consumes the SG5 single-writer write lease this crate already builds
//! (`workspace_lock.rs`): [`SchedulerInputs::conflicting_workspace_lock`] is the
//! "no conflicting workspace lock" precondition GO8 names, fed from
//! [`FakeBoundaryController::workspace_lease_holder`].
//!
//! Checkpoint/verification substrate is REQUIRED (GO8 + knowledge.md). If the
//! next step would write source, the scheduler refuses to continue unless a
//! checkpoint boundary AND the verification runner are present
//! ([`SchedulerInputs::next_step_writes_source`] gated by
//! [`SchedulerInputs::checkpoint_boundary_available`] and
//! [`SchedulerInputs::verification_runner_available`]). This keeps the first
//! unattended writes confined and reversible: a goal whose next step writes
//! source without a checkpoint boundary is `pause`d, never continued.
//!
//! No-progress / spin guard (GO8). A continuation that made no MATERIAL progress
//! suppresses the next automatic continuation until strategy changes:
//! [`SchedulerInputs::last_continuation_made_no_progress`] forces
//! `no-progress-suppress`, and the suppression is only cleared by
//! [`SchedulerInputs::strategy_changed_since_suppression`] (operator/planner
//! intervention). This stops the scheduler burning budget on a spinning loop.
//!
//! Budget / blocked transitions (GO8). Budget exhaustion is a terminal
//! `budget-limit` decision; the recording path pairs it with the RTL7
//! `run.aborted` event so an exhausted goal stops durably rather than silently.
//! A raised blocker is a `block` decision. The decision priority is fixed so the
//! outcome is deterministic for a given snapshot (see [`ContinuationScheduler::
//! decide`]).
//!
//! Budget model (GA0 open question resolved). The scheduler treats the
//! `GoalBudget` as COMPOSING the RTL7 per-run [`RunResourceCeiling`], not
//! replacing it: [`SchedulerInputs::budget`] carries the goal-level
//! ceiling+usage, and the run-level ceiling still fires independently in the
//! loop. Budget is "available" iff the goal ceiling is not yet breached by the
//! accrued goal usage. This keeps the RTL7 floor intact (it bounds a single run
//! even with no goal) while the goal budget bounds the continuation series across
//! runs.

use capo_state::{GoalContinuationProjection, GoalProjection};

use super::*;
use crate::resource_ceiling::{CeilingBreach, RunResourceCeiling, RunResourceUsage};

/// GA4 (GO8): the decision the scheduler produces for a continuation evaluation.
///
/// Exactly the five outcomes GO8 names. The decision is a pure function of
/// [`SchedulerInputs`]; nothing here performs I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationDecision {
    /// Continue the goal: every safe-boundary precondition holds and the budget
    /// is available. This is the ONLY outcome that drives another automatic turn,
    /// and it is reachable only when continuation is explicitly enabled.
    Continue,
    /// Pause: a safe-boundary precondition does not hold (input queued, not idle,
    /// permission pending, capability invalid, would write source without a
    /// checkpoint boundary, or continuation is not enabled). The carried reason
    /// code names which.
    Pause,
    /// Block: the goal is blocked (a raised blocker). The scheduler will not
    /// continue a blocked goal.
    Block,
    /// Budget-limit: the goal budget is exhausted. The recording path pairs this
    /// with a durable `run.aborted` event.
    BudgetLimit,
    /// No-progress-suppress: the previous continuation made no material progress
    /// and strategy has not changed, so the next automatic continuation is
    /// suppressed until an operator/planner intervenes.
    NoProgressSuppress,
}

impl ContinuationDecision {
    /// The stable machine string recorded in the [`GoalContinuationProjection`]
    /// `decision` column. Matches the GO8 vocabulary exactly.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Pause => "pause",
            Self::Block => "block",
            Self::BudgetLimit => "budget-limit",
            Self::NoProgressSuppress => "no-progress-suppress",
        }
    }

    /// Whether this decision authorizes another automatic turn.
    pub const fn continues(self) -> bool {
        matches!(self, Self::Continue)
    }
}

/// GA4 (GO8): the goal-level budget the continuation series runs inside.
///
/// This COMPOSES the RTL7 per-run [`RunResourceCeiling`] rather than replacing
/// it (resolving the GA0/GA4 open question): the run-level ceiling still bounds a
/// single run in the loop, and this goal ceiling bounds the accrued usage ACROSS
/// the continuation series. Budget is "available" iff the accrued goal usage has
/// not breached the goal ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GoalBudget {
    /// The goal-level ceiling (turns / wall-clock / token-cost) for the whole
    /// continuation series. Reuses the RTL7 ceiling type so the goal budget and
    /// the run floor share one classifier.
    pub ceiling: RunResourceCeiling,
    /// The usage accrued across the goal's attempts so far.
    pub usage: RunResourceUsage,
}

impl GoalBudget {
    /// An unbounded goal budget (the run-level RTL7 floor still applies).
    pub const fn unbounded() -> Self {
        Self {
            ceiling: RunResourceCeiling::unbounded(),
            usage: RunResourceUsage {
                turns_taken: 0,
                wall_clock_elapsed: std::time::Duration::ZERO,
                token_cost: 0,
            },
        }
    }

    /// The first goal-budget breach, if the accrued usage has exhausted the goal
    /// ceiling. `None` means budget is still available.
    pub fn breach(&self) -> Option<CeilingBreach> {
        self.ceiling.breach(self.usage)
    }

    /// Whether the goal still has budget to continue.
    pub fn available(&self) -> bool {
        self.breach().is_none()
    }
}

/// GA4 (GO8): the snapshot the pure scheduler decides over.
///
/// Every field is an OBSERVED condition or an explicit operator/config flag; the
/// scheduler never reaches outside this struct. Construct it from persisted goal
/// state plus the caller's runtime-condition snapshot (see
/// [`FakeBoundaryController::evaluate_continuation`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerInputs {
    /// Explicit operator/config enablement. Automatic continuation is opt-in
    /// only: with `enabled = false` the scheduler never returns `continue`.
    pub enabled: bool,
    /// The goal is currently `active` (not paused/blocked/cleared). A
    /// non-active goal is never continued.
    pub goal_active: bool,
    /// The goal is blocked (a raised blocker). Drives a `block` decision.
    pub goal_blocked: bool,
    /// The runtime is idle (no in-flight turn / live process running).
    pub runtime_idle: bool,
    /// The session is idle (no in-flight dispatch on the bound session).
    pub session_idle: bool,
    /// There is queued user input awaiting handling. A safe boundary requires
    /// none (the operator's input takes precedence over auto-continuation).
    pub user_input_queued: bool,
    /// A permission request is pending. A safe boundary requires none.
    pub permission_pending: bool,
    /// The capability profile bound to the goal is still valid.
    pub capability_profile_valid: bool,
    /// The next planned step would WRITE source. When true, continuation requires
    /// both a checkpoint boundary and the verification runner to be present.
    pub next_step_writes_source: bool,
    /// A checkpoint boundary is available (the SG checkpoint/rollback runner is
    /// present). Required before any source-writing continuation.
    pub checkpoint_boundary_available: bool,
    /// The verification runner is available (observed evidence, not agent prose).
    /// Required before any source-writing continuation.
    pub verification_runner_available: bool,
    /// Another writer holds the `safety-gates` single-writer workspace lock.
    /// A conflicting lock blocks a safe-boundary continuation.
    pub conflicting_workspace_lock: bool,
    /// The previous continuation made no MATERIAL progress.
    pub last_continuation_made_no_progress: bool,
    /// Strategy changed since the last no-progress suppression (operator/planner
    /// intervention), clearing the suppression.
    pub strategy_changed_since_suppression: bool,
    /// The goal-level budget (composes the RTL7 run ceiling).
    pub budget: GoalBudget,
}

impl SchedulerInputs {
    /// A maximally-permissive baseline: enabled, active, idle, no pending input/
    /// permission, capability valid, checkpoint + verification present, no
    /// conflicting lock, no no-progress suppression, unbounded budget, and the
    /// next step is read-only. `decide` over this returns [`ContinuationDecision::
    /// Continue`]. Tests flip ONE field to exercise one branch in isolation.
    pub fn ready_to_continue() -> Self {
        Self {
            enabled: true,
            goal_active: true,
            goal_blocked: false,
            runtime_idle: true,
            session_idle: true,
            user_input_queued: false,
            permission_pending: false,
            capability_profile_valid: true,
            next_step_writes_source: false,
            checkpoint_boundary_available: true,
            verification_runner_available: true,
            conflicting_workspace_lock: false,
            last_continuation_made_no_progress: false,
            strategy_changed_since_suppression: false,
            budget: GoalBudget::unbounded(),
        }
    }
}

/// GA4 (GO8): the pure safe-boundary continuation state machine.
///
/// Stateless: it is a namespace for the one pure decision function. The scheduler
/// holds nothing -- all state is in [`SchedulerInputs`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContinuationScheduler;

/// GA4 (GO8): the decision plus its stable machine reason code, returned by the
/// pure scheduler so the recording path can persist BOTH the decision and the
/// "why" without re-deriving it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContinuationOutcome {
    pub decision: ContinuationDecision,
    /// A stable machine reason code (e.g. `safe_boundary`, `not_enabled`,
    /// `input_queued`, `budget_exhausted`, `no_material_progress`,
    /// `writes_source_without_checkpoint`).
    pub reason: &'static str,
}

impl ContinuationScheduler {
    /// Decide whether to continue the goal, as a PURE function of `inputs`.
    ///
    /// The priority order is fixed so the outcome is deterministic for any given
    /// snapshot:
    ///
    /// 1. `block` -- a blocked goal (or a non-active goal that is blocked) never
    ///    continues.
    /// 2. `budget-limit` -- an exhausted goal budget is terminal; it outranks the
    ///    softer pause reasons so an exhausted goal stops durably.
    /// 3. `no-progress-suppress` -- a prior no-material-progress continuation
    ///    suppresses the next one until strategy changes.
    /// 4. `pause` -- any unmet safe-boundary precondition (not enabled, not
    ///    active, not idle, input queued, permission pending, capability invalid,
    ///    conflicting workspace lock, or would write source without a checkpoint
    ///    boundary / verification runner).
    /// 5. `continue` -- every precondition holds and budget is available.
    pub fn decide(inputs: &SchedulerInputs) -> ContinuationOutcome {
        // 1. A blocked goal never continues, regardless of anything else.
        if inputs.goal_blocked {
            return outcome(ContinuationDecision::Block, "goal_blocked");
        }

        // 2. Budget exhaustion is terminal and outranks the soft pause reasons:
        //    an exhausted goal must stop durably (the recording path aborts the
        //    run), not merely pause and get re-evaluated.
        if !inputs.budget.available() {
            return outcome(ContinuationDecision::BudgetLimit, "budget_exhausted");
        }

        // 3. No-progress / spin guard: a prior no-material-progress continuation
        //    suppresses the next automatic one until strategy changes. This
        //    outranks pause so a spinning loop is explicitly suppressed (and
        //    recorded as such) rather than silently paused.
        if inputs.last_continuation_made_no_progress && !inputs.strategy_changed_since_suppression {
            return outcome(
                ContinuationDecision::NoProgressSuppress,
                "no_material_progress",
            );
        }

        // 4. Safe-boundary preconditions. Any unmet one pauses with a specific
        //    reason code so the read model explains exactly why.
        if !inputs.enabled {
            return outcome(ContinuationDecision::Pause, "not_enabled");
        }
        if !inputs.goal_active {
            return outcome(ContinuationDecision::Pause, "goal_not_active");
        }
        if inputs.user_input_queued {
            return outcome(ContinuationDecision::Pause, "input_queued");
        }
        if inputs.permission_pending {
            return outcome(ContinuationDecision::Pause, "permission_pending");
        }
        if !inputs.runtime_idle {
            return outcome(ContinuationDecision::Pause, "runtime_busy");
        }
        if !inputs.session_idle {
            return outcome(ContinuationDecision::Pause, "session_busy");
        }
        if !inputs.capability_profile_valid {
            return outcome(ContinuationDecision::Pause, "capability_profile_invalid");
        }
        if inputs.conflicting_workspace_lock {
            return outcome(ContinuationDecision::Pause, "workspace_lock_conflict");
        }
        // The first-unattended-writes safety boundary: a source-writing next step
        // requires BOTH a checkpoint boundary (reversible) and the verification
        // runner (observed evidence). Refuse otherwise.
        if inputs.next_step_writes_source {
            if !inputs.checkpoint_boundary_available {
                return outcome(
                    ContinuationDecision::Pause,
                    "writes_source_without_checkpoint",
                );
            }
            if !inputs.verification_runner_available {
                return outcome(
                    ContinuationDecision::Pause,
                    "writes_source_without_verification",
                );
            }
        }

        // 5. Every precondition holds and budget is available: continue.
        outcome(ContinuationDecision::Continue, "safe_boundary")
    }
}

const fn outcome(decision: ContinuationDecision, reason: &'static str) -> ContinuationOutcome {
    ContinuationOutcome { decision, reason }
}

/// GA4 (GO8): the runtime-condition snapshot the caller supplies for one
/// continuation evaluation -- the part of [`SchedulerInputs`] that the controller
/// cannot read from persisted goal state alone (idle/queued/pending live
/// conditions plus the explicit enablement and the goal budget).
///
/// The controller folds this together with the OBSERVED persisted goal state
/// (goal active/blocked, the workspace lock holder) to build the full
/// [`SchedulerInputs`]. The no-progress and strategy-change signals are carried
/// HERE (not derived from the prior recorded decision) because they are the
/// caller's observations of the goal's actual progress -- deriving them from the
/// ledger's last decision would be circular. Keeping the live conditions in one
/// struct keeps the pure decision and its inputs explicit and testable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContinuationConditions {
    /// Explicit operator/config enablement (opt-in only).
    pub enabled: bool,
    pub runtime_idle: bool,
    pub session_idle: bool,
    pub user_input_queued: bool,
    pub permission_pending: bool,
    pub capability_profile_valid: bool,
    pub next_step_writes_source: bool,
    pub checkpoint_boundary_available: bool,
    pub verification_runner_available: bool,
    /// The caller's OBSERVATION that the prior continued turn made no MATERIAL
    /// progress (e.g. a zero progress-delta over the goal's evidence/requirement
    /// ledger between the turn that the last `continue` authorized and now). This
    /// is the organic source of the GO8 spin guard: it is observed by the caller
    /// from the goal's progress, NOT derived from the prior DECISION already being
    /// a suppression (which would be circular and could never fire the first
    /// suppression).
    pub last_continuation_made_no_progress: bool,
    /// The caller's OBSERVATION that strategy changed since the no-progress
    /// suppression (an operator/planner intervention). This clears the
    /// suppression: with it set, an otherwise-suppressed evaluation continues
    /// again. Without a real signal here a suppression would be a permanent trap.
    pub strategy_changed_since_suppression: bool,
    /// The goal-level budget (composes the RTL7 run ceiling).
    pub budget: GoalBudget,
}

impl FakeBoundaryController {
    /// GA4 (GO8): evaluate the continuation decision for a goal, PURELY.
    ///
    /// Reads the OBSERVED state the scheduler needs from persisted goal state --
    /// the goal's lifecycle status (active/blocked) and whether a conflicting
    /// `safety-gates` workspace lock is held by ANOTHER writer -- folds it together
    /// with the caller's live `conditions` (which carry the no-progress and
    /// strategy-change OBSERVATIONS, see [`ContinuationConditions`]), and returns
    /// the pure [`ContinuationScheduler::decide`] outcome. This appends NOTHING;
    /// it is the read-only evaluation used to decide before recording.
    ///
    /// The `workspace_scope`, when supplied, names the workspace root + the
    /// session the continuation would run on; a held lease owned by a DIFFERENT
    /// session is a conflicting lock. `None` means no workspace contention applies
    /// (e.g. a read-only continuation).
    pub fn evaluate_continuation(
        &self,
        goal_id: &GoalId,
        conditions: &ContinuationConditions,
        workspace_scope: Option<&WorkspaceLeaseScope>,
    ) -> StateResult<ContinuationOutcome> {
        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;

        let goal_active = goal.is_active();
        let goal_blocked = goal.status == GoalProjection::BLOCKED;

        // The no-progress / spin guard is driven by the caller's OBSERVATION of
        // whether the last continued turn made material progress, carried on
        // `conditions`. It is deliberately NOT derived from the prior recorded
        // DECISION being a suppression: that would be circular (a suppression is
        // the OUTPUT of the guard, so deriving the input from it could never fire
        // the FIRST suppression organically, and -- because a suppression re-records
        // itself -- could never clear either). The matching clear signal, also a
        // caller observation, is `strategy_changed_since_suppression`.

        // A conflicting workspace lock: a held lease whose holder is a DIFFERENT
        // session than the one the continuation would run on. A lease held by the
        // SAME session is not a conflict (the continuation already owns the
        // writer). With no scope, no workspace contention applies.
        let conflicting_workspace_lock = match workspace_scope {
            Some(scope) => self
                .workspace_lease_holder(scope)?
                .is_some_and(|lease| lease.holder_session_id != scope.session_id),
            None => false,
        };

        let inputs = SchedulerInputs {
            enabled: conditions.enabled,
            goal_active,
            goal_blocked,
            runtime_idle: conditions.runtime_idle,
            session_idle: conditions.session_idle,
            user_input_queued: conditions.user_input_queued,
            permission_pending: conditions.permission_pending,
            capability_profile_valid: conditions.capability_profile_valid,
            next_step_writes_source: conditions.next_step_writes_source,
            checkpoint_boundary_available: conditions.checkpoint_boundary_available,
            verification_runner_available: conditions.verification_runner_available,
            conflicting_workspace_lock,
            last_continuation_made_no_progress: conditions.last_continuation_made_no_progress,
            strategy_changed_since_suppression: conditions.strategy_changed_since_suppression,
            budget: conditions.budget,
        };

        Ok(ContinuationScheduler::decide(&inputs))
    }

    /// GA4 (GO8): evaluate AND durably record the continuation decision.
    ///
    /// Calls [`Self::evaluate_continuation`], then records the decision through the
    /// GA1 `goal.continuation_decision_recorded` event + [`GoalContinuationProjection`]
    /// so the "why did (or didn't) this goal continue?" answer is a derived read
    /// model. On a `budget-limit` decision it ALSO aborts the goal's attempt run
    /// through the existing RTL7 [`Self::abort_run_for_ceiling`] path, pairing the
    /// budget exhaustion with a durable `run.aborted` event (when the goal has an
    /// attempt run and the caller supplied the run refs + breach).
    ///
    /// `refs` is the goal's current attempt run, required only to pair a
    /// `budget-limit` with the `run.aborted` abort; pass `None` to record the
    /// decision without aborting (e.g. a read-only continuation series).
    ///
    /// Recording is idempotent on `(goal, continuation_id)`: the caller supplies a
    /// stable `continuation_id`, so a verbatim re-evaluation re-records nothing.
    pub fn evaluate_and_record_continuation(
        &self,
        goal_id: &GoalId,
        continuation_id: &str,
        conditions: &ContinuationConditions,
        workspace_scope: Option<&WorkspaceLeaseScope>,
        abort_refs: Option<(&FakeRunRefs, &TurnId)>,
    ) -> StateResult<ContinuationOutcome> {
        let outcome = self.evaluate_continuation(goal_id, conditions, workspace_scope)?;

        let goal = self
            .state
            .goal(goal_id)?
            .ok_or_else(|| missing_read_model("goal", goal_id))?;

        let projection = GoalContinuationProjection {
            continuation_id: continuation_id.to_string(),
            goal_id: goal_id.clone(),
            project_id: self.project_id.clone(),
            attempt_run_id: goal.attempt_run_id.clone(),
            decision: outcome.decision.as_str().to_string(),
            reason: outcome.reason.to_string(),
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "continuation_id": continuation_id,
            "goal_id": goal_id.as_str(),
            "decision": outcome.decision.as_str(),
            "reason": outcome.reason,
            "attempt_run_id": goal.attempt_run_id.as_ref().map(RunId::as_str),
        })
        .to_string();

        let event = NewEvent {
            event_id: format!("event-continuation-{continuation_id}"),
            kind: EventKind::ContinuationDecisionRecorded,
            actor: "capo-controller".to_string(),
            project_id: Some(self.project_id.clone()),
            task_id: goal.task_id.clone(),
            agent_id: goal.agent_id.clone(),
            session_id: goal.session_id.clone(),
            run_id: goal.attempt_run_id.clone(),
            turn_id: None,
            item_id: Some(continuation_id.to_string()),
            payload_json: payload,
            idempotency_key: Some(format!(
                "continuation:{}:{}",
                goal_id.as_str(),
                continuation_id
            )),
            redaction_state: RedactionState::Safe,
        };

        self.state
            .append_event(event, &[ProjectionRecord::GoalContinuation(projection)])?;

        // A `budget-limit` decision is terminal: pair it with the RTL7
        // `run.aborted` event so the exhausted goal stops durably rather than
        // merely recording a decision. Only abort when the caller supplied the run
        // refs (the goal has a live attempt run) and the goal budget actually
        // reports a breach (it always does on `budget-limit`, but reading it keeps
        // the abort reason precise).
        if outcome.decision == ContinuationDecision::BudgetLimit
            && let Some((refs, turn_id)) = abort_refs
            && let Some(breach) = conditions.budget.breach()
        {
            self.abort_run_for_ceiling(refs, turn_id, breach)?;
        }

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use capo_state::{
        AgentProjection, GoalProjection, NewEvent, ProjectionRecord, RunProjection,
        SessionProjection, TaskProjection,
    };

    use super::*;


    fn temp_root(name: &str) -> capo_tmptest::TempRoot {
        capo_tmptest::TempRoot::new(&format!("capo-ga4-{name}"))
    }

    const PROJECT: &str = "project-capo";
    const GOAL: &str = "goal-ga4";
    const TASK: &str = "task-ga4";
    const AGENT: &str = "agent-ga4";
    const SESSION: &str = "session-ga4";
    const RUN: &str = "run-ga4";

    fn open() -> (FakeBoundaryController, capo_tmptest::TempRoot) {
        let state_root = temp_root("state");
        let controller =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("controller");
        (controller, state_root)
    }

    fn seed(controller: &FakeBoundaryController, event_id: &str, records: &[ProjectionRecord]) {
        let mut event = NewEvent::new(event_id, EventKind::GoalCreated, "test-seed");
        event.project_id = Some(ProjectId::new(PROJECT));
        event.idempotency_key = Some(event_id.to_string());
        event.item_id = Some(event_id.to_string());
        controller
            .state
            .append_event(event, records)
            .expect("seed append");
    }

    fn goal_projection(status: &str) -> GoalProjection {
        GoalProjection {
            goal_id: GoalId::new(GOAL),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(TaskId::new(TASK)),
            agent_id: Some(AgentId::new(AGENT)),
            session_id: Some(SessionId::new(SESSION)),
            parent_goal_id: None,
            attempt_run_id: Some(RunId::new(RUN)),
            objective: "Drive the GA4 scheduler".to_string(),
            status: status.to_string(),
            success_criteria_json: "{}".to_string(),
            constraints_json: "{}".to_string(),
            verification_surface_json: "{}".to_string(),
            budget_json: "{}".to_string(),
            stop_conditions_json: "{}".to_string(),
            blocker_reason: String::new(),
            updated_sequence: 0,
        }
    }

    /// Seed a goal plus the run/session/agent/task projections an `abort_run_for_ceiling`
    /// needs, so the budget-limit abort path has a real run to terminate.
    fn seed_goal_with_run(controller: &FakeBoundaryController, status: &str) {
        seed(
            controller,
            "seed-goal-ga4",
            &[
                ProjectionRecord::Goal(goal_projection(status)),
                ProjectionRecord::Task(TaskProjection {
                    task_id: TaskId::new(TASK),
                    project_id: ProjectId::new(PROJECT),
                    title: "GA4".to_string(),
                    capo_execution_status: "in_progress".to_string(),
                    active_session_id: Some(SessionId::new(SESSION)),
                    latest_summary: None,
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: AgentId::new(AGENT),
                    project_id: ProjectId::new(PROJECT),
                    name: "ga4".to_string(),
                    status: "busy".to_string(),
                    current_session_id: Some(SessionId::new(SESSION)),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: SessionId::new(SESSION),
                    project_id: ProjectId::new(PROJECT),
                    task_id: Some(TaskId::new(TASK)),
                    agent_id: AgentId::new(AGENT),
                    title: "GA4".to_string(),
                    status: "running".to_string(),
                    current_goal: "Drive the GA4 scheduler".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: Some("ext-session-ga4".to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: RunId::new(RUN),
                    session_id: SessionId::new(SESSION),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        );
    }

    fn run_refs() -> FakeRunRefs {
        FakeRunRefs {
            task_id: TaskId::new(TASK),
            agent_id: AgentId::new(AGENT),
            session_id: SessionId::new(SESSION),
            run_id: RunId::new(RUN),
            runtime_process_ref: "rt-ga4".to_string(),
            external_session_ref: "ext-session-ga4".to_string(),
        }
    }

    fn lease_scope(session: &str) -> WorkspaceLeaseScope {
        WorkspaceLeaseScope {
            task_id: TaskId::new(TASK),
            agent_id: AgentId::new(AGENT),
            session_id: SessionId::new(session),
            run_id: RunId::new(RUN),
            turn_id: TurnId::new("turn-ga4"),
            workspace_root: "/w/capo".to_string(),
        }
    }

    // ----- Pure state-machine branch coverage (no DB) -----------------------

    #[test]
    fn pure_scheduler_continues_at_a_safe_boundary() {
        let inputs = SchedulerInputs::ready_to_continue();
        let out = ContinuationScheduler::decide(&inputs);
        assert_eq!(out.decision, ContinuationDecision::Continue);
        assert_eq!(out.reason, "safe_boundary");
    }

    #[test]
    fn pure_scheduler_never_continues_when_not_enabled() {
        let inputs = SchedulerInputs {
            enabled: false,
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&inputs);
        assert_eq!(out.decision, ContinuationDecision::Pause);
        assert_eq!(out.reason, "not_enabled");
        assert!(!out.decision.continues());
    }

    #[test]
    fn pure_scheduler_pauses_on_each_unsafe_boundary_condition() {
        // Each unmet safe-boundary precondition pauses with a specific reason.
        let cases: &[(SchedulerInputs, &str)] = &[
            (
                SchedulerInputs {
                    goal_active: false,
                    ..SchedulerInputs::ready_to_continue()
                },
                "goal_not_active",
            ),
            (
                SchedulerInputs {
                    user_input_queued: true,
                    ..SchedulerInputs::ready_to_continue()
                },
                "input_queued",
            ),
            (
                SchedulerInputs {
                    permission_pending: true,
                    ..SchedulerInputs::ready_to_continue()
                },
                "permission_pending",
            ),
            (
                SchedulerInputs {
                    runtime_idle: false,
                    ..SchedulerInputs::ready_to_continue()
                },
                "runtime_busy",
            ),
            (
                SchedulerInputs {
                    session_idle: false,
                    ..SchedulerInputs::ready_to_continue()
                },
                "session_busy",
            ),
            (
                SchedulerInputs {
                    capability_profile_valid: false,
                    ..SchedulerInputs::ready_to_continue()
                },
                "capability_profile_invalid",
            ),
            (
                SchedulerInputs {
                    conflicting_workspace_lock: true,
                    ..SchedulerInputs::ready_to_continue()
                },
                "workspace_lock_conflict",
            ),
        ];
        for (inputs, expected_reason) in cases {
            let out = ContinuationScheduler::decide(inputs);
            assert_eq!(
                out.decision,
                ContinuationDecision::Pause,
                "expected pause for reason {expected_reason}"
            );
            assert_eq!(out.reason, *expected_reason);
        }
    }

    #[test]
    fn pure_scheduler_refuses_source_write_without_checkpoint_or_verification() {
        // Writing source needs a checkpoint boundary...
        let no_checkpoint = SchedulerInputs {
            next_step_writes_source: true,
            checkpoint_boundary_available: false,
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&no_checkpoint);
        assert_eq!(out.decision, ContinuationDecision::Pause);
        assert_eq!(out.reason, "writes_source_without_checkpoint");

        // ...and the verification runner.
        let no_verification = SchedulerInputs {
            next_step_writes_source: true,
            verification_runner_available: false,
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&no_verification);
        assert_eq!(out.decision, ContinuationDecision::Pause);
        assert_eq!(out.reason, "writes_source_without_verification");

        // With both present, a source-writing step continues.
        let with_substrate = SchedulerInputs {
            next_step_writes_source: true,
            ..SchedulerInputs::ready_to_continue()
        };
        assert_eq!(
            ContinuationScheduler::decide(&with_substrate).decision,
            ContinuationDecision::Continue
        );
    }

    #[test]
    fn pure_scheduler_blocks_a_blocked_goal_over_every_other_signal() {
        // `block` outranks everything: even with budget exhausted and input
        // queued, a blocked goal blocks.
        let inputs = SchedulerInputs {
            goal_blocked: true,
            user_input_queued: true,
            budget: GoalBudget {
                ceiling: RunResourceCeiling::max_turns(1),
                usage: RunResourceUsage {
                    turns_taken: 5,
                    ..RunResourceUsage::default()
                },
            },
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&inputs);
        assert_eq!(out.decision, ContinuationDecision::Block);
        assert_eq!(out.reason, "goal_blocked");
    }

    #[test]
    fn pure_scheduler_budget_limit_outranks_soft_pause() {
        // An exhausted budget is terminal and outranks a soft pause reason
        // (input queued), so the decision is `budget-limit`, not `pause`.
        let inputs = SchedulerInputs {
            user_input_queued: true,
            budget: GoalBudget {
                ceiling: RunResourceCeiling::max_turns(2),
                usage: RunResourceUsage {
                    turns_taken: 3,
                    ..RunResourceUsage::default()
                },
            },
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&inputs);
        assert_eq!(out.decision, ContinuationDecision::BudgetLimit);
        assert_eq!(out.reason, "budget_exhausted");
    }

    #[test]
    fn pure_scheduler_no_progress_suppresses_until_strategy_changes() {
        // A prior no-material-progress continuation suppresses the next one.
        let suppressed = SchedulerInputs {
            last_continuation_made_no_progress: true,
            strategy_changed_since_suppression: false,
            ..SchedulerInputs::ready_to_continue()
        };
        let out = ContinuationScheduler::decide(&suppressed);
        assert_eq!(out.decision, ContinuationDecision::NoProgressSuppress);
        assert_eq!(out.reason, "no_material_progress");

        // Once strategy changes, the suppression clears and the goal continues.
        let resumed = SchedulerInputs {
            last_continuation_made_no_progress: true,
            strategy_changed_since_suppression: true,
            ..SchedulerInputs::ready_to_continue()
        };
        assert_eq!(
            ContinuationScheduler::decide(&resumed).decision,
            ContinuationDecision::Continue
        );
    }

    // ----- Controller wiring over persisted state ----------------------------

    fn ready_conditions() -> ContinuationConditions {
        ContinuationConditions {
            enabled: true,
            runtime_idle: true,
            session_idle: true,
            user_input_queued: false,
            permission_pending: false,
            capability_profile_valid: true,
            next_step_writes_source: false,
            checkpoint_boundary_available: true,
            verification_runner_available: true,
            last_continuation_made_no_progress: false,
            strategy_changed_since_suppression: false,
            budget: GoalBudget::unbounded(),
        }
    }

    #[test]
    fn controller_records_a_continue_decision_at_a_safe_boundary() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        let out = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-1",
                &ready_conditions(),
                None,
                None,
            )
            .expect("decision");
        assert_eq!(out.decision, ContinuationDecision::Continue);

        let recorded = controller
            .state()
            .goal_continuations_for_goal(&GoalId::new(GOAL))
            .expect("continuations");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].decision, "continue");
        assert_eq!(recorded[0].reason, "safe_boundary");
        assert_eq!(recorded[0].attempt_run_id, Some(RunId::new(RUN)));
    }

    #[test]
    fn controller_refuses_to_continue_when_another_writer_holds_the_workspace_lock() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        // Another session takes the single-writer workspace lease for the same
        // workspace root.
        let other = lease_scope("session-other");
        let acquired = controller
            .acquire_workspace_write_lease(&other)
            .expect("acquire");
        assert!(acquired.may_write());

        // The continuation would run on our session, which does NOT hold the lock.
        let ours = lease_scope(SESSION);
        let out = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-lock",
                &ready_conditions(),
                Some(&ours),
                None,
            )
            .expect("decision");
        assert_eq!(out.decision, ContinuationDecision::Pause);
        assert_eq!(out.reason, "workspace_lock_conflict");

        // The SAME session holding the lock on the SAME key is NOT a conflict.
        // Release the other writer's lease, then have OUR session take the lease
        // on the SAME workspace root (`/w/capo`), and evaluate a continuation that
        // runs on OUR session over that exact key. The held lease exists and is on
        // the contended key, but its holder is us, so the
        // `holder_session_id == scope.session_id` non-conflict arm fires and the
        // continuation proceeds.
        controller
            .release_workspace_write_lease(&other, "test-handoff")
            .expect("release other");
        let acquired_self = controller
            .acquire_workspace_write_lease(&ours)
            .expect("acquire self");
        assert!(acquired_self.may_write());
        // Sanity: the lease is genuinely held, on the same key, by our session.
        let held = controller
            .workspace_lease_holder(&ours)
            .expect("holder lookup")
            .expect("lease held");
        assert_eq!(held.holder_session_id, SessionId::new(SESSION));
        let out_self = controller
            .evaluate_continuation(&GoalId::new(GOAL), &ready_conditions(), Some(&ours))
            .expect("decision");
        assert_eq!(out_self.decision, ContinuationDecision::Continue);
    }

    #[test]
    fn controller_budget_limit_aborts_the_run_durably() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        // A goal budget that is already exhausted (1 turn ceiling, 2 turns used).
        let conditions = ContinuationConditions {
            budget: GoalBudget {
                ceiling: RunResourceCeiling::max_turns(1),
                usage: RunResourceUsage {
                    turns_taken: 2,
                    ..RunResourceUsage::default()
                },
            },
            ..ready_conditions()
        };
        let refs = run_refs();
        let turn_id = TurnId::new("turn-budget");
        let out = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-budget",
                &conditions,
                None,
                Some((&refs, &turn_id)),
            )
            .expect("decision");
        assert_eq!(out.decision, ContinuationDecision::BudgetLimit);
        assert_eq!(out.reason, "budget_exhausted");

        // The decision is recorded...
        let recorded = controller
            .state()
            .goal_continuations_for_goal(&GoalId::new(GOAL))
            .expect("continuations");
        assert_eq!(recorded.last().unwrap().decision, "budget-limit");

        // ...and the run is durably aborted via the RTL7 abort path.
        let run = controller
            .state()
            .run(&RunId::new(RUN))
            .expect("run lookup")
            .expect("run present");
        assert_eq!(run.status, "aborted");
    }

    #[test]
    fn controller_no_progress_suppression_blocks_the_next_continuation() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        // Drive the spin guard ORGANICALLY: a goal Continues (cont-1), the turn
        // it authorizes makes no material progress, and the caller observes that
        // on the NEXT evaluation via `last_continuation_made_no_progress`. No
        // suppression row is seeded -- the signal is the caller's observation of
        // the goal's progress, not the prior decision being a suppression.
        let first = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-1",
                &ready_conditions(),
                None,
                None,
            )
            .expect("first decision");
        assert_eq!(first.decision, ContinuationDecision::Continue);

        // The continued turn made no material progress: the next evaluation
        // suppresses, even at an otherwise-safe boundary.
        let no_progress = ContinuationConditions {
            last_continuation_made_no_progress: true,
            ..ready_conditions()
        };
        let out = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-next",
                &no_progress,
                None,
                None,
            )
            .expect("decision");
        assert_eq!(out.decision, ContinuationDecision::NoProgressSuppress);
        assert_eq!(out.reason, "no_material_progress");

        // The suppression is recorded so the "why didn't it continue?" answer is
        // a derived read model.
        let recorded = controller
            .state()
            .goal_continuations_for_goal(&GoalId::new(GOAL))
            .expect("continuations");
        assert_eq!(recorded.last().unwrap().decision, "no-progress-suppress");
    }

    #[test]
    fn controller_no_progress_suppression_clears_when_strategy_changes() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        // A no-progress turn suppresses the next continuation.
        let no_progress = ContinuationConditions {
            last_continuation_made_no_progress: true,
            ..ready_conditions()
        };
        let suppressed = controller
            .evaluate_and_record_continuation(
                &GoalId::new(GOAL),
                "cont-suppressed",
                &no_progress,
                None,
                None,
            )
            .expect("suppressed decision");
        assert_eq!(
            suppressed.decision,
            ContinuationDecision::NoProgressSuppress
        );

        // An operator/planner intervention changes strategy. The caller observes
        // that and the next evaluation continues again -- the suppression is NOT a
        // permanent trap. (The no-progress observation may still be carried; the
        // strategy-change signal clears it.)
        let strategy_changed = ContinuationConditions {
            last_continuation_made_no_progress: true,
            strategy_changed_since_suppression: true,
            ..ready_conditions()
        };
        let resumed = controller
            .evaluate_continuation(&GoalId::new(GOAL), &strategy_changed, None)
            .expect("resumed decision");
        assert_eq!(resumed.decision, ContinuationDecision::Continue);
        assert_eq!(resumed.reason, "safe_boundary");
    }

    #[test]
    fn controller_blocks_a_blocked_goal() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::BLOCKED);

        let out = controller
            .evaluate_continuation(&GoalId::new(GOAL), &ready_conditions(), None)
            .expect("decision");
        assert_eq!(out.decision, ContinuationDecision::Block);
        assert_eq!(out.reason, "goal_blocked");
    }

    #[test]
    fn controller_recording_is_idempotent_on_continuation_id() {
        let (controller, _root) = open();
        seed_goal_with_run(&controller, GoalProjection::ACTIVE);

        for _ in 0..2 {
            let _ = controller
                .evaluate_and_record_continuation(
                    &GoalId::new(GOAL),
                    "cont-dup",
                    &ready_conditions(),
                    None,
                    None,
                )
                .expect("decision");
        }
        let recorded = controller
            .state()
            .goal_continuations_for_goal(&GoalId::new(GOAL))
            .expect("continuations");
        assert_eq!(
            recorded.len(),
            1,
            "verbatim re-evaluation re-records nothing"
        );
    }

    #[test]
    fn controller_continuation_decisions_survive_restart_and_rebuild() {
        let state_root = temp_root("restart");
        {
            let controller = FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root)
                .expect("controller");
            seed_goal_with_run(&controller, GoalProjection::ACTIVE);
            let _ = controller
                .evaluate_and_record_continuation(
                    &GoalId::new(GOAL),
                    "cont-restart",
                    &ready_conditions(),
                    None,
                    None,
                )
                .expect("decision");
        }

        // Re-open over the same state root and rebuild projections from the event
        // log: the continuation decision rebuilds identically.
        let reopened =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("reopen");
        reopened.state().rebuild_projections().expect("rebuild");
        let recorded = reopened
            .state()
            .goal_continuations_for_goal(&GoalId::new(GOAL))
            .expect("continuations");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].decision, "continue");
        assert_eq!(recorded[0].reason, "safe_boundary");
    }

    // A small compile-time sanity check that the budget helpers behave.
    #[test]
    fn goal_budget_available_until_breached() {
        let mut budget = GoalBudget {
            ceiling: RunResourceCeiling::for_live_provider(3, Duration::from_secs(60), 1_000),
            usage: RunResourceUsage {
                turns_taken: 2,
                wall_clock_elapsed: Duration::from_secs(10),
                token_cost: 500,
            },
        };
        assert!(budget.available());
        budget.usage.turns_taken = 4;
        assert!(!budget.available());
        assert!(matches!(
            budget.breach(),
            Some(CeilingBreach::MaxTurns { .. })
        ));
    }
}
