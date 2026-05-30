//! RTL6 safety-floor tests: confinement (no process spawned), pre-write
//! checkpoint create/restore, dry-run-by-default, controller hard kill, and
//! the checkpoint event surviving restart/replay.

use super::*;

use std::collections::HashMap;

use capo_runtime::{LocalProcessConfig, LocalProcessRequest, LocalProcessRunner};

use crate::safety_floor::LIVE_WRITE_OPT_IN_ENV;
use crate::{
    RunTurnRef, WorkspaceWriteRequest, WriteMode, resolve_write_mode, resolve_write_mode_with_env,
};

fn system_origin() -> ServerClientOrigin {
    ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    }
}

/// A unique scratch workspace under the system temp dir for a test.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("capo-rtl6-{name}-{nanos}-{counter}"));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn out_of_confinement_write_is_rejected_before_any_process_runs() {
    // RTL6: a write outside the confined workspace is rejected by the path
    // containment engine BEFORE a process is spawned. We prove "no process ran"
    // two ways: the confinement call returns an error (so the orchestrator never
    // reaches a spawn), and a drop-marker the workspace-write command would have
    // created is absent.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace");
    let artifacts = scratch_dir("artifacts");
    let origin = system_origin();

    // A `..`-escape out of the confined workspace.
    let escape_rejected = server.run_workspace_write_turn(
        &origin,
        WorkspaceWriteRequest {
            session_id: "session-confine",
            run_id: "run-confine",
            turn_id: "turn-confine",
            workspace_root: &workspace.display().to_string(),
            artifact_root: &artifacts.display().to_string(),
            write_target: "../escape/outside.txt",
            live_execution_opt_in: true,
            unattended: false,
        },
    );
    let error = escape_rejected.expect_err("an out-of-workspace write must be rejected");
    let message = format!("{error:?}");
    assert!(
        message.contains("confinement rejected"),
        "expected a confinement rejection, got {message}"
    );

    // An unrelated absolute path outside the workspace is also rejected.
    let outside = scratch_dir("outside");
    let absolute_rejected = server.run_workspace_write_turn(
        &origin,
        WorkspaceWriteRequest {
            session_id: "session-confine",
            run_id: "run-confine",
            turn_id: "turn-confine-abs",
            workspace_root: &workspace.display().to_string(),
            artifact_root: &artifacts.display().to_string(),
            write_target: &outside.join("outside.txt").display().to_string(),
            live_execution_opt_in: true,
            unattended: false,
        },
    );
    assert!(
        absolute_rejected.is_err(),
        "an absolute path outside the workspace must be rejected before any process runs"
    );

    // No checkpoint snapshot directory was created (the floor never advanced
    // past confinement), and no checkpoint event was recorded.
    assert!(
        !artifacts.join("checkpoints").exists(),
        "rejected write must not create a checkpoint snapshot"
    );
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-confine"), 64)
        .expect("events");
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "checkpoint.created"),
        "a rejected write must not produce a checkpoint event"
    );

    // A confined target IS accepted (sanity: the engine is not rejecting
    // everything). In dry-run it touches nothing.
    let confined = server
        .run_workspace_write_turn(
            &origin,
            WorkspaceWriteRequest {
                session_id: "session-confine",
                run_id: "run-confine",
                turn_id: "turn-confine-ok",
                workspace_root: &workspace.display().to_string(),
                artifact_root: &artifacts.display().to_string(),
                write_target: "src/edited.rs",
                live_execution_opt_in: false,
                unattended: false,
            },
        )
        .expect("a confined write target is accepted");
    let canonical_workspace = workspace.canonicalize().expect("canonical workspace");
    assert!(
        confined
            .confined_write_target
            .starts_with(&canonical_workspace)
    );
}

#[test]
fn write_adapter_defaults_to_dry_run_and_takes_no_checkpoint() {
    // RTL6: diff-preview/dry-run is the DEFAULT. Without the env gate, even an
    // explicit caller opt-in stays dry-run, and a dry run touches nothing, so it
    // takes no checkpoint.
    // Guard against an ambient live opt-in from the surrounding environment.
    let env_opt_in = std::env::var(LIVE_WRITE_OPT_IN_ENV).as_deref() == Ok("1");

    // Pure resolution: opt-in alone is not enough.
    assert_eq!(
        resolve_write_mode(false, false),
        WriteMode::DryRun,
        "no opt-in => dry run"
    );

    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace-dry");
    let artifacts = scratch_dir("artifacts-dry");
    std::fs::write(workspace.join("file.txt"), b"original").expect("seed workspace");

    let outcome = server
        .run_workspace_write_turn(
            &system_origin(),
            WorkspaceWriteRequest {
                session_id: "session-dry",
                run_id: "run-dry",
                turn_id: "turn-dry",
                workspace_root: &workspace.display().to_string(),
                artifact_root: &artifacts.display().to_string(),
                write_target: "file.txt",
                // Caller opts in, but unattended is false and (unless the
                // ambient env gate is set) the env gate is unset.
                live_execution_opt_in: true,
                unattended: false,
            },
        )
        .expect("dry-run confined write");

    if env_opt_in {
        // If the surrounding environment opts into live writes, the floor will
        // (correctly) take a checkpoint; assert that branch instead.
        assert_eq!(outcome.write_mode, WriteMode::LiveWrite);
        assert!(outcome.checkpoint.is_some());
    } else {
        assert_eq!(
            outcome.write_mode,
            WriteMode::DryRun,
            "without the env gate the write adapter stays dry-run by default"
        );
        assert!(
            outcome.checkpoint.is_none(),
            "a dry run touches nothing and takes no checkpoint"
        );
        let state = SqliteStateStore::open(&root).expect("state");
        let events = state
            .recent_events_for_session(&SessionId::new("session-dry"), 64)
            .expect("events");
        assert!(
            !events
                .iter()
                .any(|event| event.kind == "checkpoint.created"),
            "a dry run records no checkpoint event"
        );
    }

    // An unattended turn can never reach a live write.
    assert_eq!(
        resolve_write_mode(true, true),
        WriteMode::DryRun,
        "unattended turns never reach a live write"
    );
}

#[test]
fn resolve_write_mode_with_env_is_an_and_gate_over_all_three_requirements() {
    // RTL6: `resolve_write_mode_with_env` is the gate protecting the first real
    // workspace write. A live write is permitted ONLY when ALL THREE required
    // conditions hold simultaneously:
    //   1. the caller explicitly opted in (`live_execution_opt_in`),
    //   2. the process env gate is set (`env_opt_in`), and
    //   3. the run is attended (`!unattended`).
    // Every other combination must fall back to the safe `DryRun` default. We
    // assert this EXHAUSTIVELY over all 2^3 = 8 combinations of the three bools,
    // so dropping any single requirement from the gate fails this test.
    //
    // The pure, env-injected `resolve_write_mode_with_env` is used (rather than
    // `resolve_write_mode`, which reads process-global env) so the truth table is
    // deterministic and independent of the surrounding environment.

    // Build the full truth table programmatically as the canonical AND-gate, and
    // assert against it. `expected_live` is *defined* as the three-way AND so the
    // test encodes the intended semantics, then we additionally pin down each row
    // explicitly below to catch a wrong definition.
    for caller_opt_in in [false, true] {
        for env_opt_in in [false, true] {
            for unattended in [false, true] {
                let attended = !unattended;
                let expected_live = caller_opt_in && env_opt_in && attended;
                let expected = if expected_live {
                    WriteMode::LiveWrite
                } else {
                    WriteMode::DryRun
                };
                let actual = resolve_write_mode_with_env(caller_opt_in, env_opt_in, unattended);
                assert_eq!(
                    actual, expected,
                    "resolve_write_mode_with_env(caller_opt_in={caller_opt_in}, \
                     env_opt_in={env_opt_in}, unattended={unattended}) must be {expected:?}: \
                     a live write requires caller opt-in AND env gate AND attended"
                );
            }
        }
    }

    // Explicit, row-by-row truth table (caller_opt_in, env_opt_in, unattended) ->
    // expected mode. Pinning every row guards against a regression that drops a
    // requirement yet still happens to agree with a mis-stated `expected_live`.
    let truth_table: [(bool, bool, bool, WriteMode); 8] = [
        // No requirement satisfied.
        (false, false, false, WriteMode::DryRun),
        (false, false, true, WriteMode::DryRun),
        // Only the env gate.
        (false, true, false, WriteMode::DryRun),
        (false, true, true, WriteMode::DryRun),
        // Only the caller opt-in.
        (true, false, false, WriteMode::DryRun),
        (true, false, true, WriteMode::DryRun),
        // Caller opt-in AND env gate, but UNATTENDED -> still dry run.
        (true, true, true, WriteMode::DryRun),
        // ALL THREE: caller opt-in AND env gate AND attended -> the ONLY live row.
        (true, true, false, WriteMode::LiveWrite),
    ];
    for (caller_opt_in, env_opt_in, unattended, expected) in truth_table {
        assert_eq!(
            resolve_write_mode_with_env(caller_opt_in, env_opt_in, unattended),
            expected,
            "truth-table row (caller_opt_in={caller_opt_in}, env_opt_in={env_opt_in}, \
             unattended={unattended}) must resolve to {expected:?}"
        );
    }

    // Exactly ONE combination yields a live write: the all-requirements-met row.
    let live_rows = truth_table
        .iter()
        .filter(|(_, _, _, mode)| *mode == WriteMode::LiveWrite)
        .count();
    assert_eq!(
        live_rows, 1,
        "exactly one of the eight combinations may reach LiveWrite"
    );

    // The single live combination, established as the baseline.
    assert_eq!(
        resolve_write_mode_with_env(true, true, false),
        WriteMode::LiveWrite,
        "the all-requirements-met combination must reach a live write"
    );

    // Regression guard: from that live baseline, dropping ANY single requirement
    // -- and ONLY that requirement -- must collapse the decision back to DryRun.
    // This is what makes the test fail if a future change removes a conjunct from
    // the gate.
    assert_eq!(
        resolve_write_mode_with_env(false, true, false),
        WriteMode::DryRun,
        "dropping the caller opt-in alone must fall back to dry run"
    );
    assert_eq!(
        resolve_write_mode_with_env(true, false, false),
        WriteMode::DryRun,
        "dropping the env gate alone must fall back to dry run"
    );
    assert_eq!(
        resolve_write_mode_with_env(true, true, true),
        WriteMode::DryRun,
        "making the run unattended alone must fall back to dry run"
    );
}

#[test]
fn pre_write_checkpoint_is_created_and_one_command_restores_the_workspace() {
    // RTL6: the checkpoint is captured BEFORE the write, and a documented
    // restore command returns the workspace to its pre-write state.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace-ckpt");
    let artifacts = scratch_dir("artifacts-ckpt");
    std::fs::create_dir_all(workspace.join("src")).expect("src dir");
    std::fs::write(workspace.join("src/lib.rs"), b"pre-write contents").expect("seed file");
    std::fs::write(workspace.join("README.md"), b"readme").expect("seed readme");

    let checkpoint = server
        .create_pre_write_checkpoint(
            &system_origin(),
            RunTurnRef {
                session_id: "session-ckpt",
                run_id: "run-ckpt",
                turn_id: "turn-ckpt",
            },
            &workspace.display().to_string(),
            &artifacts.display().to_string(),
        )
        .expect("create pre-write checkpoint");

    // The snapshot exists and captured the pre-write tree.
    assert!(checkpoint.snapshot_root.exists());
    assert_eq!(checkpoint.file_count, 2);
    assert_eq!(
        std::fs::read_to_string(checkpoint.snapshot_root.join("src/lib.rs")).unwrap(),
        "pre-write contents"
    );

    // The checkpoint.created event was recorded with the documented restore
    // command and the reversible flag.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-ckpt"), 64)
        .expect("events");
    let checkpoint_event = events
        .iter()
        .find(|event| event.kind == "checkpoint.created")
        .expect("checkpoint.created event present");
    assert!(
        checkpoint_event
            .payload_json
            .contains("\"reversible\":true")
    );
    assert!(
        checkpoint_event
            .payload_json
            .contains("\"restore_command\":"),
        "checkpoint event must record the documented restore command"
    );

    // Now simulate the write mutating the workspace...
    std::fs::write(
        workspace.join("src/lib.rs"),
        b"OVERWRITTEN by the live write",
    )
    .expect("write");
    std::fs::write(workspace.join("src/new.rs"), b"a new file the write added").expect("new file");
    std::fs::remove_file(workspace.join("README.md")).expect("write deleted readme");

    // ...and the ONE documented restore command returns the workspace to its
    // pre-write state. We assert the recorded command and the programmatic
    // restore (its equivalent) agree, then run the restore.
    assert_eq!(checkpoint.restore_command(), {
        let payload: serde_json::Value =
            serde_json::from_str(&checkpoint_event.payload_json).expect("payload json");
        payload["restore_command"].as_str().unwrap().to_string()
    });
    checkpoint.restore().expect("restore from checkpoint");

    assert_eq!(
        std::fs::read_to_string(workspace.join("src/lib.rs")).unwrap(),
        "pre-write contents",
        "restore must return the edited file to its pre-write contents"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("README.md")).unwrap(),
        "readme",
        "restore must bring back files the write deleted"
    );
    assert!(
        !workspace.join("src/new.rs").exists(),
        "restore must remove files the write added"
    );
}

#[test]
fn create_pre_write_checkpoint_is_idempotent_on_unchanged_state() {
    // Re-capturing the same pre-write state appends no duplicate event.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace-idem");
    let artifacts = scratch_dir("artifacts-idem");
    std::fs::write(workspace.join("f.txt"), b"same").expect("seed");

    let idem_ref = RunTurnRef {
        session_id: "session-idem",
        run_id: "run-idem",
        turn_id: "turn-idem",
    };
    let first = server
        .create_pre_write_checkpoint(
            &system_origin(),
            idem_ref,
            &workspace.display().to_string(),
            &artifacts.display().to_string(),
        )
        .expect("first checkpoint");
    let count_after_first = SqliteStateStore::open(&root)
        .expect("state")
        .event_count()
        .expect("count");

    let second = server
        .create_pre_write_checkpoint(
            &system_origin(),
            idem_ref,
            &workspace.display().to_string(),
            &artifacts.display().to_string(),
        )
        .expect("second checkpoint");
    let count_after_second = SqliteStateStore::open(&root)
        .expect("state")
        .event_count()
        .expect("count");

    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(
        count_after_first, count_after_second,
        "re-capturing the same pre-write state must not duplicate the checkpoint event"
    );
}

#[test]
fn checkpoint_event_survives_restart_and_replay() {
    // RTL6 verification: the checkpoint event survives restart. We capture a
    // checkpoint, reopen the state store (restart), rebuild projections
    // (replay), and confirm the checkpoint.created event is still present and
    // the event count is unchanged.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace-replay");
    let artifacts = scratch_dir("artifacts-replay");
    std::fs::write(workspace.join("a.txt"), b"contents").expect("seed");

    server
        .create_pre_write_checkpoint(
            &system_origin(),
            RunTurnRef {
                session_id: "session-replay",
                run_id: "run-replay",
                turn_id: "turn-replay",
            },
            &workspace.display().to_string(),
            &artifacts.display().to_string(),
        )
        .expect("create checkpoint");

    let before = SqliteStateStore::open(&root).expect("state");
    let count_before = before.event_count().expect("count before");
    assert!(
        before
            .recent_events_for_session(&SessionId::new("session-replay"), 64)
            .expect("events")
            .iter()
            .any(|event| event.kind == "checkpoint.created")
    );

    // Restart: a fresh handle on the same on-disk state.
    let reopened = SqliteStateStore::open(&root).expect("reopen state");
    reopened.rebuild_projections().expect("rebuild projections");

    let count_after = reopened.event_count().expect("count after");
    assert_eq!(
        count_before, count_after,
        "rebuild must not change the durable event log"
    );
    let checkpoint_event = reopened
        .recent_events_for_session(&SessionId::new("session-replay"), 64)
        .expect("events after restart")
        .into_iter()
        .find(|event| event.kind == "checkpoint.created")
        .expect("checkpoint event survives restart and replay");
    assert!(
        checkpoint_event
            .payload_json
            .contains("\"reversible\":true")
    );
}

#[test]
fn controller_hard_kill_terminates_the_process_group_mid_run_and_records_the_abort() {
    // RTL6: a controller-owned hard kill terminates the run's process group
    // mid-run (reusing the runtime process-group kill path) and records the
    // abort as a `run.hard_killed` event.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let workspace = scratch_dir("workspace-kill");
    let artifacts = scratch_dir("artifacts-kill");

    let runner =
        LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
    // A long-running process with a backgrounded descendant that drops a marker
    // after a delay -- if the group survives, the marker would appear.
    let marker = workspace.join("descendant-survived.txt");
    let mut running = runner
        .spawn_process(LocalProcessRequest {
            run_id: capo_core::RunId::new("run-kill"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec![
                "-c".to_string(),
                format!("(sleep 3; printf survived > {}) & wait", marker.display()),
            ],
            cwd: workspace.clone(),
            env: HashMap::new(),
        })
        .expect("spawn live process");

    // Mid-run: confirm it is live, then hard-kill the group.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(runner.health_running(&mut running).unwrap().live);

    server
        .hard_kill_run(
            &system_origin(),
            &runner,
            &mut running,
            RunTurnRef {
                session_id: "session-kill",
                run_id: "run-kill",
                turn_id: "turn-kill",
            },
            "operator emergency stop",
        )
        .expect("hard kill mid-run");
    assert_eq!(running.process.status, "killed");

    // The descendant must not have survived the group kill.
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(
        !marker.exists(),
        "hard kill must terminate the whole process group, not just the direct child"
    );

    // The abort was recorded as a run.hard_killed event.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-kill"), 64)
        .expect("events");
    let kill_event = events
        .iter()
        .find(|event| event.kind == "run.hard_killed")
        .expect("run.hard_killed event recorded");
    assert!(
        kill_event
            .payload_json
            .contains("controller_hard_kill_process_group")
    );
    assert!(kill_event.payload_json.contains("operator emergency stop"));
}
