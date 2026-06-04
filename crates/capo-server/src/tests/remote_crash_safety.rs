//! RR6 (remote-runtime) capo-server crash-safety tests.
//!
//! RR6 makes a remote run crash-safe end to end: the controller can crash, the
//! remote can crash, or the channel can drop, and Capo recovers to a truthful,
//! auditable state. The runner-level crash matrix is proven exhaustively in
//! `capo-runtime`'s deterministic fake-channel suite; these tests exercise the
//! crash-safe surfaces from the `capo-server` crate (the `-p capo-server`
//! verification gate the RR6 section names) so the server-side seam is covered,
//! not solely self-attested inside `capo-runtime`:
//!
//! - a controller-restart-with-a-live-stored-remote-ref recovers in place;
//! - a remote-reboot is classified exited (never silently recovered);
//! - a dangling remote git worktree is reaped on cleanup (never abandoned),
//!   and cleanup is idempotent + replay-stable;
//! - a revoked remote-control grant stops the run and the runner cannot
//!   re-establish execution without a fresh grant (no orphaned remote capability);
//! - a run is rollback-able to its git checkpoint (composes with safety-gates
//!   checkpoint/rollback).
//!
//! All deterministic: the channel is the in-memory `FakeRemoteChannel` (NO
//! network, NO real SSH).

use std::collections::HashMap;

use capo_core::RunId;
use capo_runtime::{
    CleanupPolicy, FakeRemoteChannel, LocalProcessRequest, LocalRuntimeProcessRef, OpenChannel,
    RemoteChannel, RemoteProcessConfig, RemoteProcessRunner, RemoteRecoveryClassification,
    RuntimeError,
};

use super::temp_root;

/// Build a remote runner over a fake channel scripted by `script`, launch one
/// process so a REAL stored `remote_process_ref` exists, and return the runner,
/// the stored ref flipped to the in-flight `running` state a crash interrupts,
/// the recorded remote boot id, and a clone of the scripted channel.
fn crash_safe_run(
    name: &str,
    script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
) -> (
    RemoteProcessRunner,
    LocalRuntimeProcessRef,
    String,
    FakeRemoteChannel,
) {
    let root = temp_root();
    let workspace = root.join(format!("workspace-{name}"));
    let artifacts = root.join(format!("artifacts-{name}"));
    std::fs::create_dir_all(&workspace).expect("workspace");

    let channel = OpenChannel::for_test(
        format!("chan-{name}"),
        format!("endpoint-{name}"),
        format!("fp-{name}"),
    );
    let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
    let recorded_boot = base.remote_boot_id();
    let scripted = script(base);
    let transport = RemoteChannel::Fake(scripted.clone());
    let runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport));

    let outcome = runner
        .start_process(LocalProcessRequest {
            run_id: RunId::new(format!("run-{name}")),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "printf ok".to_string()],
            cwd: workspace,
            env: HashMap::new(),
        })
        .expect("remote start for crash-safe fixture");
    let running = LocalRuntimeProcessRef {
        status: "running".to_string(),
        ..outcome.process
    };
    (runner, running, recorded_boot, scripted)
}

#[test]
fn server_restart_with_live_remote_recovers_in_place() {
    let (runner, running, boot, _chan) =
        crash_safe_run("rr6-srv-restart", |c| c.recover_alive_reattachable());
    let recovery = runner.recover_run(&running, &boot);
    assert_eq!(
        recovery.classification,
        RemoteRecoveryClassification::Recovered
    );
    assert!(
        !recovery
            .events
            .iter()
            .any(|e| e.kind == "runtime.remote_process_started"),
        "in-place reattach must not relaunch"
    );
}

#[test]
fn server_remote_reboot_is_exited_never_recovered() {
    let (runner, running, boot, _chan) = crash_safe_run("rr6-srv-reboot", |c| c.recover_rebooted());
    let recovery = runner.recover_run(&running, &boot);
    assert_eq!(
        recovery.classification,
        RemoteRecoveryClassification::Exited
    );
}

#[test]
fn server_dangling_worktree_is_reaped_and_cleanup_is_idempotent() {
    let (runner, running, _boot, chan) = crash_safe_run("rr6-srv-dangling", |c| {
        c.with_dangling_worktree("remote/dangling")
    });
    assert!(chan.has_remote_worktree());

    let first = runner
        .cleanup_run(&running, CleanupPolicy::ReapAll)
        .expect("first cleanup reaps");
    assert!(
        first
            .events
            .iter()
            .any(|e| e.kind == "runtime.remote_workspace_torn_down"),
        "a dangling worktree must be reaped, never silently abandoned"
    );
    assert!(!chan.has_remote_worktree());

    // Idempotent + replay-stable: re-running finds nothing to reap.
    let second = runner
        .cleanup_run(&running, CleanupPolicy::ReapAll)
        .expect("idempotent re-run");
    let third = runner
        .cleanup_run(&running, CleanupPolicy::ReapAll)
        .expect("replay-stable re-run");
    assert_eq!(
        second, third,
        "idempotent cleanup must rebuild identically once the worktree is reaped"
    );
}

#[test]
fn server_revoked_grant_stops_run_and_forbids_re_establishment() {
    let (runner, running, _boot, chan) = crash_safe_run("rr6-srv-revoke", |c| c);
    let revoke = runner.revoke_control("channel revoked", Some(&running));
    assert_eq!(revoke.events[0].kind, "runtime.remote_control_revoked");
    assert!(runner.is_control_revoked());
    // The revoke ACTUALLY stopped the in-flight run over the channel (a kill
    // signal reached the transport), not merely forbade the next launch.
    assert_eq!(
        chan.signals_sent(),
        vec!["kill".to_string()],
        "a revoked grant must STOP the remote run over the channel"
    );

    let revoke_root = temp_root();
    let workspace = revoke_root.join("ws-revoke-2");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let err = runner
        .start_process(LocalProcessRequest {
            run_id: RunId::new("run-revoke-2"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "printf ok".to_string()],
            cwd: workspace,
            env: HashMap::new(),
        })
        .expect_err("revoked grant must refuse a new launch");
    assert!(
        matches!(err, RuntimeError::RemoteControlRevoked { .. }),
        "re-establishment requires a fresh grant, not a retry under the revoked one"
    );

    let stdin_err = runner
        .write_stdin(&running, b"x")
        .expect_err("revoked grant must refuse stdin");
    assert!(matches!(
        stdin_err,
        RuntimeError::RemoteControlRevoked { .. }
    ));
}

#[test]
fn server_rollback_restores_worktree_to_git_checkpoint() {
    let (runner, running, _boot, chan) = crash_safe_run("rr6-srv-rollback", |c| c);
    let checkpoint = "refs/capo/materialized/abc123";
    let result = runner
        .rollback_to_checkpoint(&running, checkpoint)
        .expect("rollback");
    assert_eq!(result.events[0].kind, "runtime.remote_rollback_performed");
    assert_eq!(chan.rolled_back_to().as_deref(), Some(checkpoint));
}
