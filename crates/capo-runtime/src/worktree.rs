//! DP8: git worktree isolation per session/goal, a SWAPPABLE option behind the
//! [`RuntimeRunner`] boundary.
//!
//! Today a session's workspace-write run executes directly in the operator's
//! live working tree, confined only by [`LocalProcessRunner`]'s path-prefix check
//! (the `cwd` must sit under a `workspace_roots` entry) and the `safety-gates`
//! single-writer lease + pre-write checkpoint. DP8 adds a real `git worktree` as
//! the confinement unit: instead of running in the operator's checkout, a
//! session runs in a DEDICATED `git worktree` carved off the same repository, on
//! its own branch, so the `real-turn-loop` workspace confinement is scoped to
//! that worktree root and two concurrent sessions never share a working tree.
//!
//! This is the runtime-boundary primitive only. It exposes the worktree
//! LIFECYCLE -- create-on-session-start ([`WorktreeManager::create`]),
//! reconcile/merge-back point ([`WorktreeManager::reconcile`]), and teardown
//! ([`WorktreeManager::teardown`]) -- as plain typed outcomes the controller
//! records as events (`worktree.created`/`worktree.reconciled`/
//! `worktree.torn_down`) so a worktree is reconstructable/inspectable after a
//! restart and never silently abandoned. The controller (capo-controller)
//! composes it with the `safety-gates` single-writer lease and pre-write
//! checkpoint, and binds it to a `Goal`/`GoalAttempt` for the per-goal slice.
//!
//! Why a worktree and not a clone: a `git worktree` shares the origin
//! repository's object store while giving the session an independent working
//! directory + index + `HEAD`. That keeps creation cheap and the merge-back a
//! normal in-repo ref operation, while still isolating the session's working
//! files from the operator's live tree. The cardinal rule mirrors the sandbox
//! tier ([`crate::OsSandbox`]): the worktree confines writes to its own root, and
//! the manager never claims isolation it did not actually create -- a failed
//! `git worktree add` is a typed [`WorktreeError`], never a silent fall-through
//! to the operator's tree.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::normalize_path;

/// The runtime-boundary variant label for worktree isolation, mirroring
/// [`crate::SandboxTier::variant`] so the runtime boundary records WHICH
/// confinement option a run executed under.
pub const WORKTREE_ISOLATION_VARIANT: &str = "worktree-git-isolated";
/// The variant label for the legacy "run in the operator's live tree" option.
pub const WORKTREE_ISOLATION_NONE_VARIANT: &str = "worktree-none";

/// A runtime-lifecycle event the worktree manager emits, the same shape as
/// [`crate::RuntimeEvent`], so the controller records it on the event log with
/// the loop's scope provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeEvent {
    pub kind: String,
    pub status: String,
    pub detail: String,
}

/// A typed reason a worktree create/reconcile/teardown could not proceed,
/// surfaced as an outcome (never a panic, never a silent fall-through to the
/// operator's live tree).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeError {
    pub message: String,
}

impl WorktreeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.message
    }
}

/// The request to carve an isolated worktree for a session/goal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeRequest {
    /// The originating repository root (the operator's live working tree) the
    /// worktree is carved from. Must be inside a git repository.
    pub repo_root: PathBuf,
    /// The directory under which the dedicated worktree root is created. The
    /// worktree lives at `<worktrees_root>/<key>`; keeping it OUTSIDE `repo_root`
    /// means the session's files never overlay the operator's tree.
    pub worktrees_root: PathBuf,
    /// A stable key for this session/goal worktree (e.g. the session id, or a
    /// goal-bound key for the per-goal slice). Two distinct keys get two distinct
    /// worktrees with no cross-contamination; the SAME key reattaches to the same
    /// worktree.
    pub key: String,
}

/// The dedicated worktree a session writes in -- the `real-turn-loop` workspace
/// confinement scope for that session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsolatedWorktree {
    /// The worktree root the session's workspace-write run executes in. This is
    /// the path the runner's `workspace_roots` confinement should be scoped to.
    pub worktree_path: PathBuf,
    /// The branch checked out in the worktree (an isolated per-key ref).
    pub branch: String,
    /// The originating repository root.
    pub repo_root: PathBuf,
}

/// The outcome of a worktree lifecycle step: the worktree plus the events to
/// record. Created/reconciled steps carry the worktree; teardown clears the
/// on-disk worktree but still reports the (now-removed) path for the event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeOutcome {
    pub worktree: IsolatedWorktree,
    pub events: Vec<WorktreeEvent>,
    /// True when the create reattached to a worktree that already existed on disk
    /// for this key (idempotent create -- a continued goal reattaches rather than
    /// erroring or spawning a second worktree).
    pub reattached: bool,
}

/// A swappable git-worktree isolation option behind the runtime boundary.
///
/// Construct with a [`WorktreeRequest`]'s repo/worktrees roots; the manager owns
/// the `git worktree add/remove` mechanics and keeps the operator's live tree
/// untouched. It does NOT itself run the session command -- the controller scopes
/// a [`LocalProcessRunner`] to [`IsolatedWorktree::worktree_path`] and runs there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeManager;

impl WorktreeManager {
    pub fn new() -> Self {
        Self
    }

    /// The runtime-boundary variant label this manager runs under.
    pub fn binding_variant(&self) -> &'static str {
        WORKTREE_ISOLATION_VARIANT
    }

    /// Derive the dedicated worktree path for a request's key (a collision-free
    /// per-key segment under `worktrees_root`).
    fn worktree_path_for(request: &WorktreeRequest) -> PathBuf {
        request.worktrees_root.join(key_segment(&request.key))
    }

    /// The isolated branch name for a key. A per-key ref so two concurrent
    /// sessions never check out the same branch in two worktrees (git forbids
    /// that, and it would also defeat isolation).
    fn branch_for(key: &str) -> String {
        format!("capo/worktree/{}", key_segment(key))
    }

    /// CREATE-ON-SESSION-START: carve (or reattach to) the session's isolated
    /// worktree.
    ///
    /// If a worktree for this key already exists on disk (a continued goal, or a
    /// restart that rebuilt the projection), it is REATTACHED idempotently rather
    /// than recreated -- the worktree is never silently abandoned and a continued
    /// goal reuses its existing tree. Otherwise `git worktree add` creates a fresh
    /// worktree on an isolated per-key branch. A failure is a typed error.
    pub fn create(&self, request: &WorktreeRequest) -> Result<WorktreeOutcome, WorktreeError> {
        let repo_root = normalize_path(&request.repo_root)
            .map_err(|error| WorktreeError::new(format!("repo root not resolvable: {error:?}")))?;
        if !is_git_repo(&repo_root) {
            return Err(WorktreeError::new(format!(
                "worktree repo root is not a git repository: {}",
                repo_root.display()
            )));
        }
        let worktree_path = Self::worktree_path_for(request);
        let branch = Self::branch_for(&request.key);
        let isolated = IsolatedWorktree {
            worktree_path: worktree_path.clone(),
            branch: branch.clone(),
            repo_root: repo_root.clone(),
        };

        // Reattach: a worktree directory that git already tracks is reused as-is.
        if worktree_path.join(".git").exists() {
            return Ok(WorktreeOutcome {
                events: vec![WorktreeEvent {
                    kind: "worktree.created".to_string(),
                    status: "reattached".to_string(),
                    detail: format!(
                        "reattached worktree {} on branch {branch}",
                        worktree_path.display()
                    ),
                }],
                worktree: isolated,
                reattached: true,
            });
        }

        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                WorktreeError::new(format!("failed to create worktrees root: {error}"))
            })?;
        }

        // `git worktree add -b <branch> <path>` creates a new working tree on a
        // fresh branch off the current HEAD. The new branch keeps the session's
        // commits off the operator's checked-out branch.
        git_in_repo(
            &repo_root,
            &[
                "worktree",
                "add",
                "-b",
                &branch,
                &worktree_path.to_string_lossy(),
            ],
        )?;

        Ok(WorktreeOutcome {
            events: vec![WorktreeEvent {
                kind: "worktree.created".to_string(),
                status: "created".to_string(),
                detail: format!(
                    "created worktree {} on branch {branch}",
                    worktree_path.display()
                ),
            }],
            worktree: isolated,
            reattached: false,
        })
    }

    /// RECONCILE / MERGE-BACK POINT: record a reconcile point for the worktree.
    ///
    /// This commits any outstanding work in the worktree onto its isolated branch
    /// (allowing an empty commit so a no-op session still produces a stable
    /// merge-back ref) and returns the tip the operator can later merge. It does
    /// NOT mutate the operator's checked-out branch -- merging into the
    /// operator's tree is a deliberate operator action, not an automatic one. The
    /// reconcile point is recorded as an event so the merge-back is auditable.
    pub fn reconcile(&self, worktree: &IsolatedWorktree) -> Result<WorktreeOutcome, WorktreeError> {
        if !worktree.worktree_path.join(".git").exists() {
            return Err(WorktreeError::new(format!(
                "cannot reconcile a worktree that is not present: {}",
                worktree.worktree_path.display()
            )));
        }
        git_in_worktree(&worktree.worktree_path, &["add", "-A"])?;
        git_in_worktree(
            &worktree.worktree_path,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "--no-verify",
                "--quiet",
                "-m",
                "capo worktree reconcile point",
            ],
        )?;
        let tip = git_capture_in_worktree(&worktree.worktree_path, &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        Ok(WorktreeOutcome {
            events: vec![WorktreeEvent {
                kind: "worktree.reconciled".to_string(),
                status: "reconciled".to_string(),
                detail: format!(
                    "reconcile point {tip} on branch {} for worktree {}",
                    worktree.branch,
                    worktree.worktree_path.display()
                ),
            }],
            worktree: worktree.clone(),
            reattached: false,
        })
    }

    /// TEARDOWN: remove the worktree from disk and prune git's bookkeeping.
    ///
    /// Uses `git worktree remove --force` so a worktree with uncommitted changes
    /// is still cleaned up (the controller takes a reconcile point first when it
    /// wants to keep the work). The teardown is recorded as an event; a worktree
    /// that is already gone tears down idempotently rather than erroring, so a
    /// retried teardown after a partial crash still completes.
    pub fn teardown(&self, worktree: &IsolatedWorktree) -> Result<WorktreeOutcome, WorktreeError> {
        let repo_root = &worktree.repo_root;
        if worktree.worktree_path.exists() {
            git_in_repo(
                repo_root,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    &worktree.worktree_path.to_string_lossy(),
                ],
            )?;
        }
        // Prune stale administrative entries (idempotent; safe if nothing to do).
        let _ = git_in_repo(repo_root, &["worktree", "prune"]);
        Ok(WorktreeOutcome {
            events: vec![WorktreeEvent {
                kind: "worktree.torn_down".to_string(),
                status: "torn_down".to_string(),
                detail: format!(
                    "removed worktree {} (branch {})",
                    worktree.worktree_path.display(),
                    worktree.branch
                ),
            }],
            worktree: worktree.clone(),
            reattached: false,
        })
    }
}

impl Default for WorktreeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `path` is inside a git working tree (a real repo, so `git worktree
/// add` can run). Uses `git rev-parse --is-inside-work-tree`.
fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a git subcommand against the origin repository (used for `worktree add`
/// / `worktree remove` / `worktree prune`), with the deterministic capo identity
/// so the operation never depends on a global git identity.
fn git_in_repo(repo_root: &Path, args: &[&str]) -> Result<(), WorktreeError> {
    let output = worktree_git(repo_root, args).output().map_err(|error| {
        WorktreeError::new(format!("failed to spawn git {}: {error}", args.join(" ")))
    })?;
    if !output.status.success() {
        return Err(WorktreeError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Run a git subcommand inside the worktree directory (used for the reconcile
/// commit), with the deterministic capo identity.
fn git_in_worktree(worktree_path: &Path, args: &[&str]) -> Result<(), WorktreeError> {
    git_in_repo(worktree_path, args)
}

/// Run a git subcommand inside the worktree and capture stdout (used to read the
/// reconcile tip SHA).
fn git_capture_in_worktree(worktree_path: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let output = worktree_git(worktree_path, args)
        .output()
        .map_err(|error| {
            WorktreeError::new(format!("failed to spawn git {}: {error}", args.join(" ")))
        })?;
    if !output.status.success() {
        return Err(WorktreeError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Build a `git -C <dir>` command with a deterministic committer identity, so a
/// worktree operation never fails on a missing global `user.name`/`user.email`
/// and never records the operator's identity.
fn worktree_git(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_AUTHOR_NAME", "capo-worktree")
        .env("GIT_AUTHOR_EMAIL", "worktree@capo.local")
        .env("GIT_COMMITTER_NAME", "capo-worktree")
        .env("GIT_COMMITTER_EMAIL", "worktree@capo.local")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args);
    command
}

/// A collision-free per-key segment: lower-hex of the key bytes, the same
/// injective encoding the SG5 lock / SG8 checkpoint use, so two distinct keys
/// never collide to one worktree path/branch.
fn key_segment(key: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(key.len() * 2);
    for byte in key.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("capo-dp8-{name}-{nanos}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// Initialize a git repo with one commit so `git worktree add` has a base.
    fn init_repo(name: &str) -> PathBuf {
        let repo = temp_dir(name);
        run_git(&repo, &["init", "--quiet"]);
        run_git(&repo, &["config", "user.name", "test"]);
        run_git(&repo, &["config", "user.email", "test@capo.local"]);
        std::fs::write(repo.join("seed.txt"), b"seed\n").unwrap();
        run_git(&repo, &["add", "-A"]);
        run_git(
            &repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--no-verify",
                "--quiet",
                "-m",
                "seed",
            ],
        );
        repo
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@capo.local")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@capo.local")
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    fn request(repo: &Path, worktrees: &Path, key: &str) -> WorktreeRequest {
        WorktreeRequest {
            repo_root: repo.to_path_buf(),
            worktrees_root: worktrees.to_path_buf(),
            key: key.to_string(),
        }
    }

    #[test]
    fn create_carves_a_dedicated_worktree_off_the_repo() {
        let repo = init_repo("create");
        let worktrees = temp_dir("create-wt");
        let manager = WorktreeManager::new();
        let outcome = manager
            .create(&request(&repo, &worktrees, "session-a"))
            .expect("create worktree");
        assert!(!outcome.reattached);
        assert!(outcome.worktree.worktree_path.join(".git").exists());
        assert!(outcome.worktree.worktree_path.join("seed.txt").exists());
        // The worktree lives outside the operator's repo root.
        assert!(!outcome.worktree.worktree_path.starts_with(&repo));
        assert!(outcome.events.iter().any(|e| e.kind == "worktree.created"));
    }

    /// Two concurrent sessions get distinct worktrees with no cross-contamination.
    #[test]
    fn two_sessions_write_distinct_worktrees_without_cross_contamination() {
        let repo = init_repo("two-sessions");
        let worktrees = temp_dir("two-sessions-wt");
        let manager = WorktreeManager::new();
        let a = manager
            .create(&request(&repo, &worktrees, "session-a"))
            .unwrap()
            .worktree;
        let b = manager
            .create(&request(&repo, &worktrees, "session-b"))
            .unwrap()
            .worktree;
        assert_ne!(a.worktree_path, b.worktree_path);
        assert_ne!(a.branch, b.branch);

        // Each session writes a DISTINCT file in its own worktree.
        std::fs::write(a.worktree_path.join("a-only.txt"), b"a\n").unwrap();
        std::fs::write(b.worktree_path.join("b-only.txt"), b"b\n").unwrap();

        // No cross-contamination: a's file is absent from b's tree and vice versa,
        // and neither leaks into the operator's live tree.
        assert!(!b.worktree_path.join("a-only.txt").exists());
        assert!(!a.worktree_path.join("b-only.txt").exists());
        assert!(!repo.join("a-only.txt").exists());
        assert!(!repo.join("b-only.txt").exists());
    }

    #[test]
    fn create_is_idempotent_reattach_for_a_continued_key() {
        let repo = init_repo("reattach");
        let worktrees = temp_dir("reattach-wt");
        let manager = WorktreeManager::new();
        let first = manager
            .create(&request(&repo, &worktrees, "goal-1"))
            .unwrap();
        assert!(!first.reattached);
        std::fs::write(first.worktree.worktree_path.join("work.txt"), b"x\n").unwrap();
        // A continued goal reattaches to the SAME worktree, keeping its files.
        let second = manager
            .create(&request(&repo, &worktrees, "goal-1"))
            .unwrap();
        assert!(second.reattached);
        assert_eq!(first.worktree.worktree_path, second.worktree.worktree_path);
        assert!(second.worktree.worktree_path.join("work.txt").exists());
    }

    #[test]
    fn reconcile_records_a_merge_back_tip_without_touching_operator_branch() {
        let repo = init_repo("reconcile");
        let worktrees = temp_dir("reconcile-wt");
        let manager = WorktreeManager::new();
        let wt = manager
            .create(&request(&repo, &worktrees, "session-r"))
            .unwrap()
            .worktree;
        let operator_head_before =
            String::from_utf8(run_git_capture(&repo, &["rev-parse", "HEAD"])).unwrap();
        std::fs::write(wt.worktree_path.join("change.txt"), b"changed\n").unwrap();
        let outcome = manager.reconcile(&wt).expect("reconcile");
        assert!(
            outcome
                .events
                .iter()
                .any(|e| e.kind == "worktree.reconciled")
        );
        // The operator's checked-out branch is untouched by the reconcile.
        let operator_head_after =
            String::from_utf8(run_git_capture(&repo, &["rev-parse", "HEAD"])).unwrap();
        assert_eq!(operator_head_before, operator_head_after);
    }

    #[test]
    fn teardown_removes_the_worktree_and_is_idempotent() {
        let repo = init_repo("teardown");
        let worktrees = temp_dir("teardown-wt");
        let manager = WorktreeManager::new();
        let wt = manager
            .create(&request(&repo, &worktrees, "session-t"))
            .unwrap()
            .worktree;
        assert!(wt.worktree_path.exists());
        let outcome = manager.teardown(&wt).expect("teardown");
        assert!(
            outcome
                .events
                .iter()
                .any(|e| e.kind == "worktree.torn_down")
        );
        assert!(!wt.worktree_path.exists());
        // A second teardown of an already-removed worktree completes idempotently.
        manager.teardown(&wt).expect("idempotent teardown");
    }

    #[test]
    fn create_refuses_a_non_git_root() {
        let not_repo = temp_dir("not-a-repo");
        let worktrees = temp_dir("not-a-repo-wt");
        let manager = WorktreeManager::new();
        let error = manager
            .create(&request(&not_repo, &worktrees, "session-x"))
            .expect_err("must refuse a non-git root");
        assert!(error.detail().contains("not a git repository"));
    }

    fn run_git_capture(dir: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        output.stdout
    }
}
