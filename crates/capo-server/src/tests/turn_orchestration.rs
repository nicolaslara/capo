use super::*;

use crate::{DispatchTurnMode, DispatchTurnRequest};
use capo_controller::TurnStopReason;

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
            mode: DispatchTurnMode::LiveProvider {
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
                timeout_seconds: 1,
            },
        })
        .expect("run dispatch turn");

    // The run went through the live preflight gate and ingested the mock batch.
    assert_eq!(outcome.run.status, "mocked_live_provider_output_ingested");
    assert!(!outcome.run.provider_cli_executed);
    assert!(outcome.run.input_event_count > 0);

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
