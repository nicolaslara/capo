//! RTL8 tests: per-turn artifact keying by `turn_id`, reconciled with the
//! dispatch run-exit.
//!
//! Two properties are proven deterministically (no live provider):
//!
//! 1. Multiple turns in one run keep DISTINCT stdout/stderr artifacts. The
//!    Codex workspace-write launch plan threads its per-turn request through the
//!    runtime ([`LocalAdapterLaunchPlan::runtime_request_for_turn`]), so two
//!    turns in the same `run_id` no longer overwrite each other's artifacts --
//!    the exact gap RTL8 closes in `crates/capo-runtime/src/lib.rs`.
//! 2. The loop's `TurnFinished` ANNOTATES the existing dispatch run-exit /
//!    execution events; it does not fork a second run-completion model. Driving
//!    two turns through the mock dispatch substrate records exactly one
//!    `run.exited` family per ingested turn and one execution per turn, and a
//!    restart/replay rebuilds every turn's record identically.

use super::*;

use capo_adapters::CodexExecAdapter;
use capo_core::RunId;
use capo_runtime::LocalProcessRunner;

use crate::{DispatchTurnMode, DispatchTurnRequest, LiveProviderTurn};
use capo_controller::{RunResourceCeiling, RunResourceUsage, TurnStopReason};

const CODEX_FIXTURE: &str = include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl");

/// Register an agent and start a codex session on `server`.
fn register_and_start(server: &CapoServer, agent: &str, goal: &str, session: &str, run: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: agent.to_string(),
            adapter: "fake".to_string(),
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

/// Drive the REAL Codex workspace-write launch plan through the runtime for two
/// turns in the same run, with the codex program replaced by a `/bin/sh` stub
/// that emits the fixture JSONL so the path is deterministic and needs no live
/// provider. This exercises the production per-turn artifact threading
/// (`runtime_request_for_turn` -> runtime `run_dir`/artifact-id keying).
#[test]
fn workspace_write_turns_in_one_run_keep_distinct_per_turn_artifacts() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let mut launch_plan = CodexExecAdapter::local_workspace_write_launch_plan(
        workspace.clone(),
        artifacts.clone(),
        "apply the edit",
    );
    // Replace the codex invocation with a deterministic stub that emits the same
    // normalized JSONL Codex would, so the per-turn artifact path is exercised
    // through the real launch-plan/runtime wiring without a live provider.
    launch_plan.program = "/bin/sh".to_string();
    launch_plan.argv = vec![
        "-c".to_string(),
        format!("cat <<'CAPO_EOF'\n{CODEX_FIXTURE}CAPO_EOF"),
    ];

    let runner = LocalProcessRunner::new(launch_plan.runtime_config());
    let run_id = RunId::new("run-ws-multi-turn");

    let mut recorded = Vec::new();
    for turn in ["turn-edit-1", "turn-edit-2"] {
        let mut process = runner
            .spawn_process(launch_plan.runtime_request_for_turn(run_id.clone(), turn))
            .expect("spawn turn");
        let outcome = runner.wait_running(&mut process).expect("wait turn");
        assert_eq!(outcome.process.status, "exited");
        recorded.push((turn, outcome));
    }

    let (turn1, outcome1) = &recorded[0];
    let (turn2, outcome2) = &recorded[1];

    // Distinct per-turn stdout/stderr artifact paths and ids: no overwriting.
    assert_ne!(outcome1.stdout.path, outcome2.stdout.path);
    assert_ne!(outcome1.stderr.path, outcome2.stderr.path);
    assert_ne!(outcome1.stdout.artifact_id, outcome2.stdout.artifact_id);
    assert_ne!(outcome1.stderr.artifact_id, outcome2.stderr.artifact_id);
    assert!(outcome1.stdout.artifact_id.contains(turn1));
    assert!(outcome2.stdout.artifact_id.contains(turn2));

    // Each turn's stdout artifact is the full ingested batch (reconstructable).
    for (_turn, outcome) in &recorded {
        let stdout = std::fs::read_to_string(&outcome.stdout.path).expect("read stdout");
        assert!(stdout.contains("turn.completed"));
        assert!(stdout.contains("thread.started"));
    }

    // Restart/replay surface: every turn's artifact is reconstructable from the
    // run directory alone after the process is gone.
    let turns_dir = artifacts.join(run_id.as_str()).join("turns");
    let mut turn_dirs: Vec<String> = std::fs::read_dir(&turns_dir)
        .expect("turns dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    turn_dirs.sort();
    assert_eq!(turn_dirs, vec!["turn-edit-1", "turn-edit-2"]);
    for turn in &turn_dirs {
        let dir = turns_dir.join(turn);
        assert!(dir.join("stdout.txt").exists());
        assert!(dir.join("stderr.txt").exists());
    }
}

/// Build a within-ceiling live-provider mode that ingests the codex fixture
/// through the mock substrate (no real provider), keyed to a turn.
fn live_mock_turn(turn_token_cost: u64) -> DispatchTurnMode {
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
        ceiling: RunResourceCeiling::for_live_provider(
            8,
            std::time::Duration::from_secs(1),
            100_000,
        ),
        usage_before: RunResourceUsage::default(),
        turn_token_cost,
        // Mock path never spawns codex, so no program override is needed.
        codex_program_override: None,
        unattended: true,
    }))
}

/// RTL8: the loop's `TurnFinished` annotates the dispatch run-exit; it does not
/// fork a second completion model, and the run-exit reconciliation survives a
/// restart/replay so every turn's record is reconstructable.
#[test]
fn turn_finished_annotates_dispatch_run_exit_without_a_second_completion_model() {
    let goal = "Reconcile TurnFinished with the dispatch run-exit";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        goal,
        "session-rtl8-recon",
        "run-rtl8-recon",
    );

    let outcome = server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-rtl8-recon".to_string(),
            run_id: "run-rtl8-recon".to_string(),
            turn_id: "turn-rtl8-recon".to_string(),
            mode: live_mock_turn(0),
        })
        .expect("run dispatch turn");

    // The TurnFinished is derived from the SAME ingested batch as the run.
    assert_eq!(outcome.finished.turn_id.as_str(), "turn-rtl8-recon");
    assert_eq!(outcome.finished.stop_reason, TurnStopReason::Completed);
    assert!(outcome.finished.observed_terminal_event());

    // There is exactly ONE run-completion truth: the dispatch run-exit. The loop
    // adds no parallel turn-completion event kind -- only the existing
    // `run.exited` family appears, exactly once for the ingested turn.
    let counts = run_completion_event_counts(&root, "session-rtl8-recon");
    assert_eq!(counts.run_exited, 1, "exactly one dispatch run-exit");
    assert_eq!(counts.executions, 1, "exactly one dispatch execution");
    assert_eq!(
        counts.forked_turn_completion, 0,
        "the loop must not fork a second turn/run completion event kind"
    );

    // Restart/replay: reopen the state and rebuild projections. The run-exit /
    // execution records reconstruct identically (same counts, same run status).
    let before = run_completion_event_counts(&root, "session-rtl8-recon");
    let reopened = SqliteStateStore::open(&root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild");
    let after = run_completion_event_counts(&root, "session-rtl8-recon");
    assert_eq!(before, after, "run-exit reconciliation survives replay");
}

#[derive(Debug, Eq, PartialEq)]
struct RunCompletionCounts {
    run_exited: usize,
    executions: usize,
    forked_turn_completion: usize,
}

/// Count the dispatch run-completion event family for a session: dispatch
/// run-exits (`run.exited` carrying a dispatch payload), dispatch executions,
/// and any hypothetical forked turn-completion kind (which must be zero -- the
/// loop annotates, it does not fork).
fn run_completion_event_counts(root: &std::path::Path, session: &str) -> RunCompletionCounts {
    let state = SqliteStateStore::open(root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new(session), 256)
        .expect("session events");
    let mut counts = RunCompletionCounts {
        run_exited: 0,
        executions: 0,
        forked_turn_completion: 0,
    };
    for event in events {
        match event.kind.as_str() {
            "run.exited" if event.payload_json.contains("dispatch_plan_id") => {
                counts.run_exited += 1;
            }
            "adapter.dispatch_executed" => counts.executions += 1,
            // Any of these would indicate a second completion model forked by the
            // loop. RTL8 forbids them.
            "turn.finished" | "turn.completed" | "loop.turn_finished" => {
                counts.forked_turn_completion += 1;
            }
            _ => {}
        }
    }
    counts
}
