use super::*;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use crate::{DispatchTurnMode, DispatchTurnRequest, LiveProviderTurn};
use capo_controller::{
    CeilingBreach, FakeBoundaryController, RealBoundaryController, RunResourceCeiling,
    RunResourceUsage, TurnFinished, TurnStopReason,
};

const CODEX_FIXTURE: &str = include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl");

/// Register an agent and start a codex session on `server`, returning nothing
/// beyond the side effect (the helper keeps the two parity arms identical).
fn register_and_start(server: &CapoServer, agent: &str, goal: &str, session: &str, run: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: agent.to_string(),
        },
    );
    handle(
        server,
        ServerCommand::StartSession {
            agent_name: agent.to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
}

/// The dispatch-relevant event kinds for a session, in sequence order, excluding
/// the per-request audit envelope (`server.request_handled`) whose id/payload is
/// keyed to the request envelope and so legitimately differs between the direct
/// command path and the loop-driven path. Everything else -- plan, gate,
/// materialization, execution-request, the projected normalized batch, the
/// dispatch execution, run-exit, and replay -- must match exactly.
fn dispatch_event_kinds(root: &std::path::Path, session: &str) -> Vec<String> {
    let state = SqliteStateStore::open(root).expect("state");
    state
        .recent_events_for_session(&SessionId::new(session), 128)
        .expect("session events")
        .into_iter()
        .map(|event| event.kind)
        .filter(|kind| kind != "server.request_handled")
        .collect()
}

#[test]
fn loop_turn_drives_the_same_dispatch_sequence_as_the_direct_command_path() {
    // RTL4: a loop turn produces the SAME dispatch plan/gate/execution event
    // sequence as the direct command path for a scripted run. The loop drives
    // the existing PlanDispatch -> GateDispatch -> RunDispatchLocal primitives;
    // it does not run a second pipeline.
    let goal = "Run Codex fixture through the dispatch substrate";

    // Arm A: the direct command path (PlanDispatch -> GateDispatch -> RunDispatchLocal).
    let direct_root = temp_root();
    let direct = CapoServer::open(ProjectId::new("project-capo"), &direct_root).expect("server");
    register_and_start(&direct, "codex-local", goal, "session-direct", "run-direct");
    let planned = handle(
        &direct,
        ServerCommand::PlanDispatch {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-direct".to_string(),
            run_id: "run-direct".to_string(),
            turn_id: "turn-direct".to_string(),
            deterministic_opt_in: true,
        },
    );
    let ServerResponsePayload::DispatchPlanned(plan) = planned.payload else {
        panic!("expected dispatch planned response");
    };
    handle(
        &direct,
        ServerCommand::GateDispatch {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
        },
    );
    let direct_run = handle(
        &direct,
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
            fixture_jsonl: CODEX_FIXTURE.to_string(),
        },
    );
    let ServerResponsePayload::DispatchRun(direct_run) = direct_run.payload else {
        panic!("expected dispatch run response");
    };

    // Arm B: the loop-driven path (run_dispatch_turn), identical inputs except
    // a distinct session/run id so the two states are independent.
    let loop_root = temp_root();
    let loop_server = CapoServer::open(ProjectId::new("project-capo"), &loop_root).expect("server");
    register_and_start(
        &loop_server,
        "codex-local",
        goal,
        "session-loop",
        "run-loop",
    );
    let outcome = loop_server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-loop".to_string(),
            run_id: "run-loop".to_string(),
            turn_id: "turn-loop".to_string(),
            mode: DispatchTurnMode::DeterministicFixture {
                deterministic_opt_in: true,
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: CODEX_FIXTURE.to_string(),
            },
        })
        .expect("run dispatch turn");

    // The loop drove the exact same dispatch primitives: same run-completion
    // truth (status, counts, provider flags) as the direct command path.
    assert_eq!(outcome.run.status, direct_run.status);
    assert_eq!(outcome.run.status, "exited");
    assert_eq!(
        outcome.run.provider_cli_executed,
        direct_run.provider_cli_executed
    );
    assert_eq!(outcome.run.input_event_count, direct_run.input_event_count);
    assert_eq!(
        outcome.run.appended_event_count,
        direct_run.appended_event_count
    );
    assert_eq!(outcome.run.tool_event_count, direct_run.tool_event_count);
    assert_eq!(
        outcome.run.completed_turn_count,
        direct_run.completed_turn_count
    );

    // The dispatch-relevant event sequence is identical between the two paths.
    let direct_kinds = dispatch_event_kinds(&direct_root, "session-direct");
    let loop_kinds = dispatch_event_kinds(&loop_root, "session-loop");
    assert_eq!(
        loop_kinds, direct_kinds,
        "loop turn must produce the same dispatch event sequence as the direct command path"
    );
    // Sanity: the sequence actually contains the full dispatch substrate.
    for required in [
        "adapter.dispatch_planned",
        "adapter.dispatch_gate_checked",
        "adapter.dispatch_prompt_materialized",
        "adapter.dispatch_execution_requested",
        "adapter.dispatch_executed",
        "run.exited",
        "adapter.dispatch_replayed",
    ] {
        assert!(
            direct_kinds.iter().any(|kind| kind == required),
            "direct path missing {required}"
        );
        assert!(
            loop_kinds.iter().any(|kind| kind == required),
            "loop path missing {required}"
        );
    }

    // Emit: the loop annotated the run it drove with a TurnFinished derived from
    // the SAME normalized batch the dispatch run ingested -- not a second
    // completion model. Codex fixture: completed turn, two summary items, one
    // observed tool (deduped from request + completion).
    let finished = outcome.finished;
    assert_eq!(finished.turn_id.as_str(), "turn-loop");
    assert_eq!(finished.stop_reason, TurnStopReason::Completed);
    assert!(finished.observed_terminal_event());
    assert_eq!(finished.summary_refs.len(), direct_run.summary_event_count);
    assert!(!finished.observed_tool_refs.is_empty());
    assert!(finished.observed_tool_refs.len() <= direct_run.tool_event_count);
}

#[test]
fn real_controller_matches_fake_path_over_a_scripted_adapter_from_the_server_crate() {
    // RTL5: the RealBoundaryController is the production consumer of the RTL3
    // loop and the RTL1 trait. Driven from the server crate (the controller's
    // client) over the SAME scripted adapter as the fake handle, it must
    // produce byte-compatible read models and an identical TurnFinished -- the
    // controller swap is invisible above the boundary, and the two handles
    // coexist. The typed ServerCommand/ServerResponse boundary is untouched:
    // this test exercises the controller methods the server calls, on both
    // handles, with identical inputs.
    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};

    type Projections = (
        capo_state::SessionProjection,
        Vec<capo_state::ToolCallProjection>,
        Vec<capo_state::EvidenceProjection>,
        i64,
        TurnFinished,
    );

    fn capture(
        state: &SqliteStateStore,
        session: &SessionId,
        finished: TurnFinished,
    ) -> Projections {
        (
            state
                .session(session)
                .expect("session")
                .expect("session present"),
            state.tool_calls_for_session(session).expect("tool calls"),
            state.evidence_for_session(session).expect("evidence"),
            state.event_count().expect("event count"),
            finished,
        )
    }

    fn scripted_batch(
        refs_external_session_ref: &str,
    ) -> Vec<capo_adapters::NormalizedAdapterEvent> {
        ScriptedMockTurn::new("turn-rtl5-server-1")
            .message_delta("msg-1", "inspecting state")
            .tool_requested("tool-1", "capo.agent_status")
            .tool_completed("tool-1", "capo.agent_status", "agent is running")
            .message_completed("msg-2", "state inspected")
            .turn_completed("done-1")
            .normalized_events(refs_external_session_ref)
    }

    let turn_id = capo_core::TurnId::new("turn-rtl5-server-1");
    let goal = "Run an RTL5 turn from the server crate";

    // Fake handle.
    let fake_root = temp_root();
    let fake = FakeBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &fake_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl5-server-session")),
    )
    .expect("open fake controller");
    let fake_reg = fake.register_agent("rtl5-server-worker").expect("register");
    let fake_refs = fake.send_task(&fake_reg, goal).expect("send task");
    let fake_batch = scripted_batch(&fake_refs.external_session_ref);
    let fake_finished = fake
        .run_turn(&fake_refs, &turn_id, &fake_batch)
        .expect("run turn");
    let fake_result = capture(fake.state(), &fake_refs.session_id, fake_finished);

    // Real handle: the production consumer of the same loop/trait.
    let real_root = temp_root();
    let real = RealBoundaryController::open_with_adapter(
        ProjectId::new("project-capo"),
        &real_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("rtl5-server-session")),
    )
    .expect("open real controller");
    let real_reg = real.register_agent("rtl5-server-worker").expect("register");
    let real_refs = real.send_task(&real_reg, goal).expect("send task");
    let real_batch = scripted_batch(&real_refs.external_session_ref);
    let real_finished = real
        .run_turn(&real_refs, &turn_id, &real_batch)
        .expect("run turn");
    let real_result = capture(real.state(), &real_refs.session_id, real_finished);

    assert_eq!(real_result.0, fake_result.0, "session projection diverged");
    assert_eq!(
        real_result.1, fake_result.1,
        "tool-call projections diverged"
    );
    assert_eq!(
        real_result.2, fake_result.2,
        "evidence projections diverged"
    );
    assert_eq!(real_result.3, fake_result.3, "event count diverged");
    assert_eq!(real_result.4, fake_result.4, "TurnFinished diverged");

    // Sanity: the scripted turn actually exercised the loop.
    assert_eq!(real_result.4.stop_reason, TurnStopReason::Completed);
    assert!(real_result.4.observed_terminal_event());
    assert!(!real_result.4.observed_tool_refs.is_empty());
}

#[test]
fn loop_turn_does_not_run_provider_without_passing_the_gate() {
    // RTL4 non-goal assertion, enforced by behavior: the loop reuses the
    // existing dispatch gate. Without the deterministic opt-in the gate blocks,
    // and the loop-driven run never executes -- it returns the same
    // blocked_by_preflight outcome the direct path would, with no ingested batch.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        "Blocked without opt-in",
        "session-blocked",
        "run-blocked",
    );
    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Blocked without opt-in".to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-blocked".to_string(),
            run_id: "run-blocked".to_string(),
            turn_id: "turn-blocked".to_string(),
            mode: DispatchTurnMode::DeterministicFixture {
                // No deterministic opt-in: the existing gate must block.
                deterministic_opt_in: false,
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: CODEX_FIXTURE.to_string(),
            },
        })
        .expect("run dispatch turn");

    assert_eq!(outcome.run.status, "blocked_by_preflight");
    assert!(!outcome.run.provider_cli_executed);
    assert_eq!(outcome.run.appended_event_count, 0);
    assert_eq!(outcome.run.input_event_count, 0);
    // No batch was ingested, so the loop's outcome carries no terminal/summary/
    // tool refs -- the gate fully stopped the run before any projection.
    assert!(!outcome.finished.observed_terminal_event());
    assert!(outcome.finished.summary_refs.is_empty());
    assert!(outcome.finished.observed_tool_refs.is_empty());

    // The gate event was still recorded (the loop went THROUGH the gate), but no
    // provider/ingestion ran.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-blocked"), 64)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "adapter.dispatch_gate_checked"),
        "loop must pass through the gate even when blocked"
    );
    assert!(
        !events.iter().any(|event| event.kind == "run.exited"),
        "a gate-blocked loop turn must not complete a run"
    );
}

#[test]
fn loop_turn_drives_the_live_substrate_through_preflight_and_run() {
    // RTL4: the live Codex execution path (PreflightLiveProvider ->
    // RunLiveProviderLocal) is a STEP inside the loop, reusing the existing
    // preflight gate. Phase 1 drives it with a mock provider output so the
    // substrate is testable without a live provider.
    let goal = "Run Codex live provider through the loop";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-live-loop",
        "run-live-loop",
    );

    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-live-loop".to_string(),
            run_id: "run-live-loop".to_string(),
            turn_id: "turn-live-loop".to_string(),
            mode: live_mode_under_ceiling(
                RunResourceCeiling::for_live_provider(
                    8,
                    std::time::Duration::from_secs(1),
                    100_000,
                ),
                RunResourceUsage::default(),
                0,
            ),
        })
        .expect("run dispatch turn");

    // The run went through the live preflight gate and ingested the mock batch.
    assert_eq!(outcome.run.status, "mocked_live_provider_output_ingested");
    assert!(!outcome.run.provider_cli_executed);
    assert!(outcome.run.input_event_count > 0);

    // RTL7: a within-ceiling live turn did not abort, and the per-run usage
    // accumulator advanced so the loop can carry it into the next turn.
    assert!(outcome.ceiling_breach.is_none());
    assert_eq!(
        outcome.usage_after,
        Some(RunResourceUsage {
            turns_taken: 1,
            wall_clock_elapsed: std::time::Duration::ZERO,
            token_cost: 0,
        })
    );

    // The loop emitted a TurnFinished derived from the same ingested batch.
    assert_eq!(outcome.finished.turn_id.as_str(), "turn-live-loop");
    assert_eq!(outcome.finished.stop_reason, TurnStopReason::Completed);
    assert!(outcome.finished.observed_terminal_event());
    assert!(!outcome.finished.observed_tool_refs.is_empty());

    // The live preflight gate was recorded: the loop did not bypass it.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-live-loop"), 64)
        .expect("events");
    assert!(events.iter().any(|event| {
        event.kind == "adapter.dispatch_gate_checked"
            && event
                .payload_json
                .contains("\"preflight_kind\":\"live_provider\"")
    }));
    assert!(events.iter().any(|event| {
        event.kind == "run.exited"
            && event
                .payload_json
                .contains("mock_live_provider_output_ingested_without_provider_cli")
    }));
}

/// Build a LiveProvider mode for the ceiling-enforcement tests: a mock-runtime
/// codex turn that ingests `CODEX_FIXTURE`, parameterized only by the ceiling
/// and the pre-turn usage/estimate the loop carries in.
fn live_mode_under_ceiling(
    ceiling: RunResourceCeiling,
    usage_before: RunResourceUsage,
    turn_token_cost: u64,
) -> DispatchTurnMode {
    DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
        capability_profile: "trusted-local".to_string(),
        runtime_scope: "local_process_loopback".to_string(),
        credential_scan_policy: "metadata_only_no_secret_read".to_string(),
        raw_prompt_policy: "not_rendered".to_string(),
        raw_output_policy: "artifacts_scanned_redacted".to_string(),
        tool_wrapper_policy: "capo_wrapped_required".to_string(),
        live_provider_opt_in: true,
        live_execution_opt_in: false,
        mock_runtime_opt_in: true,
        mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
        mock_provider_output_jsonl: Some(CODEX_FIXTURE.to_string()),
        ceiling,
        usage_before,
        turn_token_cost,
        codex_program_override: None,
        unattended: true,
    }))
}

#[test]
fn live_turn_without_a_wall_clock_bound_is_rejected_before_any_provider_runs() {
    // RTL7: the active-ceiling prerequisite for the live path. A live-provider
    // turn whose ceiling does not bound wall-clock is rejected before preflight
    // or any provider run -- the live Codex path never runs without a ceiling.
    let goal = "Reject a live turn with no wall-clock bound";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-no-clock",
        "run-no-clock",
    );

    for ceiling in [
        RunResourceCeiling::unbounded(),
        RunResourceCeiling::max_turns(4),
    ] {
        let error = server
            .run_dispatch_turn(DispatchTurnRequest {
                agent_name: "codex-local".to_string(),
                adapter: "codex".to_string(),
                goal: goal.to_string(),
                workspace: "/tmp/capo-workspace".to_string(),
                artifacts: "/tmp/capo-artifacts".to_string(),
                session_id: "session-no-clock".to_string(),
                run_id: "run-no-clock".to_string(),
                turn_id: "turn-no-clock".to_string(),
                mode: live_mode_under_ceiling(ceiling, RunResourceUsage::default(), 0),
            })
            .expect_err("a live turn without a wall-clock bound must be rejected");
        let message = format!("{error:?}");
        assert!(
            message.contains("wall-clock bound"),
            "expected the wall-clock-bound rejection, got: {message}"
        );
    }

    // No provider run reached the state: no run.exited and no run.aborted.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-no-clock"), 64)
        .expect("events");
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "run.exited" || event.kind == "run.aborted"),
        "the rejected live turn must not run or abort a provider"
    );
}

#[test]
fn live_turn_over_max_turns_aborts_on_the_loop_path_without_running_the_provider() {
    // RTL7 (live path): the turns ceiling is enforced IN run_dispatch_turn, on
    // the same substrate the live provider runs through. A turn that would push
    // the run over max_turns aborts BEFORE the provider spawns, emits a durable
    // run.aborted event, and flips the run projection to aborted.
    let goal = "Abort a live turn over max_turns";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-live-turns",
        "run-live-turns",
    );

    // max_turns=1 with one turn already taken: the turn about to run is the 2nd,
    // which trips the ceiling.
    let ceiling =
        RunResourceCeiling::for_live_provider(1, std::time::Duration::from_secs(30), 1_000);
    let usage_before = RunResourceUsage {
        turns_taken: 1,
        ..RunResourceUsage::default()
    };
    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-live-turns".to_string(),
            run_id: "run-live-turns".to_string(),
            turn_id: "turn-live-turns-2".to_string(),
            mode: live_mode_under_ceiling(ceiling, usage_before, 0),
        })
        .expect("run dispatch turn");

    assert_eq!(
        outcome.ceiling_breach,
        Some(CeilingBreach::MaxTurns {
            limit: 1,
            observed: 2
        })
    );
    assert_eq!(outcome.run.status, "aborted");
    assert!(!outcome.run.provider_cli_executed);
    assert!(outcome.usage_after.is_none());
    // The over-ceiling turn projected nothing.
    assert!(outcome.finished.summary_refs.is_empty());
    assert!(outcome.finished.observed_tool_refs.is_empty());

    // A durable run.aborted event was recorded keyed to the aborting turn, the
    // provider never ran (no run.exited), and the run projection is aborted.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-live-turns"), 64)
        .expect("events");
    let aborted = events
        .iter()
        .find(|event| event.kind == "run.aborted")
        .expect("run.aborted recorded");
    assert_eq!(aborted.turn_id.as_deref(), Some("turn-live-turns-2"));
    assert!(aborted.payload_json.contains("max_turns_exceeded"));
    assert!(
        !events.iter().any(|event| event.kind == "run.exited"),
        "the provider must not run when the turns ceiling is already tripped"
    );
    assert_eq!(
        state
            .run(&capo_core::RunId::new("run-live-turns"))
            .expect("run")
            .expect("present")
            .status,
        "aborted"
    );
    // Coordinated terminal projection set: the agent is freed and the session is
    // terminal, exactly like every other terminal stop.
    let agent = state
        .agent_by_name("codex-local")
        .expect("agent")
        .expect("present");
    assert_eq!(agent.status, "available");
    assert!(agent.current_session_id.is_none());
    assert_eq!(
        state
            .session(&SessionId::new("session-live-turns"))
            .expect("session")
            .expect("present")
            .status,
        "aborted"
    );
}

#[test]
fn live_turn_over_token_cost_aborts_on_the_loop_path_without_running_the_provider() {
    // RTL7 (live path): the token/cost ceiling is enforced IN run_dispatch_turn.
    // A turn whose pre-turn token estimate pushes the run over max_token_cost
    // aborts before the provider spawns with a token-cost breach.
    let goal = "Abort a live turn over max_token_cost";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-live-tokens",
        "run-live-tokens",
    );

    // max_token_cost=1000 with 900 already spent; this turn estimates 200 ->
    // projected 1100 > 1000.
    let ceiling =
        RunResourceCeiling::for_live_provider(8, std::time::Duration::from_secs(30), 1_000);
    let usage_before = RunResourceUsage {
        token_cost: 900,
        ..RunResourceUsage::default()
    };
    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-live-tokens".to_string(),
            run_id: "run-live-tokens".to_string(),
            turn_id: "turn-live-tokens".to_string(),
            mode: live_mode_under_ceiling(ceiling, usage_before, 200),
        })
        .expect("run dispatch turn");

    assert_eq!(
        outcome.ceiling_breach,
        Some(CeilingBreach::TokenCost {
            limit: 1_000,
            observed: 1_100
        })
    );
    assert_eq!(outcome.run.status, "aborted");
    assert!(!outcome.run.provider_cli_executed);

    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-live-tokens"), 64)
        .expect("events");
    assert!(
        events.iter().any(|event| event.kind == "run.aborted"
            && event.payload_json.contains("max_token_cost_exceeded")),
        "a token/cost breach records a run.aborted with the token reason code"
    );
    assert!(
        !events.iter().any(|event| event.kind == "run.exited"),
        "the provider must not run when the token ceiling is already tripped"
    );
}

/// Write an executable `/bin/sh` stub that stands in for the codex binary on the
/// live SPAWN path. It (a) launches a background descendant which, only AFTER a
/// long sleep, writes `marker_path`, then (b) sleeps `sleep_secs` itself -- far
/// longer than the tiny wall-clock ceiling. So the whole process GROUP outlives
/// the runtime timeout; when the timeout fires it hard-kills the group, the
/// descendant is reaped before it can write, and `marker_path` never appears.
/// That absence is the deterministic proof the process group was killed.
///
/// Uses only POSIX builtins + `/bin/sleep` (no `codex`, no env gates): the
/// runtime spawns with `env_clear()` so the marker path and sleeps are baked in
/// as literals rather than relying on `$PATH`/`$PWD`.
fn wall_clock_timeout_stub(dir: &Path, tag: &str, marker_path: &Path, sleep_secs: u64) -> String {
    let stub = dir.join(format!("codex-timeout-stub-{tag}.sh"));
    let script = format!(
        "#!/bin/sh\n\
         ( /bin/sleep {sleep} ; printf survived > '{marker}' ) &\n\
         /bin/sleep {sleep}\n",
        sleep = sleep_secs,
        marker = marker_path.display(),
    );
    std::fs::write(&stub, &script).expect("write timeout stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

#[test]
fn live_turn_wall_clock_timeout_kills_the_process_group_and_aborts_the_run() {
    // RTL7 (live path, post-spawn): wall-clock is the ONLY ceiling dimension that
    // fires AFTER the provider spawns, and this is its end-to-end test. It drives a
    // REAL local process (a `/bin/sh` codex stub -- NOT Codex, no live-provider env
    // gates) that sleeps far longer than a sub-second wall-clock ceiling through
    // the live dispatch arm (`run_dispatch_turn` -> `RunLiveProviderLocal` ->
    // `live_provider::execute_codex_live_provider` -> `wait_running_with_timeout`).
    //
    // It asserts BOTH halves of the abort: (1) the process GROUP was hard-killed at
    // the deadline -- proved deterministically because a background descendant that
    // would write a marker only after a long sleep never gets to write it; and
    // (2) a durable `run.aborted` event was recorded via `abort_run_for_ceiling`
    // with the WallClock breach, flipping the run/session to `aborted` and freeing
    // the agent. The stub sleeps generously (5s) against a clamped 1s ceiling, so
    // the test is robust to scheduling jitter without depending on wall time.
    let goal = "Run a long live process under a tiny wall-clock ceiling";
    let root = temp_root();
    std::fs::create_dir_all(&root).expect("root");
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-wall-clock",
        "run-wall-clock",
    );

    // The descendant's marker lives OUTSIDE the confined workspace (under `root`)
    // and its path is baked into the stub, so the assertion does not depend on the
    // process's cwd. It must never be written: the group is killed first.
    let descendant_marker = root.join("descendant-survived-the-timeout.txt");
    let stub = wall_clock_timeout_stub(&root, "wall-clock", &descendant_marker, 5);

    // A sub-second wall-clock ceiling: `wall_clock_timeout_seconds` clamps it up to
    // a 1s runtime timeout, which still fires long before the 5s stub finishes.
    let ceiling = RunResourceCeiling::for_live_provider(8, Duration::from_millis(1), 100_000);
    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: workspace_str,
            artifacts: artifacts_str,
            session_id: "session-wall-clock".to_string(),
            run_id: "run-wall-clock".to_string(),
            turn_id: "turn-wall-clock".to_string(),
            mode: DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
                capability_profile: "trusted-local".to_string(),
                runtime_scope: "local_process_loopback".to_string(),
                credential_scan_policy: "metadata_only_no_secret_read".to_string(),
                raw_prompt_policy: "not_rendered".to_string(),
                raw_output_policy: "artifacts_scanned_redacted".to_string(),
                tool_wrapper_policy: "capo_wrapped_required".to_string(),
                live_provider_opt_in: true,
                // Take the SPAWN path (not the mock-ingest path): a real local
                // process is launched and waited on with the runtime timeout.
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                ceiling,
                usage_before: RunResourceUsage::default(),
                turn_token_cost: 0,
                // The deterministic codex stub: a real process, not Codex.
                codex_program_override: Some(stub),
                // Unattended -> the RTL6 gate resolves the read-only DryRun profile,
                // so the spawn needs no checkpoint and touches nothing.
                unattended: true,
            })),
        })
        .expect("run dispatch turn");

    // The loop classified the post-spawn timeout as a wall-clock ceiling breach.
    // `run_dispatch_turn` reports the configured `max_wall_clock` (1ms) as both the
    // limit and the observed elapsed (the process ran at least to the deadline).
    // The runtime timeout itself clamps 1ms up to a 1s wait, which is why the
    // generous 5s stub is reliably caught.
    assert_eq!(
        outcome.ceiling_breach,
        Some(CeilingBreach::WallClock {
            limit: Duration::from_millis(1),
            observed: Duration::from_millis(1),
        }),
        "a timed-out live spawn must abort with a WallClock breach"
    );
    assert_eq!(outcome.run.status, "aborted");
    assert!(!outcome.run.provider_cli_executed);
    // The aborting turn projected nothing.
    assert!(outcome.usage_after.is_none());
    assert!(outcome.finished.summary_refs.is_empty());
    assert!(outcome.finished.observed_tool_refs.is_empty());

    // (1) The process GROUP was killed at the deadline. The descendant that would
    // write the marker only after a long sleep was reaped with the group before it
    // could -- so the marker must be absent. Wait past the descendant's own sleep
    // to make the absence meaningful (had the group survived, it would exist now).
    std::thread::sleep(Duration::from_millis(6_000));
    assert!(
        !descendant_marker.exists(),
        "the timeout must hard-kill the whole process group: a descendant survived \
         the wall-clock kill and wrote {}",
        descendant_marker.display()
    );

    // (2) A durable run.aborted event was recorded via abort_run_for_ceiling with
    // the WallClock reason, keyed to the aborting turn.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-wall-clock"), 128)
        .expect("events");
    let aborted = events
        .iter()
        .find(|event| event.kind == "run.aborted")
        .expect("run.aborted recorded for the wall-clock timeout");
    assert_eq!(aborted.turn_id.as_deref(), Some("turn-wall-clock"));
    assert!(
        aborted.payload_json.contains("max_wall_clock_exceeded"),
        "the run.aborted payload must carry the wall-clock reason code, got: {}",
        aborted.payload_json
    );

    // The runtime DID spawn and wait on a real process before the abort: the
    // in-flight RTL10 start marker was persisted before the wait.
    assert!(
        events.iter().any(|event| event.kind == "run.started"),
        "the live spawn must persist an in-flight run.started marker before waiting"
    );

    // Coordinated terminal projection set: the run and session are aborted and the
    // agent is freed -- the same terminal shape as every other ceiling stop.
    assert_eq!(
        state
            .run(&capo_core::RunId::new("run-wall-clock"))
            .expect("run")
            .expect("present")
            .status,
        "aborted"
    );
    assert_eq!(
        state
            .session(&SessionId::new("session-wall-clock"))
            .expect("session")
            .expect("present")
            .status,
        "aborted"
    );
    let agent = state
        .agent_by_name("codex-local")
        .expect("agent")
        .expect("present");
    assert_eq!(agent.status, "available");
    assert!(agent.current_session_id.is_none());
}
