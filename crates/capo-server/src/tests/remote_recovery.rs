//! RR2 (remote-runtime) capo-server recovery tests.
//!
//! RR2's runner exposes `recover_run`; the controller's restart loop is what
//! drives it for each stored remote `RuntimeProcessRef`. The runner-level
//! classification matrix is proven exhaustively in `capo-runtime`'s deterministic
//! fake-channel suite. These tests exercise the recovery path from the
//! `capo-server` crate (the `-p capo-server` verification gate the RR2 section
//! names) so the server-side recovery seam is covered, not solely self-attested
//! inside `capo-runtime`:
//!
//! - a controller-restart-with-a-live-stored-remote-ref recovers in place and
//!   produces the append-first recovery events the controller would persist;
//! - the channel-unreachable case is held `recovery_pending` (never forced), and
//!   the SAME stored ref recovers when the channel returns;
//! - recovery is replay-stable across repeated restarts (idempotent rebuild).
//!
//! All deterministic: the channel is the in-memory `FakeRemoteChannel` (NO
//! network, NO real SSH).

use std::collections::HashMap;

use capo_core::RunId;
use capo_runtime::{
    FakeRemoteChannel, LocalProcessRequest, LocalRuntimeProcessRef, OpenChannel, RemoteChannel,
    RemoteProcessConfig, RemoteProcessRunner, RemoteRecoveryClassification,
};

use super::temp_root;

/// Build a remote runner over a fake channel scripted with a recovery outcome,
/// launch one process so a REAL stored `remote_process_ref` (with the recorded
/// `:pid=...:boot=...` tail) exists, and return the runner, the stored ref flipped
/// to the in-flight `running` state a controller crash interrupts, plus the boot
/// id recorded at launch. The channel identity is reused so the recovery probe
/// re-resolves the SAME channel.
fn survived_remote_run(
    name: &str,
    script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
) -> (RemoteProcessRunner, LocalRuntimeProcessRef, String) {
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
    let transport = RemoteChannel::Fake(script(base));
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
        .expect("remote start for recovery fixture");
    let running = LocalRuntimeProcessRef {
        status: "running".to_string(),
        ..outcome.process
    };
    (runner, running, recorded_boot)
}

#[test]
fn server_restart_with_live_remote_run_recovers_in_place_with_events() {
    let (runner, running, recorded_boot) =
        survived_remote_run("srv-rec-alive", |c| c.recover_alive_reattachable());

    // The controller's restart loop calls recover_run for each stored remote ref.
    let recovery = runner.recover_run(&running, &recorded_boot);
    assert_eq!(
        recovery.classification,
        RemoteRecoveryClassification::Recovered
    );

    // The append-first recovery events the controller persists: a
    // recovery_attempted fact then the single terminal classification.
    let kinds: Vec<&str> = recovery.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "runtime.remote_recovery_attempted",
            "runtime.remote_run_recovered"
        ],
        "recovery must append the attempted fact then exactly one terminal event"
    );
    // In-place reattach, not a relaunch.
    assert_eq!(recovery.runtime_process_ref, running.runtime_process_ref);
    assert!(
        !recovery
            .events
            .iter()
            .any(|e| e.kind == "runtime.remote_process_started"),
        "an in-place reattach must NOT relaunch the remote process"
    );
}

#[test]
fn server_recovery_channel_unreachable_is_pending_then_recovers_on_return() {
    // First restart: the channel cannot be re-resolved -> the run is held
    // recovery_pending (NOT forced to recovered or exited).
    let (pending_runner, running, recorded_boot) =
        survived_remote_run("srv-rec-pending", |c| c.recover_channel_unreachable());
    let pending = pending_runner.recover_run(&running, &recorded_boot);
    assert_eq!(
        pending.classification,
        RemoteRecoveryClassification::RecoveryPending
    );
    assert_eq!(
        pending.events.last().unwrap().kind,
        "runtime.remote_recovery_pending"
    );

    // Channel returns on a later restart: re-resolve the SAME channel identity to a
    // now-reachable one and re-run recovery against the SAME stored ref.
    let chan = OpenChannel::for_test(
        "chan-srv-rec-pending",
        "endpoint-srv-rec-pending",
        "fp-srv-rec-pending",
    );
    let root = temp_root();
    let workspace = root.join("workspace-return");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let reachable =
        FakeRemoteChannel::from_open_channel(&chan, workspace, root.join("artifacts-return"))
            .recover_alive_reattachable();
    let return_runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
        chan,
        RemoteChannel::Fake(reachable),
    ));
    let recovered = return_runner.recover_run(&running, &recorded_boot);
    assert_eq!(
        recovered.classification,
        RemoteRecoveryClassification::Recovered,
        "the same pending stored ref must recover when the channel returns"
    );
    assert_eq!(recovered.runtime_process_ref, running.runtime_process_ref);
}

#[test]
fn server_remote_recovery_is_replay_stable_across_repeated_restarts() {
    let (runner, running, recorded_boot) =
        survived_remote_run("srv-rec-replay", |c| c.recover_alive_reattachable());
    let first = runner.recover_run(&running, &recorded_boot);
    let second = runner.recover_run(&running, &recorded_boot);
    assert_eq!(
        first, second,
        "remote recovery must rebuild identically across repeated restarts"
    );
}
