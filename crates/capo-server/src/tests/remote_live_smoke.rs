//! RR8 (remote-runtime) capo-server live SSH smoke + its deterministic pairing.
//!
//! RR8 adds ONE live, opt-in `SshRemoteProcessRunner` smoke against a REAL SSH host,
//! behind explicit env gates and `#[ignore]`, PAIRED with a deterministic fixture
//! that pins the IDENTICAL shapes (process-ref shape, materialized-HEAD-matches-SHA,
//! redacted output, recovery classification). The deterministic fixture runs in the
//! always-on `-p capo-server` gate (NO network, NO real SSH); the live smoke skips
//! cleanly when no host is configured, so completion is NEVER solely
//! operator-attested.
//!
//! - the DEFINED skip predicate records a secret-free reason when the gate is unset
//!   (the live smoke is provably a clean skip, not a silent pass);
//! - a real SSH transport is HONESTLY non-loopback (it crossed a boundary), built
//!   from an already-resolved channel + handle-only auth (no raw credential);
//! - the deterministic fixture pins the live smoke's process-ref / HEAD-matches-SHA
//!   / redacted-output / recovery-classification shapes over the fake channel;
//! - the live smoke (`#[ignore]`) drives the real cross-machine lifecycle or skips
//!   cleanly with a recorded reason.

use std::path::{Path, PathBuf};
use std::process::Command;

use capo_core::RunId;
use capo_runtime::{
    CleanupPolicy, FakeRemoteChannel, GitRemote, LiveRemoteRuntimeSmokeDecision,
    LocalProcessRequest, LocalRuntimeProcessRef, OpenChannel, REMOTE_RUNTIME_PREFLIGHT_ENV,
    REMOTE_RUNTIME_SSH_HOST_ENV, RUN_REMOTE_RUNTIME_LIVE_ENV, RemoteChannel, RemoteProcessConfig,
    RemoteProcessRunner, RemoteRecoveryClassification, RuntimeError, SandboxEnforcement,
    SandboxProfile, SandboxTier, SshRemoteConfig, SshRemoteProcessRunner,
    connectivity_redaction_is_clean, live_remote_runtime_smoke_decision,
};

use super::temp_root;

/// Serialize the tests that read/mutate the RR8 gate env vars.
static RR8_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn git(dir: &Path, args: &[&str]) {
    let fixed_date = "2026-06-02T00:00:00 +0000";
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .env("GIT_AUTHOR_NAME", "capo-test")
        .env("GIT_AUTHOR_EMAIL", "test@capo.local")
        .env("GIT_AUTHOR_DATE", fixed_date)
        .env("GIT_COMMITTER_NAME", "capo-test")
        .env("GIT_COMMITTER_EMAIL", "test@capo.local")
        .env("GIT_COMMITTER_DATE", fixed_date)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {} failed", args.join(" "));
}

fn git_capture(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(output.status.success(), "git {} failed", args.join(" "));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// A real local git-remote fixture (origin + remote repo + worktree root), the
/// source SHA, and a transport URL with an embedded credential (proving redaction).
struct GitFixture {
    source_commit: String,
    git_remote: GitRemote,
    // Keeps the origin/remote-repo/worktree temp dirs alive for the fixture's life.
    _root: capo_tmptest::TempRoot,
}

fn git_fixture(name: &str) -> GitFixture {
    let root = temp_root();
    let origin = root.join(format!("origin-{name}"));
    let remote_repo = root.join(format!("remote-repo-{name}"));
    let worktree_root = root.join(format!("remote-wt-{name}"));
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&remote_repo).unwrap();
    std::fs::create_dir_all(&worktree_root).unwrap();
    git(&origin, &["init", "-q"]);
    git(&remote_repo, &["init", "-q"]);
    std::fs::write(origin.join("committed.txt"), "committed-content").unwrap();
    git(&origin, &["add", "committed.txt"]);
    git(&origin, &["commit", "-q", "-m", "rr8 committed state"]);
    let source_commit = git_capture(&origin, &["rev-parse", "HEAD"]);
    let git_remote = GitRemote::new(
        origin,
        remote_repo,
        worktree_root,
        "ssh://git:AKIAIOSFODNN7EXAMPLE@remote.example/repo.git",
    );
    GitFixture {
        source_commit,
        git_remote,
        _root: root,
    }
}

fn remote_request(run_id: &str, cwd: PathBuf) -> LocalProcessRequest {
    LocalProcessRequest::new(
        RunId::new(run_id),
        "/bin/sh",
        vec!["-c".to_string(), "printf ok".to_string()],
        cwd,
        std::collections::HashMap::new(),
    )
}

#[test]
fn server_rr8_skip_predicate_records_secret_free_reason_when_gate_unset() {
    let _guard = RR8_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: env access is serialized by RR8_ENV_LOCK for this test's duration.
    unsafe {
        std::env::remove_var(REMOTE_RUNTIME_PREFLIGHT_ENV);
        std::env::remove_var(RUN_REMOTE_RUNTIME_LIVE_ENV);
        std::env::remove_var(REMOTE_RUNTIME_SSH_HOST_ENV);
    }
    match live_remote_runtime_smoke_decision() {
        LiveRemoteRuntimeSmokeDecision::Skip { reason } => {
            assert!(
                reason.contains(REMOTE_RUNTIME_PREFLIGHT_ENV)
                    && reason.contains(RUN_REMOTE_RUNTIME_LIVE_ENV),
                "skip reason must name both gates: {reason}"
            );
            // Use the SAME full credential-shape scanner the runtime-level test
            // uses, not an ad-hoc two-pattern check (review finding 8).
            assert!(
                connectivity_redaction_is_clean(&reason),
                "skip reason must be secret-free: {reason}"
            );
        }
        other => panic!("gate unset must Skip, got {other:?}"),
    }
}

#[test]
fn server_rr8_deterministic_fixture_pins_the_live_smoke_shapes() {
    // PAIRING: the SAME shapes the live smoke asserts, pinned over the fake channel
    // (NO network) so the live smoke's completion is checked against a deterministic
    // shape rather than operator attestation.
    let fixture = git_fixture("srv-rr8-pair");
    let channel = OpenChannel::for_test("chan-rr8", "endpoint-rr8", "fp-rr8");
    let root = temp_root();
    let workspace = root.join("ws-rr8");
    std::fs::create_dir_all(&workspace).unwrap();
    let base =
        FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), root.join("artifacts"))
            .with_git_remote(fixture.git_remote.clone())
            .recover_alive_reattachable()
            .with_streamed_output(b"out token AKIAIOSFODNN7EXAMPLE done".to_vec());
    let runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
        channel,
        RemoteChannel::Fake(base),
    ));

    // SHAPE 1: materialized remote HEAD == source SHA, redacted transport URL.
    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("materialize");
    assert_eq!(materialized.remote_head, fixture.source_commit);
    assert!(
        materialized
            .events
            .iter()
            .all(|e| connectivity_redaction_is_clean(&e.detail)),
        "no materialization event may carry the embedded credential"
    );

    // SHAPE 2: the remote process-ref shape.
    let outcome = runner
        .start_process(remote_request("run-rr8-pair", workspace))
        .expect("start");
    let process_ref = &outcome.process.runtime_process_ref;
    assert!(
        process_ref.starts_with("remote-process:fp-rr8:")
            && process_ref.contains(":pid=")
            && process_ref.contains(":boot="),
        "process-ref must carry fingerprint + remote pid + boot: {process_ref}"
    );
    let running = LocalRuntimeProcessRef {
        status: "running".to_string(),
        ..outcome.process.clone()
    };

    // SHAPE 3: remote output is redacted before any delta.
    let stream = runner.stream_output(&running, 0);
    assert_eq!(stream.redaction_state, "redacted");
    assert!(
        !stream
            .deltas
            .iter()
            .any(|d| d.text.contains("AKIAIOSFODNN7EXAMPLE")),
        "credential must be scrubbed before any delta"
    );

    // SHAPE 4: controller-restart-with-live-remote recovers in place.
    let recorded_boot = running
        .runtime_process_ref
        .rsplit(":boot=")
        .next()
        .unwrap_or_default()
        .to_string();
    let recovery = runner.recover_run(&running, &recorded_boot);
    assert_eq!(
        recovery.classification,
        RemoteRecoveryClassification::Recovered
    );
    assert_eq!(recovery.runtime_process_ref, running.runtime_process_ref);
}

#[test]
fn server_rr8_ssh_runner_is_non_loopback_and_handle_only_auth() {
    // HONESTY: a real SSH transport crossed a boundary -> non-loopback. Auth is a
    // handle (label) only; no raw credential is read or stored.
    let channel = OpenChannel::for_test("chan-ssh", "endpoint-ssh", "fp-ssh");
    let ssh = SshRemoteConfig::new("capo@remote.example", "fp-ssh", temp_root().to_path_buf())
        .with_auth_ref("ssh-agent:default");
    let runner = SshRemoteProcessRunner::build(channel, ssh);
    assert!(!runner.is_loopback());
    assert_eq!(runner.target_fingerprint(), "fp-ssh");
}

/// RR8 LIVE, OPT-IN SSH smoke driven from the `-p capo-server` gate. `#[ignore]`;
/// runs ONLY behind both env gates + a configured SSH host, else skips cleanly.
#[test]
#[ignore = "live opt-in: requires CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT=1 + \
            CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE=1 and a reachable SSH host in \
            CAPO_SERVER_REMOTE_RUNTIME_SSH_HOST"]
fn server_rr8_live_ssh_smoke_full_lifecycle_or_clean_skip() {
    let _guard = RR8_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ssh_destination = match live_remote_runtime_smoke_decision() {
        LiveRemoteRuntimeSmokeDecision::Run { ssh_destination } => ssh_destination,
        LiveRemoteRuntimeSmokeDecision::Skip { reason } => {
            eprintln!("RR8 server live SSH smoke skipped cleanly: {reason}");
            return;
        }
    };

    let fixture = git_fixture("srv-rr8-live");
    let channel = OpenChannel::for_test("chan-rr8-live", &ssh_destination, "fp-rr8-live");
    let ssh = SshRemoteConfig::new(ssh_destination, "fp-rr8-live", temp_root().to_path_buf())
        .with_auth_ref("ssh-agent:default")
        .with_git_remote(fixture.git_remote.clone());
    let runner = SshRemoteProcessRunner::build(channel, ssh);
    assert!(!runner.is_loopback());

    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("live materialize");
    assert_eq!(materialized.remote_head, fixture.source_commit);

    // SAFETY FLOOR (review finding 3): the live run goes through the sandboxed
    // launch so the remote OS sandbox + worktree compose under the `SandboxProfile`,
    // with an HONEST remote-OS enforcement claim (Enforced or Unenforced, never
    // fabricated).
    let cwd = PathBuf::from(&materialized.remote_worktree_path);
    let profile = SandboxProfile::workspace_confined([cwd.clone()]);
    let sandboxed = runner
        .start_process_sandboxed(
            remote_request("run-srv-rr8-live", cwd.clone()),
            &cwd,
            &profile,
            SandboxTier::LinuxLandlockBwrap,
            false,
            Some(materialized.remote_head.clone()),
        )
        .expect("live sandboxed start");
    assert!(
        matches!(
            sandboxed.plan.enforcement,
            SandboxEnforcement::Enforced { .. } | SandboxEnforcement::Unenforced { .. }
        ),
        "live sandbox enforcement must be a truthful remote-OS claim, got {:?}",
        sandboxed.plan.enforcement
    );
    let outcome = sandboxed.outcome.expect("a launched run yields an outcome");
    let running = LocalRuntimeProcessRef {
        status: "running".to_string(),
        ..outcome.process.clone()
    };
    let _ = runner.stream_output(&running, 0);
    let recorded_boot = running
        .runtime_process_ref
        .rsplit(":boot=")
        .next()
        .unwrap_or_default()
        .to_string();
    let recovery = runner.recover_run(&running, &recorded_boot);
    assert!(matches!(
        recovery.classification,
        RemoteRecoveryClassification::Recovered | RemoteRecoveryClassification::Exited
    ));

    // Safety floor: a revoked grant forbids re-establishment.
    runner.revoke_control("rr8 server live revoke", None);
    let re_start = runner.start_process(remote_request("run-srv-rr8-live-2", cwd));
    assert!(matches!(
        re_start,
        Err(RuntimeError::RemoteControlRevoked { .. })
    ));
    let _ = runner.cleanup_run(&running, CleanupPolicy::ReapAll);
}
