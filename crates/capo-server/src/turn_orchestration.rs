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

use capo_controller::{FakeBoundaryController, RunResourceCeiling, TurnFinished};
use capo_core::TurnId;

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
    /// ceiling. The `ceiling` MUST bound wall-clock (the wall-clock ceiling is
    /// wired to the runtime's `wait_running_with_timeout`); `run_dispatch_turn`
    /// rejects a live turn whose ceiling does not. The live Codex path therefore
    /// never runs without a ceiling.
    LiveProvider {
        capability_profile: String,
        runtime_scope: String,
        credential_scan_policy: String,
        raw_prompt_policy: String,
        raw_output_policy: String,
        tool_wrapper_policy: String,
        live_provider_opt_in: bool,
        live_execution_opt_in: bool,
        mock_runtime_opt_in: bool,
        mock_provider_output_name: Option<String>,
        mock_provider_output_jsonl: Option<String>,
        /// The per-run resource ceiling this live turn runs inside. Its
        /// wall-clock bound drives the runtime timeout; a ceiling without a
        /// wall-clock bound is rejected.
        ceiling: RunResourceCeiling,
    },
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
pub struct DispatchTurnOutcome {
    pub run: DispatchRunSummary,
    pub finished: TurnFinished,
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
            DispatchTurnMode::LiveProvider {
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
            } => {
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
                (run, request.adapter.clone(), mock_jsonl.unwrap_or_default())
            }
        };

        // Emit: annotate the run we just drove with a TurnFinished derived from
        // the SAME batch the dispatch run ingested. This is the loop's outcome
        // over the existing run-completion truth -- not a second one.
        let finished =
            self.turn_finished_for_run(&request.turn_id, &batch_adapter, &batch_jsonl, &run)?;
        Ok(DispatchTurnOutcome { run, finished })
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
