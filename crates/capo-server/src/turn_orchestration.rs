//! RTL4: reconcile the turn loop with the existing dispatch pipeline.
//!
//! The single most important RTL4 decision (recorded in
//! `workpads/real-turn-loop/knowledge.md`) is that the turn loop has exactly
//! ONE execution path. The dispatch state machine -- `PlanDispatch` ->
//! `PreflightLiveProvider` -> `GateDispatch` ->
//! `RunDispatchLocal`/`RunLiveProviderLocal` -- is already a real multi-step
//! orchestration over the typed [`ServerCommand`] boundary. RTL4 makes the
//! loop's emit step DRIVE those primitives as its substrate rather than
//! re-running plan/gate/run beside them.
//!
//! Chosen call shape (the leaning resolved by RTL4): the loop INVOKES the
//! dispatch `ServerCommand`s through [`CapoServer::handle`], so the typed
//! boundary stays the single definition of plan/gate/run. [`run_dispatch_turn`]
//! is a thin orchestrator that sequences the existing commands and then
//! ANNOTATES the run it just drove with a [`TurnFinished`] derived from the
//! SAME normalized batch the dispatch run ingested
//! ([`FakeBoundaryController::derive_turn_finished`]). It adds no new event
//! kind, no second gate, and no second run-completion model -- a loop turn
//! produces the identical plan/gate/execution event sequence as the direct
//! command path for a scripted run.
//!
//! NON-GOAL ASSERTION (code-review contract, enforced by construction):
//! `run_dispatch_turn` never runs a provider without passing through the
//! existing gate/preflight. The deterministic path goes through
//! [`CapoServer::dispatch_gate_for_plan`] via `GateDispatch`; the live path
//! goes through [`CapoServer::preflight_live_provider`] via
//! `PreflightLiveProvider`. Both reuse the existing checks unchanged -- the
//! orchestrator only forwards `ServerCommand`s, it owns no provider-spawn or
//! gate-bypass path of its own. RTL5's `RealBoundaryController`
//! (`capo_controller::RealBoundaryController`) is the production consumer and
//! inherits this contract.

use capo_controller::{
    CeilingBreach, FakeBoundaryController, RunResourceCeiling, RunResourceUsage, TurnFinished,
};
use capo_core::{RunId, SessionId, TurnId};

use crate::util::parse_adapter_events;
use crate::{
    CapoServer, DispatchRunSummary, ServerCommand, ServerError, ServerRequest,
    ServerResponsePayload, ServerResult,
};

/// How a single turn drives the dispatch substrate.
pub enum DispatchTurnMode {
    /// Deterministic fixture path: `PlanDispatch` -> `GateDispatch` ->
    /// `RunDispatchLocal`, reusing [`CapoServer::dispatch_gate_for_plan`].
    DeterministicFixture {
        deterministic_opt_in: bool,
        fixture_name: String,
        fixture_jsonl: String,
    },
    /// Live-provider path: `PreflightLiveProvider` -> `RunLiveProviderLocal`,
    /// reusing [`CapoServer::preflight_live_provider`]. In phase 1 the run is
    /// driven with a mock provider output so the substrate is fully testable
    /// without a live provider; the real Codex round-trip plugs in at RTL9.
    ///
    /// RTL7: a live-provider turn always runs inside an active per-run resource
    /// ceiling, and `run_dispatch_turn` ENFORCES all three dimensions on this
    /// path -- not just wall-clock. The `ceiling` MUST bound wall-clock (the
    /// wall-clock ceiling is wired to the runtime's `wait_running_with_timeout`);
    /// `run_dispatch_turn` rejects a live turn whose ceiling does not. The live
    /// Codex path therefore never runs without a ceiling.
    ///
    /// `usage_before` is the per-run usage accrued by the loop BEFORE this turn;
    /// `run_dispatch_turn` accounts the turn about to run (one more turn plus
    /// `turn_token_cost` as a pre-turn estimate) and aborts the run -- WITHOUT
    /// spawning the provider -- if that projected usage trips the turns or
    /// token/cost ceiling. After the run, the wall-clock dimension is enforced
    /// from the runtime timeout (a `timed_out` run aborts via
    /// [`FakeBoundaryController::abort_run_for_ceiling`]) and the observed
    /// post-turn token cost is folded into the returned usage so the next
    /// turn's pre-turn check sees real spend.
    ///
    /// The payload is boxed because it is much larger than the deterministic
    /// arm; keeping the enum small avoids `clippy::large_enum_variant`.
    LiveProvider(Box<LiveProviderTurn>),
}

/// The inputs for one live-provider turn driven through the dispatch substrate.
/// Boxed inside [`DispatchTurnMode::LiveProvider`] so the enum stays small.
pub struct LiveProviderTurn {
    pub capability_profile: String,
    pub runtime_scope: String,
    pub credential_scan_policy: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub tool_wrapper_policy: String,
    pub live_provider_opt_in: bool,
    pub live_execution_opt_in: bool,
    pub mock_runtime_opt_in: bool,
    pub mock_provider_output_name: Option<String>,
    pub mock_provider_output_jsonl: Option<String>,
    /// The per-run resource ceiling this live turn runs inside. Its wall-clock
    /// bound drives the runtime timeout; a ceiling without a wall-clock bound is
    /// rejected.
    pub ceiling: RunResourceCeiling,
    /// Per-run usage accrued before this turn (turns/wall-clock/token cost). The
    /// loop carries this across turns; the turn about to run is counted on top of
    /// it for the pre-turn ceiling check.
    pub usage_before: RunResourceUsage,
    /// Pre-turn token/cost estimate for the turn about to run, counted against
    /// `max_token_cost` BEFORE the provider spawns. A live provider's real token
    /// cost is only known after the turn, so this is a bound, not a measurement;
    /// the observed post-turn cost (when the provider reports it) is folded into
    /// [`DispatchTurnOutcome::usage_after`].
    pub turn_token_cost: u64,
}

/// One turn's worth of inputs for the loop's dispatch-driven execution.
pub struct DispatchTurnRequest {
    pub agent_name: String,
    pub adapter: String,
    pub goal: String,
    pub workspace: String,
    pub artifacts: String,
    pub session_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub mode: DispatchTurnMode,
}

/// The outcome of driving one turn through the dispatch substrate: the dispatch
/// run summary (the existing run-completion truth) plus the loop's
/// [`TurnFinished`] annotation derived from the same ingested batch.
#[derive(Debug)]
pub struct DispatchTurnOutcome {
    pub run: DispatchRunSummary,
    pub finished: TurnFinished,
    /// The per-run usage AFTER this turn: `usage_before` plus this turn (one
    /// more turn and the observed-or-estimated token cost). The loop feeds this
    /// back as the next turn's `usage_before` so the ceiling is enforced across
    /// turns on the same substrate the live provider runs through. `None` on the
    /// deterministic-fixture path, which is not under a per-run ceiling.
    pub usage_after: Option<RunResourceUsage>,
    /// Set when the run was aborted by the resource ceiling on the live path:
    /// the over-ceiling turn that never spawned a provider (turns/token) or the
    /// wall-clock timeout (a `timed_out` run). A durable `run.aborted` event and
    /// the coordinated terminal projection set were recorded.
    pub ceiling_breach: Option<CeilingBreach>,
}

impl CapoServer {
    /// Drive one loop turn through the EXISTING dispatch primitives and emit a
    /// [`TurnFinished`] annotation.
    ///
    /// This is the RTL4 reconciliation point: a turn opens, the loop invokes the
    /// typed dispatch `ServerCommand`s (plan/gate/run for the deterministic
    /// substrate, preflight/run for the live substrate), the existing handlers
    /// project the normalized batch through
    /// [`FakeBoundaryController::apply_normalized_adapter_events_with_turn`], and
    /// the loop annotates the run with a `TurnFinished` derived from the same
    /// batch. The dispatch run-exit remains the single run-completion truth; the
    /// loop does not fork a second one.
    pub fn run_dispatch_turn(
        &self,
        request: DispatchTurnRequest,
    ) -> ServerResult<DispatchTurnOutcome> {
        // Plan: the loop drives the existing PlanDispatch command. This is the
        // same command, idempotency, and projection the direct path uses.
        let planned = self.handle(ServerRequest::cli(ServerCommand::PlanDispatch {
            agent_name: request.agent_name.clone(),
            adapter: request.adapter.clone(),
            goal: request.goal.clone(),
            workspace: request.workspace.clone(),
            artifacts: request.artifacts.clone(),
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            turn_id: request.turn_id.clone(),
            deterministic_opt_in: matches!(
                request.mode,
                DispatchTurnMode::DeterministicFixture {
                    deterministic_opt_in: true,
                    ..
                }
            ),
        }))?;
        let ServerResponsePayload::DispatchPlanned(_plan) = planned.payload else {
            return Err(ServerError::AdapterFixture(
                "loop turn plan step did not produce a dispatch plan".to_string(),
            ));
        };

        // `usage_after` is populated by the live arm so the loop can carry usage
        // across turns; `ceiling_breach` stays `None` on every non-abort return
        // (the abort path returns early from `abort_live_turn_for_ceiling`).
        let mut usage_after: Option<RunResourceUsage> = None;
        let ceiling_breach: Option<CeilingBreach> = None;
        let (run, batch_adapter, batch_jsonl) = match request.mode {
            DispatchTurnMode::DeterministicFixture {
                fixture_name,
                fixture_jsonl,
                ..
            } => {
                // Gate: reuse the existing dispatch_gate_for_plan via GateDispatch.
                // The loop never runs a provider without this gate.
                let plan_id = self.dispatch_plan_id_for_turn(
                    &request.adapter,
                    &request.agent_name,
                    &request.session_id,
                    &request.run_id,
                    &request.turn_id,
                    &request.goal,
                    &request.workspace,
                    &request.artifacts,
                )?;
                self.handle(ServerRequest::cli(ServerCommand::GateDispatch {
                    dispatch_plan_id: plan_id.clone(),
                }))?;
                // Run: drive the existing RunDispatchLocal, which projects the
                // normalized batch keyed to this turn.
                let run_response =
                    self.handle(ServerRequest::cli(ServerCommand::RunDispatchLocal {
                        dispatch_plan_id: plan_id,
                        fixture_name,
                        fixture_jsonl: fixture_jsonl.clone(),
                    }))?;
                let ServerResponsePayload::DispatchRun(run) = run_response.payload else {
                    return Err(ServerError::AdapterFixture(
                        "loop turn run step did not produce a dispatch run".to_string(),
                    ));
                };
                (run, request.adapter.clone(), fixture_jsonl)
            }
            DispatchTurnMode::LiveProvider(live) => {
                let LiveProviderTurn {
                    capability_profile,
                    runtime_scope,
                    credential_scan_policy,
                    raw_prompt_policy,
                    raw_output_policy,
                    tool_wrapper_policy,
                    live_provider_opt_in,
                    live_execution_opt_in,
                    mock_runtime_opt_in,
                    mock_provider_output_name,
                    mock_provider_output_jsonl,
                    ceiling,
                    usage_before,
                    turn_token_cost,
                } = *live;
                // RTL7: a live-provider turn must run inside an active ceiling.
                // The wall-clock bound is wired to the runtime timeout below; a
                // ceiling that does not bound wall-clock is rejected here, so the
                // live Codex path never runs without one.
                let Some(timeout_seconds) = ceiling.wall_clock_timeout_seconds() else {
                    return Err(ServerError::AdapterFixture(
                        "live-provider turn requires an active resource ceiling with a \
                         wall-clock bound"
                            .to_string(),
                    ));
                };

                // RTL7: enforce the turns and token/cost ceilings IN THE LOOP, on
                // the SAME substrate the live provider runs through. Account the
                // turn about to run (one more turn + its pre-turn token estimate)
                // BEFORE spawning the provider; if that projected usage trips the
                // ceiling, abort the run -- the provider never spawns, a durable
                // `run.aborted` event and the coordinated terminal projection set
                // are recorded, and the over-ceiling turn projects nothing.
                let projected_usage = RunResourceUsage {
                    turns_taken: usage_before.turns_taken.saturating_add(1),
                    wall_clock_elapsed: usage_before.wall_clock_elapsed,
                    token_cost: usage_before.token_cost.saturating_add(turn_token_cost),
                };
                if let Some(breach) = ceiling.breach(projected_usage) {
                    return self.abort_live_turn_for_ceiling(
                        &request.session_id,
                        &request.run_id,
                        &request.turn_id,
                        breach,
                    );
                }

                // Preflight: reuse the existing preflight_live_provider via
                // PreflightLiveProvider. The loop never runs a provider without
                // this preflight gate.
                let preflight =
                    self.handle(ServerRequest::cli(ServerCommand::PreflightLiveProvider {
                        agent_name: request.agent_name.clone(),
                        adapter: request.adapter.clone(),
                        goal: request.goal.clone(),
                        workspace: request.workspace.clone(),
                        artifacts: request.artifacts.clone(),
                        session_id: request.session_id.clone(),
                        run_id: request.run_id.clone(),
                        turn_id: request.turn_id.clone(),
                        capability_profile,
                        runtime_scope,
                        credential_scan_policy,
                        raw_prompt_policy,
                        raw_output_policy,
                        tool_wrapper_policy,
                        live_provider_opt_in,
                    }))?;
                let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload
                else {
                    return Err(ServerError::AdapterFixture(
                        "loop turn preflight step did not produce a live preflight".to_string(),
                    ));
                };
                let mock_jsonl = mock_provider_output_jsonl.clone();
                let run_response =
                    self.handle(ServerRequest::cli(ServerCommand::RunLiveProviderLocal {
                        dispatch_plan_id: preflight.dispatch_plan_id,
                        goal: request.goal.clone(),
                        live_execution_opt_in,
                        mock_runtime_opt_in,
                        mock_provider_output_name,
                        mock_provider_output_jsonl,
                        timeout_seconds,
                    }))?;
                let ServerResponsePayload::DispatchRun(run) = run_response.payload else {
                    return Err(ServerError::AdapterFixture(
                        "loop turn live run step did not produce a dispatch run".to_string(),
                    ));
                };

                // RTL7: the wall-clock dimension is enforced from the runtime
                // timeout. `wait_running_with_timeout` already hard-kills the
                // process group at the deadline and reports a `timed_out` run, so
                // here we pair that kill with a durable `run.aborted` event +
                // terminal projection set (distinct from the timed-out dispatch
                // execution record the live path already wrote).
                if run.status == "timed_out" {
                    let breach = CeilingBreach::WallClock {
                        limit: ceiling.max_wall_clock.unwrap_or_default(),
                        // The process ran at least to the deadline; report the
                        // limit as the observed elapsed so `observed >= limit`.
                        observed: ceiling.max_wall_clock.unwrap_or_default(),
                    };
                    return self.abort_live_turn_for_ceiling(
                        &request.session_id,
                        &request.run_id,
                        &request.turn_id,
                        breach,
                    );
                }

                // Account the turn that ran: fold the observed post-turn token
                // cost (when the provider reports it) into the per-run usage, so
                // the next turn's pre-turn check sees real spend rather than only
                // an estimate. With no observed cost yet (phase-1 mock/live has no
                // token source), the pre-turn estimate stands.
                usage_after = Some(RunResourceUsage {
                    turns_taken: projected_usage.turns_taken,
                    wall_clock_elapsed: usage_before.wall_clock_elapsed,
                    token_cost: usage_before
                        .token_cost
                        .saturating_add(run.observed_token_cost.unwrap_or(turn_token_cost)),
                });
                (run, request.adapter.clone(), mock_jsonl.unwrap_or_default())
            }
        };

        // Emit: annotate the run we just drove with a TurnFinished derived from
        // the SAME batch the dispatch run ingested. This is the loop's outcome
        // over the existing run-completion truth -- not a second one.
        let finished =
            self.turn_finished_for_run(&request.turn_id, &batch_adapter, &batch_jsonl, &run)?;
        Ok(DispatchTurnOutcome {
            run,
            finished,
            usage_after,
            ceiling_breach,
        })
    }

    /// Abort a live turn that breached the resource ceiling: record the durable
    /// `run.aborted` event + coordinated terminal projection set through the
    /// controller's [`FakeBoundaryController::abort_run_for_ceiling`], then
    /// return a `DispatchTurnOutcome` carrying the breach and a no-ref
    /// `TurnFinished` (the over-ceiling turn projects nothing).
    ///
    /// The provider is NOT spawned for a pre-turn turns/token breach; for the
    /// wall-clock breach the runtime timeout already hard-killed the process
    /// group (`wait_running_with_timeout`), so this records the run-level abort
    /// truth that pairs with that kill -- distinct from the timed-out dispatch
    /// execution record the live path already wrote.
    fn abort_live_turn_for_ceiling(
        &self,
        session_id: &str,
        run_id: &str,
        turn_id: &str,
        breach: CeilingBreach,
    ) -> ServerResult<DispatchTurnOutcome> {
        let session = SessionId::new(session_id);
        let run_id = RunId::new(run_id);
        let (_session, run_projection, _agent, refs) =
            self.run_refs_for_session_run(&session, &run_id)?;
        let turn = TurnId::new(turn_id);
        self.controller
            .abort_run_for_ceiling(&refs, &turn, breach)
            .map_err(ServerError::State)?;
        let finished = FakeBoundaryController::derive_turn_finished(&turn, &[], Default::default());
        // The aborted run's summary reflects the ceiling stop: no provider
        // execution beyond what already ran, and a status that mirrors the now
        // `aborted` run projection. Reuse the persisted run id/session id.
        let run = DispatchRunSummary {
            dispatch_plan_id: String::new(),
            dispatch_execution_id: String::new(),
            adapter: String::new(),
            session_id: refs.session_id.clone(),
            run_id: run_projection.run_id.clone(),
            provider_cli_execution_allowed: false,
            provider_cli_executed: false,
            status: "aborted".to_string(),
            runtime_process_ref: None,
            credential_scan_status: "not_executed".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "not_executed".to_string(),
            reason_codes: breach.code().to_string(),
            input_event_count: 0,
            appended_event_count: 0,
            tool_event_count: 0,
            summary_event_count: 0,
            completed_turn_count: 0,
            observed_token_cost: None,
        };
        Ok(DispatchTurnOutcome {
            run,
            finished,
            usage_after: None,
            ceiling_breach: Some(breach),
        })
    }

    /// Re-derive the dispatch plan id the way `PlanDispatch` does, so the loop
    /// can address the plan it just created without leaking new id vocabulary.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_plan_id_for_turn(
        &self,
        adapter: &str,
        agent_name: &str,
        session_id: &str,
        run_id: &str,
        turn_id: &str,
        goal: &str,
        workspace: &str,
        artifacts: &str,
    ) -> ServerResult<String> {
        let adapter_label = crate::util::adapter_label(adapter)?;
        let goal_hash = crate::util::stable_hash(goal.as_bytes());
        let target_hash = crate::util::stable_hash(
            format!("{agent_name}:{adapter_label}:{session_id}:{run_id}:{turn_id}:{workspace}:{artifacts}")
                .as_bytes(),
        );
        Ok(format!(
            "server-dispatch-plan-{adapter_label}-{goal_hash}-{target_hash}"
        ))
    }

    /// Derive the loop's [`TurnFinished`] for the run just driven.
    ///
    /// When the run actually ingested a batch (it reached the projection path),
    /// classify the SAME normalized batch through the loop's pure derivation so
    /// the loop's outcome cannot drift from what the dispatch run projected. A
    /// run blocked before ingestion produces no terminal/summary/tool refs.
    fn turn_finished_for_run(
        &self,
        turn_id: &str,
        adapter: &str,
        batch_jsonl: &str,
        run: &DispatchRunSummary,
    ) -> ServerResult<TurnFinished> {
        let turn = TurnId::new(turn_id);
        // The dispatch run reports input_event_count > 0 only when it parsed and
        // projected the batch. If it was blocked before ingestion, there is no
        // batch to classify and the outcome carries no refs.
        if run.input_event_count == 0 || batch_jsonl.trim().is_empty() {
            return Ok(FakeBoundaryController::derive_turn_finished(
                &turn,
                &[],
                Default::default(),
            ));
        }
        let adapter_events =
            parse_adapter_events(adapter, batch_jsonl).map_err(ServerError::AdapterFixture)?;
        let replay = capo_controller::AdapterReplayReport {
            input_event_count: run.input_event_count,
            appended_event_count: run.appended_event_count,
            tool_event_count: run.tool_event_count,
            summary_event_count: run.summary_event_count,
            completed_turn_count: run.completed_turn_count,
        };
        Ok(FakeBoundaryController::derive_turn_finished(
            &turn,
            &adapter_events,
            replay,
        ))
    }
}
