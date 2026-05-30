//! RTL12 tests: deterministic scripted multi-turn EDIT over the real loop, plus
//! the multi-turn restart/replay.
//!
//! These drive the turn loop end-to-end through its single execution substrate
//! ([`CapoServer::run_dispatch_turn`], the RTL4 reconciliation point): a session
//! opens, then TWO turns each run through the EXISTING dispatch primitives
//! (`PlanDispatch` -> `PreflightLiveProvider` -> `RunLiveProviderLocal`) and each
//! produces a DISTINCT workspace edit. The Codex binary is replaced by a
//! per-turn `/bin/sh` stub that writes a distinct file and emits a distinct
//! workspace-write JSONL fixture, so the whole path is deterministic with no
//! live provider (the gated live smoke is RTL13).
//!
//! Two properties are proven:
//!
//! 1. Distinct per-turn artifacts and projected items: each turn keys its own
//!    stdout/stderr artifacts under `run_id/turns/<turn_id>` (RTL8) and ingests
//!    its own observed `apply_patch` result, so the two turns do not overwrite
//!    each other on disk or in the read model.
//! 2. Restart/replay: the multi-turn thread, per-turn artifacts (the recorded
//!    artifact references), dispatch executions, and run-exit events rebuild
//!    identically after a reopen + `rebuild_projections`.

use super::*;

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use capo_controller::{RunResourceCeiling, RunResourceUsage, TurnStopReason};

use crate::{DispatchTurnMode, DispatchTurnRequest, LiveProviderTurn};

/// Register a codex agent and open a session keyed to one run on `server`.
fn register_and_start(server: &CapoServer, session: &str, run: &str, goal: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
        },
    );
    handle(
        server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
}

/// A self-contained workspace-write JSONL fixture for one turn: the agent claims
/// an edit, applies a patch to `file` (the observed tool result carries the
/// diff), and the turn completes. `tag` makes the thread/item/call ids distinct
/// per turn so the two turns' projected items never collapse via dedup.
fn write_turn_fixture(tag: &str, file: &str) -> String {
    format!(
        concat!(
            "{{\"type\":\"thread.started\",\"thread_id\":\"thread-{tag}\"}}\n",
            "{{\"type\":\"item.completed\",\"thread_id\":\"thread-{tag}\",\"item\":{{\"id\":\"item-{tag}\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"I will create {file}.\"}}]}}}}\n",
            "{{\"type\":\"patch_apply.begin\",\"thread_id\":\"thread-{tag}\",\"call_id\":\"call-{tag}\",\"tool_name\":\"apply_patch\"}}\n",
            "{{\"type\":\"patch_apply.end\",\"thread_id\":\"thread-{tag}\",\"call_id\":\"call-{tag}\",\"tool_name\":\"apply_patch\",\"unified_diff\":\"--- /dev/null\\n+++ b/{file}\\n@@\\n+edit from {tag}\\n\",\"output\":\"Applied patch to {file} (1 file changed, 1 insertion).\",\"exit_code\":0}}\n",
            "{{\"type\":\"turn.completed\",\"thread_id\":\"thread-{tag}\",\"usage\":{{\"input_tokens\":11,\"output_tokens\":7}}}}\n",
        ),
        tag = tag,
        file = file,
    )
}

/// Write an executable `/bin/sh` codex stub that creates `file` in the workspace
/// (`$PWD`) and emits `fixture_jsonl` on stdout, so the live spawn path is fully
/// deterministic with no live provider. Uses only POSIX builtins because the
/// runtime spawns with `env_clear()` (empty `PATH`).
fn write_turn_stub(dir: &std::path::Path, tag: &str, file: &str, fixture_jsonl: &str) -> String {
    let stub = dir.join(format!("codex-stub-{tag}.sh"));
    let fixture_path = dir.join(format!("fixture-{tag}.jsonl"));
    std::fs::write(&fixture_path, fixture_jsonl).expect("write fixture");
    let script = format!(
        "#!/bin/sh\nprintf 'edit from {tag}\\n' > \"$PWD/{file}\"\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < {fixture}\n",
        tag = tag,
        file = file,
        fixture = fixture_path.display(),
    );
    std::fs::write(&stub, &script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

/// Build a live-provider turn mode that spawns the per-turn stub (no mock,
/// no live env gate so the write mode resolves to the read-only `DryRun` default
/// -- the stub still creates its file, so the edit is observable while the path
/// stays a deterministic, unattended-safe dry run). Always runs inside an active
/// wall-clock ceiling (the RTL7 invariant).
fn stub_turn_mode(stub: String) -> DispatchTurnMode {
    DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
        capability_profile: "trusted-local".to_string(),
        runtime_scope: "local_process_loopback".to_string(),
        credential_scan_policy: "metadata_only_no_secret_read".to_string(),
        raw_prompt_policy: "not_rendered".to_string(),
        raw_output_policy: "artifacts_scanned_redacted".to_string(),
        tool_wrapper_policy: "capo_wrapped_required".to_string(),
        live_provider_opt_in: true,
        live_execution_opt_in: true,
        mock_runtime_opt_in: false,
        mock_provider_output_name: None,
        mock_provider_output_jsonl: None,
        ceiling: RunResourceCeiling::for_live_provider(8, Duration::from_secs(10), 100_000),
        usage_before: RunResourceUsage::default(),
        turn_token_cost: 0,
        codex_program_override: Some(stub),
        unattended: true,
    }))
}

/// RTL12: two turns over the real loop, each a distinct workspace edit, with
/// distinct per-turn artifacts and distinct projected items.
#[test]
fn scripted_multi_turn_edit_over_the_real_loop_keeps_distinct_per_turn_artifacts_and_items() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let goal = "Apply two distinct workspace edits across two turns";
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl12-edit", "run-rtl12-edit", goal);

    let turns = [("turn-edit-1", "first.txt"), ("turn-edit-2", "second.txt")];
    let mut outcomes = Vec::new();
    for (turn_id, file) in turns {
        let tag = turn_id;
        let stub = write_turn_stub(&root, tag, file, &write_turn_fixture(tag, file));
        let outcome = server
            .run_dispatch_turn(DispatchTurnRequest {
                agent_name: "codex-local".to_string(),
                adapter: "codex".to_string(),
                goal: goal.to_string(),
                workspace: workspace_str.clone(),
                artifacts: artifacts_str.clone(),
                session_id: "session-rtl12-edit".to_string(),
                run_id: "run-rtl12-edit".to_string(),
                turn_id: turn_id.to_string(),
                mode: stub_turn_mode(stub),
            })
            .expect("run dispatch turn");
        assert_eq!(outcome.finished.turn_id.as_str(), turn_id);
        assert_eq!(outcome.finished.stop_reason, TurnStopReason::Completed);
        assert!(outcome.finished.observed_terminal_event());
        assert_eq!(outcome.run.status, "exited");
        assert!(outcome.run.provider_cli_executed);
        outcomes.push((turn_id, file, outcome));
    }

    // Both distinct edits landed in the confined workspace.
    assert!(
        workspace.join("first.txt").exists(),
        "turn-1 edit must land"
    );
    assert!(
        workspace.join("second.txt").exists(),
        "turn-2 edit must land"
    );

    // Distinct per-turn artifacts on disk: each turn nests its stdout/stderr
    // under `run_id/turns/<turn_id>` (RTL8 keying), so neither overwrites the
    // other.
    let turns_dir = artifacts.join("run-rtl12-edit").join("turns");
    let mut turn_dirs: Vec<String> = std::fs::read_dir(&turns_dir)
        .expect("turns dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    turn_dirs.sort();
    assert_eq!(turn_dirs, vec!["turn-edit-1", "turn-edit-2"]);
    for turn in &turn_dirs {
        let dir = turns_dir.join(turn);
        let stdout = std::fs::read_to_string(dir.join("stdout.txt")).expect("read stdout");
        // Each turn's stdout artifact is its OWN ingested batch, not the other's.
        assert!(
            stdout.contains(&format!("thread-{turn}")),
            "turn {turn} stdout must carry its own thread id"
        );
        assert!(dir.join("stderr.txt").exists());
    }

    // Distinct projected items: two observed `apply_patch` results, one per turn,
    // each anchored to its own content artifact (distinct ids).
    let state = SqliteStateStore::open(&root).expect("state");
    let session = SessionId::new("session-rtl12-edit");
    let apply_patch_observations: Vec<_> = state
        .tool_observations_for_session(&session)
        .expect("observations")
        .into_iter()
        .filter(|observation| observation.tool_name == "apply_patch")
        .collect();
    assert_eq!(
        apply_patch_observations.len(),
        2,
        "each turn must project its own observed apply_patch result"
    );
    let mut observation_artifacts: Vec<String> = apply_patch_observations
        .iter()
        .map(|observation| observation.artifact_id.clone().expect("anchored artifact"))
        .collect();
    observation_artifacts.sort();
    observation_artifacts.dedup();
    assert_eq!(
        observation_artifacts.len(),
        2,
        "the two turns' observed results must be distinct content-anchored items"
    );

    // The per-turn tool calls are keyed to their own turn ids (no overwrite in
    // the read model either).
    let tool_call_turns: std::collections::BTreeSet<String> = state
        .tool_calls_for_session(&session)
        .expect("tool calls")
        .into_iter()
        .filter(|call| call.tool_name == "apply_patch")
        .filter_map(|call| call.turn_id)
        .collect();
    assert_eq!(
        tool_call_turns,
        ["turn-edit-1".to_string(), "turn-edit-2".to_string()]
            .into_iter()
            .collect()
    );

    // Single completion model (RTL8): exactly one dispatch run-exit + execution
    // per ingested turn, and no forked turn-completion kind.
    let counts = dispatch_completion_counts(&root, "session-rtl12-edit");
    assert_eq!(counts.run_exited, 2, "one dispatch run-exit per turn");
    assert_eq!(counts.executions, 2, "one dispatch execution per turn");
    assert_eq!(counts.forked_turn_completion, 0);
}

/// RTL12: the multi-turn thread, per-turn artifacts, dispatch executions, and
/// run-exit events rebuild identically after a restart/replay.
#[test]
fn multi_turn_edit_thread_rebuilds_identically_after_restart_replay() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let goal = "Apply two edits, then replay the multi-turn thread";
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl12-replay", "run-rtl12-replay", goal);

    for (turn_id, file) in [
        ("turn-replay-1", "alpha.txt"),
        ("turn-replay-2", "beta.txt"),
    ] {
        let stub = write_turn_stub(&root, turn_id, file, &write_turn_fixture(turn_id, file));
        server
            .run_dispatch_turn(DispatchTurnRequest {
                agent_name: "codex-local".to_string(),
                adapter: "codex".to_string(),
                goal: goal.to_string(),
                workspace: workspace_str.clone(),
                artifacts: artifacts_str.clone(),
                session_id: "session-rtl12-replay".to_string(),
                run_id: "run-rtl12-replay".to_string(),
                turn_id: turn_id.to_string(),
                mode: stub_turn_mode(stub),
            })
            .expect("run dispatch turn");
    }

    let session = SessionId::new("session-rtl12-replay");
    // The full multi-turn snapshot: observations (with their content artifacts),
    // tool calls (with turn ids), the session projection, and the event count.
    let snapshot = |store: &SqliteStateStore| {
        let observations = store
            .tool_observations_for_session(&session)
            .expect("observations")
            .into_iter()
            .map(|observation| {
                (
                    observation.tool_name,
                    observation.observed_status,
                    observation.artifact_id,
                )
            })
            .collect::<Vec<_>>();
        let tool_calls = store
            .tool_calls_for_session(&session)
            .expect("tool calls")
            .into_iter()
            .map(|call| (call.tool_name, call.status, call.turn_id))
            .collect::<Vec<_>>();
        let session_projection = store.session(&session).expect("session").expect("present");
        let event_count = store.event_count().expect("event count");
        (observations, tool_calls, session_projection, event_count)
    };

    let state = SqliteStateStore::open(&root).expect("state");
    let before = snapshot(&state);
    let counts_before = dispatch_completion_counts(&root, "session-rtl12-replay");

    // Restart: reopen and rebuild projections from the persisted event log.
    let reopened = SqliteStateStore::open(&root).expect("reopen");
    reopened.rebuild_projections().expect("rebuild");
    let after = snapshot(&reopened);
    let counts_after = dispatch_completion_counts(&root, "session-rtl12-replay");

    assert_eq!(
        before, after,
        "the multi-turn thread + per-turn artifacts must rebuild identically"
    );
    assert_eq!(
        counts_before, counts_after,
        "the dispatch executions + run-exits must rebuild identically"
    );
    // Both turns' observed apply_patch results survive the replay, distinct.
    assert_eq!(
        after
            .0
            .iter()
            .filter(|(name, _, _)| name == "apply_patch")
            .count(),
        2
    );
    assert_eq!(counts_after.run_exited, 2);
    assert_eq!(counts_after.executions, 2);
}

#[derive(Debug, Eq, PartialEq)]
struct DispatchCompletionCounts {
    run_exited: usize,
    executions: usize,
    forked_turn_completion: usize,
}

/// Count the dispatch run-completion family for a session: dispatch run-exits
/// (`run.exited` carrying a dispatch payload), dispatch executions, and any
/// hypothetical forked turn-completion kind (which must stay zero -- the loop
/// annotates the dispatch run-exit, it does not fork a second completion model).
fn dispatch_completion_counts(root: &std::path::Path, session: &str) -> DispatchCompletionCounts {
    let state = SqliteStateStore::open(root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new(session), 512)
        .expect("session events");
    let mut counts = DispatchCompletionCounts {
        run_exited: 0,
        executions: 0,
        forked_turn_completion: 0,
    };
    for event in events {
        match event.kind.as_str() {
            "run.exited" if event.payload_json.contains("dispatch_plan_id") => {
                counts.run_exited += 1
            }
            "adapter.dispatch_executed" => counts.executions += 1,
            "turn.finished" | "turn.completed" | "loop.turn_finished" => {
                counts.forked_turn_completion += 1
            }
            _ => {}
        }
    }
    counts
}
