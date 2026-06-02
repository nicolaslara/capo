//! RR7 (remote-runtime) capo-server git-materialization tests.
//!
//! RR3 makes a remote run's workspace GIT-BASED: the run's target commit is
//! pushed/fetched and `git worktree add`-ed ON the remote, and remote-produced
//! commits are mapped BACK by git. The full materialization invariant matrix is
//! proven exhaustively against REAL `git` in `capo-runtime`'s deterministic
//! fake-channel suite. These tests exercise the materialization seam from the
//! `capo-server` crate (the `-p capo-server` verification gate the RR7 section
//! names) so the server-side seam is covered, not solely self-attested inside
//! `capo-runtime`:
//!
//! - materialization is content-addressed: the remote worktree HEAD pins to the
//!   source commit SHA exactly, and the committed file is present at that SHA;
//! - the injected git-sync decision holds: uncommitted/untracked scratch is NEVER
//!   materialized on the remote, and the non-sync is an EXPLICIT recorded fact;
//! - the git transport URL passes the credential scan BEFORE it is recorded, so an
//!   embedded secret never lands on a materialization event;
//! - a remote-produced commit maps BACK by git into a named local ref;
//! - materialization is replay-stable: a re-materialization of the same source SHA
//!   rebuilds identical projected state.
//!
//! All deterministic: the channel is the in-memory `FakeRemoteChannel` backed by a
//! REAL local git-remote model (`GitRemote`) over local bare-ish repos (NO network,
//! NO real SSH).

use std::path::{Path, PathBuf};
use std::process::Command;

use capo_runtime::{
    FakeRemoteChannel, GitRemote, OpenChannel, RemoteChannel, RemoteProcessConfig,
    RemoteProcessRunner, RuntimeError,
};

use super::temp_root;

/// Run a git subcommand against `dir` with a deterministic, system-config-free
/// identity so the fixture is reproducible and replay-stable.
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

/// A real local "origin" repo with one COMMITTED file plus a DIRTY untracked file
/// (proving uncommitted scratch never travels), the source commit SHA, and the
/// empty remote-repo + worktree-root dirs the channel materializes into. NO
/// network — every path is local + deterministic.
struct GitRemoteFixture {
    origin: PathBuf,
    source_commit: String,
    dirty_filename: String,
    git_remote: GitRemote,
}

fn git_remote_fixture(name: &str) -> GitRemoteFixture {
    let root = temp_root();
    let origin = root.join(format!("origin-{name}"));
    let remote_repo = root.join(format!("remote-repo-{name}"));
    let worktree_root = root.join(format!("remote-wt-{name}"));
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&remote_repo).unwrap();
    std::fs::create_dir_all(&worktree_root).unwrap();

    git(&origin, &["init", "-q"]);
    git(&remote_repo, &["init", "-q"]);

    // One COMMITTED file — the only thing a git-based sync carries.
    std::fs::write(origin.join("committed.txt"), "committed-content").unwrap();
    git(&origin, &["add", "committed.txt"]);
    git(&origin, &["commit", "-q", "-m", "rr7 committed state"]);
    let source_commit = git_capture(&origin, &["rev-parse", "HEAD"]);

    // A DIRTY untracked file that MUST NOT travel (uncommitted scratch is not
    // auto-synced — the injected git-sync decision).
    let dirty_filename = "uncommitted-scratch.txt".to_string();
    std::fs::write(origin.join(&dirty_filename), "secret local scratch").unwrap();

    let git_remote = GitRemote::new(
        origin.clone(),
        remote_repo,
        worktree_root,
        // A transport URL with an EMBEDDED credential — it MUST be redacted before
        // it lands on the materialization event.
        "ssh://git:AKIAIOSFODNN7EXAMPLE@remote.example/repo.git",
    );
    GitRemoteFixture {
        origin,
        source_commit,
        dirty_filename,
        git_remote,
    }
}

/// Build a remote runner over a fake channel backed by a real git-remote model.
/// NO network, NO real SSH.
fn runner_with_git_remote(name: &str, git_remote: GitRemote) -> RemoteProcessRunner {
    let channel = OpenChannel::for_test(
        format!("chan-{name}"),
        format!("endpoint-{name}"),
        format!("fp-{name}"),
    );
    let workspace = temp_root().join(format!("ws-{name}"));
    let artifacts = temp_root().join(format!("art-{name}"));
    std::fs::create_dir_all(&workspace).unwrap();
    let base = FakeRemoteChannel::from_open_channel(&channel, workspace, artifacts)
        .with_git_remote(git_remote);
    RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
        channel,
        RemoteChannel::Fake(base),
    ))
}

#[test]
fn server_materialization_pins_head_to_the_source_sha() {
    // INVARIANT: materialization is content-addressed — the remote worktree HEAD
    // matches the source commit SHA exactly, and the committed file is present.
    let fixture = git_remote_fixture("srv-pins-head");
    let runner = runner_with_git_remote("srv-pins-head", fixture.git_remote.clone());

    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("materialize");

    assert_eq!(materialized.remote_head, fixture.source_commit);
    assert_eq!(materialized.source_commit, fixture.source_commit);
    let head_committed = std::fs::read_to_string(
        Path::new(&materialized.remote_worktree_path).join("committed.txt"),
    )
    .expect("committed file present on remote worktree");
    assert_eq!(head_committed, "committed-content");

    let event = &materialized.events[0];
    assert_eq!(event.kind, "runtime.remote_workspace_materialized");
    assert!(event.detail.contains(&fixture.source_commit));
}

#[test]
fn server_uncommitted_scratch_is_never_materialized_on_the_remote() {
    // INVARIANT (injected git-sync decision): uncommitted/untracked scratch does
    // NOT travel. A dirty local file is ABSENT on the materialized remote worktree,
    // and the non-sync is an EXPLICIT recorded fact, not a silent gap.
    let fixture = git_remote_fixture("srv-no-scratch");
    let runner = runner_with_git_remote("srv-no-scratch", fixture.git_remote.clone());

    assert!(fixture.origin.join(&fixture.dirty_filename).exists());

    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("materialize");

    let scratch_on_remote =
        Path::new(&materialized.remote_worktree_path).join(&fixture.dirty_filename);
    assert!(
        !scratch_on_remote.exists(),
        "uncommitted scratch must NOT be materialized on the remote worktree"
    );
    assert!(
        materialized
            .events
            .iter()
            .any(|e| e.detail.contains("uncommitted_scratch_synced=false")),
        "the non-sync of uncommitted scratch must be an explicit recorded fact"
    );
}

#[test]
fn server_materialization_event_redacts_an_embedded_credential_in_the_transport_url() {
    // INVARIANT (safety boundary): the git transport URL passes the credential scan
    // BEFORE it is recorded — an embedded secret is scrubbed, so no credential ever
    // lands on a remote-runtime event.
    let fixture = git_remote_fixture("srv-redact");
    let runner = runner_with_git_remote("srv-redact", fixture.git_remote.clone());

    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("materialize");

    assert!(
        !materialized.transport_url.contains("AKIAIOSFODNN7EXAMPLE"),
        "an embedded credential must never reach the recorded transport URL"
    );
    assert!(
        !materialized
            .events
            .iter()
            .any(|e| e.detail.contains("AKIAIOSFODNN7EXAMPLE")),
        "no materialization event may carry the raw credential"
    );
    assert_eq!(materialized.transport_url_redaction, "redacted");
}

#[test]
fn server_remote_commit_maps_back_by_git_into_a_named_local_ref() {
    // INVARIANT: results are mapped BACK by git — the remote worktree tip is fetched
    // into a named local ref (the DP8 reconcile/merge-back point).
    let fixture = git_remote_fixture("srv-fetch-back");
    let runner = runner_with_git_remote("srv-fetch-back", fixture.git_remote.clone());

    let materialized = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("materialize");
    let worktree = PathBuf::from(&materialized.remote_worktree_path);

    let local_ref = "refs/capo/remote/rr7-srv-fetch-back";
    let reconciled = runner
        .reconcile_workspace(&worktree, local_ref)
        .expect("reconcile");

    assert_eq!(reconciled.remote_commit, fixture.source_commit);
    assert_eq!(reconciled.local_ref, local_ref);
    assert_eq!(
        reconciled.events[0].kind,
        "runtime.remote_workspace_reconciled"
    );
    // The named local ref actually resolves to the remote commit in the origin.
    let resolved = git_capture(&fixture.origin, &["rev-parse", local_ref]);
    assert_eq!(resolved, fixture.source_commit);
}

#[test]
fn server_materialization_of_an_unknown_commit_is_a_typed_failure() {
    // INVARIANT: a failed git step is the TYPED error, never a silent fall-through
    // to the wrong dir.
    let fixture = git_remote_fixture("srv-typed-fail");
    let runner = runner_with_git_remote("srv-typed-fail", fixture.git_remote.clone());

    let err = runner
        .materialize_workspace("0000000000000000000000000000000000000000")
        .expect_err("an unknown commit must fail typed");
    assert!(
        matches!(err, RuntimeError::RemoteMaterializeFailed { .. }),
        "a failed materialization must be RemoteMaterializeFailed, got {err:?}"
    );
}

#[test]
fn server_materialization_is_replay_stable_across_repeated_runs() {
    // INVARIANT: re-materializing the SAME source SHA rebuilds identical projected
    // state (idempotent + replay-stable).
    let fixture = git_remote_fixture("srv-replay");
    let runner = runner_with_git_remote("srv-replay", fixture.git_remote.clone());

    let first = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("first materialize");
    let second = runner
        .materialize_workspace(&fixture.source_commit)
        .expect("replay materialize");
    assert_eq!(
        first, second,
        "re-materializing the same source SHA must rebuild identical state"
    );
}
