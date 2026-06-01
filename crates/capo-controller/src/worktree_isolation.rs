//! DP8: controller-owned git worktree isolation per session/goal.
//!
//! This graduates the runtime-boundary [`WorktreeManager`] primitive
//! (`capo-runtime::worktree`) into the controller seam: the manager owns the
//! `git worktree add/reconcile/remove` mechanics, and THIS module turns each
//! lifecycle step into durable `worktree.created` / `worktree.reconciled` /
//! `worktree.torn_down` events plus a [`WorktreeProjection`] read-model row, so a
//! worktree can be reconstructed/inspected after a restart and is never silently
//! abandoned. It mirrors the SG8 [`crate::checkpoint`] pattern: a runtime
//! mechanism + a controller method that persists an auditable event + projection.
//!
//! Isolation unit: a session's workspace-write run executes in a DEDICATED `git
//! worktree` carved off the operator's live repository (on its own per-key
//! branch) rather than in the operator's checked-out tree. The `real-turn-loop`
//! workspace confinement is then scoped to [`IsolatedWorktree::worktree_path`],
//! so two concurrent sessions never share a working tree.
//!
//! Composition with `safety-gates`:
//!
//! - Single-writer lock: each worktree is its own workspace root, so the SG5
//!   single-writer lease keys on the worktree path -- two sessions in two
//!   worktrees hold two independent leases and never contend.
//! - Pre-write checkpoint + rollback: a [`crate::CheckpointScope`] scoped to a
//!   worktree path takes/rolls back that worktree's checkpoints independently of
//!   any other worktree, so a rollback in one worktree leaves the other untouched
//!   (proven by the rollback test).
//!
//! Worktree-PER-GOAL slice (`goal-autonomy`): a worktree can be BOUND to a
//! `Goal`/`GoalAttempt` via the request key + the durable `goal_id` on the
//! projection, so a continued goal reattaches to its existing worktree
//! ([`WorktreeManager::create`] reattaches idempotently for a known key) rather
//! than spawning a fresh one. This does NOT change the goal model itself.

use std::time::{SystemTime, UNIX_EPOCH};

use capo_runtime::{IsolatedWorktree, WorktreeError, WorktreeManager, WorktreeRequest};
use capo_state::WorktreeProjection;

use super::*;

/// Wall-clock millis-since-epoch, the instant a worktree lifecycle step is
/// recorded. Clamped to 0 before the epoch -- the shared lifecycle-timestamp
/// shape the SG8 checkpoint and SG5 lease use.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a worktree hangs on the loop's scope tree, plus the optional goal
/// binding for the worktree-PER-GOAL slice and the originating/worktrees roots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    /// The goal/goal-attempt this worktree is bound to (the per-goal slice).
    /// `None` for a plain session-scoped worktree. A continued goal reattaches to
    /// the worktree carrying this goal id.
    pub goal_id: Option<GoalId>,
    /// The operator's live repository root the worktree is carved from. Writes are
    /// confined to the worktree path, never this root.
    pub repo_root: String,
    /// The controller-owned root under which the dedicated worktree is created.
    /// Keeping it OUTSIDE `repo_root` means a session's files never overlay the
    /// operator's tree.
    pub worktrees_root: String,
}

impl WorktreeScope {
    /// The stable per-worktree key. The per-goal slice keys on the goal so a
    /// continued goal reattaches to its existing worktree; otherwise the session
    /// is the isolation unit. Two distinct keys get two distinct worktrees.
    fn key(&self) -> String {
        match &self.goal_id {
            Some(goal_id) => format!("goal:{goal_id}"),
            None => format!("session:{}", self.session_id),
        }
    }

    fn request(&self) -> WorktreeRequest {
        WorktreeRequest {
            repo_root: PathBuf::from(&self.repo_root),
            worktrees_root: PathBuf::from(&self.worktrees_root),
            key: self.key(),
        }
    }

    /// The deterministic worktree id (one durable row per worktree). Keyed on the
    /// session + the isolation key so the SAME session/goal reattaches to the same
    /// row across a restart, and two distinct sessions/goals never collide.
    fn worktree_id(&self) -> String {
        format!(
            "worktree-{}-{}",
            self.session_id,
            stable_worktree_hash(&self.key())
        )
    }
}

/// The typed outcome of a controller worktree lifecycle step: the isolated
/// worktree (its `worktree_path` is the run's confinement scope) plus the durable
/// projection and whether a create reattached to an existing worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeLifecycle {
    pub worktree_id: String,
    pub worktree: IsolatedWorktree,
    pub reattached: bool,
    pub projection: WorktreeProjection,
}

impl FakeBoundaryController {
    /// DP8: CREATE-ON-SESSION-START -- carve (or reattach to) the session/goal's
    /// isolated worktree and record a durable `worktree.created` event +
    /// [`WorktreeProjection`].
    ///
    /// A continued goal (same key) reattaches idempotently to its existing
    /// worktree rather than spawning a fresh one; the durable row is keyed on the
    /// session + isolation key so the reattach holds across a restart. A failed
    /// `git worktree add` is a typed [`WorktreeError`], never a silent
    /// fall-through to the operator's live tree.
    pub fn create_worktree(
        &self,
        scope: &WorktreeScope,
    ) -> StateResult<Result<WorktreeLifecycle, WorktreeError>> {
        let manager = WorktreeManager::new();
        let outcome = match manager.create(&scope.request()) {
            Ok(outcome) => outcome,
            Err(error) => return Ok(Err(error)),
        };
        let worktree_id = scope.worktree_id();
        let created_at = epoch_millis().to_string();
        let projection = WorktreeProjection {
            worktree_id: worktree_id.clone(),
            project_id: self.project_id.clone(),
            session_id: scope.session_id.clone(),
            run_id: Some(scope.run_id.clone()),
            goal_id: scope.goal_id.clone(),
            repo_root: outcome.worktree.repo_root.display().to_string(),
            worktree_path: outcome.worktree.worktree_path.display().to_string(),
            branch: outcome.worktree.branch.clone(),
            status: WorktreeProjection::ACTIVE.to_string(),
            created_at: Some(created_at.clone()),
            reconciled_at: None,
            torn_down_at: None,
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "worktree_id": worktree_id,
            "worktree_path": projection.worktree_path,
            "repo_root": projection.repo_root,
            "branch": projection.branch,
            "goal_id": scope.goal_id.as_ref().map(|id| id.to_string()),
            "reattached": outcome.reattached,
            "created_at": created_at,
            "status": WorktreeProjection::ACTIVE,
        })
        .to_string();

        self.append_worktree_event(
            scope,
            EventKind::WorktreeCreated,
            &worktree_id,
            payload,
            &projection,
        )?;

        Ok(Ok(WorktreeLifecycle {
            worktree_id,
            worktree: outcome.worktree,
            reattached: outcome.reattached,
            projection,
        }))
    }

    /// DP8: RECONCILE / MERGE-BACK POINT -- record a reconcile point for the
    /// worktree and stamp `reconciled_at` on its durable row.
    ///
    /// Commits any outstanding work onto the worktree's isolated branch (the
    /// operator's checked-out branch is untouched) and records an auditable
    /// `worktree.reconciled` event so the merge-back tip is on the log.
    pub fn reconcile_worktree(
        &self,
        scope: &WorktreeScope,
        worktree: &IsolatedWorktree,
    ) -> StateResult<Result<WorktreeLifecycle, WorktreeError>> {
        let worktree_id = scope.worktree_id();
        let Some(existing) = self.state.worktree_by_id(&worktree_id)? else {
            return Ok(Err(WorktreeError {
                message: format!("cannot reconcile unknown worktree: {worktree_id}"),
            }));
        };
        let manager = WorktreeManager::new();
        let outcome = match manager.reconcile(worktree) {
            Ok(outcome) => outcome,
            Err(error) => return Ok(Err(error)),
        };
        let reconciled_at = epoch_millis().to_string();
        let mut projection = existing;
        projection.status = WorktreeProjection::RECONCILED.to_string();
        projection.reconciled_at = Some(reconciled_at.clone());
        projection.updated_sequence = 0;

        let detail = outcome
            .events
            .iter()
            .find(|event| event.kind == "worktree.reconciled")
            .map(|event| event.detail.clone())
            .unwrap_or_default();
        let payload = serde_json::json!({
            "worktree_id": worktree_id,
            "worktree_path": projection.worktree_path,
            "branch": projection.branch,
            "reconciled_at": reconciled_at,
            "detail": detail,
            "status": WorktreeProjection::RECONCILED,
        })
        .to_string();

        self.append_worktree_event(
            scope,
            EventKind::WorktreeReconciled,
            &worktree_id,
            payload,
            &projection,
        )?;

        Ok(Ok(WorktreeLifecycle {
            worktree_id,
            worktree: outcome.worktree,
            reattached: false,
            projection,
        }))
    }

    /// DP8: TEARDOWN -- remove the worktree from disk and stamp `torn_down_at` on
    /// its durable row, recording an auditable `worktree.torn_down` event.
    ///
    /// Idempotent: tearing down an already-removed worktree still completes and
    /// records the terminal status, so a retried teardown after a partial crash
    /// finishes cleanly.
    pub fn teardown_worktree(
        &self,
        scope: &WorktreeScope,
        worktree: &IsolatedWorktree,
    ) -> StateResult<Result<WorktreeLifecycle, WorktreeError>> {
        let worktree_id = scope.worktree_id();
        let Some(existing) = self.state.worktree_by_id(&worktree_id)? else {
            return Ok(Err(WorktreeError {
                message: format!("cannot tear down unknown worktree: {worktree_id}"),
            }));
        };
        let manager = WorktreeManager::new();
        let outcome = match manager.teardown(worktree) {
            Ok(outcome) => outcome,
            Err(error) => return Ok(Err(error)),
        };
        let torn_down_at = epoch_millis().to_string();
        let mut projection = existing;
        projection.status = WorktreeProjection::TORN_DOWN.to_string();
        projection.torn_down_at = Some(torn_down_at.clone());
        projection.updated_sequence = 0;

        let payload = serde_json::json!({
            "worktree_id": worktree_id,
            "worktree_path": projection.worktree_path,
            "branch": projection.branch,
            "torn_down_at": torn_down_at,
            "status": WorktreeProjection::TORN_DOWN,
        })
        .to_string();

        self.append_worktree_event(
            scope,
            EventKind::WorktreeTornDown,
            &worktree_id,
            payload,
            &projection,
        )?;

        Ok(Ok(WorktreeLifecycle {
            worktree_id,
            worktree: outcome.worktree,
            reattached: false,
            projection,
        }))
    }

    /// DP8: read one worktree back by id (`None` when absent), so the recovery /
    /// audit path can find a worktree's durable record after a restart.
    pub fn worktree(&self, worktree_id: &str) -> StateResult<Option<WorktreeProjection>> {
        self.state.worktree_by_id(worktree_id)
    }

    /// DP8: every worktree for a session, oldest first -- the per-session
    /// lifecycle the recovery/audit path reads to prove no worktree is orphaned.
    pub fn worktrees_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<WorktreeProjection>> {
        self.state.worktrees_for_session(session_id)
    }

    /// DP8 (per-goal slice): the worktree currently bound to a goal, most-recent
    /// first, so a continued goal can reattach to its existing live worktree.
    pub fn worktrees_for_goal(&self, goal_id: &GoalId) -> StateResult<Vec<WorktreeProjection>> {
        self.state.worktrees_for_goal(goal_id)
    }

    /// Append one worktree lifecycle event + its projection, scoped to the loop's
    /// scope tree (shared shape with the SG8 checkpoint append).
    fn append_worktree_event(
        &self,
        scope: &WorktreeScope,
        kind: EventKind,
        worktree_id: &str,
        payload: String,
        projection: &WorktreeProjection,
    ) -> StateResult<()> {
        self.state.append_event(
            scoped_event(
                &format!("event-{}-{worktree_id}", kind.as_str().replace('.', "-")),
                kind,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_item(worktree_id.to_string())
            .with_payload(payload),
            &[ProjectionRecord::Worktree(projection.clone())],
        )?;
        Ok(())
    }
}

/// FNV-1a hash for stable worktree ids (no extra dependency; same shape as the
/// SG8 checkpoint stable-id hash).
fn stable_worktree_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_state::SqliteStateStore;

    use super::*;
    use crate::CheckpointScope;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("capo-dp8c-{name}-{nanos}-{n}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.canonicalize().expect("canonicalize")
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
            .expect("git");
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    /// A git repo with one seed commit so `git worktree add` has a base.
    fn init_repo(name: &str) -> PathBuf {
        let repo = temp_root(name);
        run_git(&repo, &["init", "--quiet"]);
        run_git(&repo, &["config", "user.name", "test"]);
        run_git(&repo, &["config", "user.email", "test@capo.local"]);
        std::fs::write(repo.join("seed.txt"), b"seed\n").expect("seed");
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

    fn open_controller(state_root: &Path) -> FakeBoundaryController {
        FakeBoundaryController::open(ProjectId::new("project-capo"), state_root)
            .expect("controller")
    }

    fn scope(repo: &Path, worktrees: &Path, session: &str, goal: Option<&str>) -> WorktreeScope {
        WorktreeScope {
            task_id: TaskId::new("task-dp8"),
            agent_id: AgentId::new("agent-dp8"),
            session_id: SessionId::new(session),
            run_id: RunId::new(format!("run-{session}")),
            goal_id: goal.map(GoalId::new),
            repo_root: repo.display().to_string(),
            worktrees_root: worktrees.display().to_string(),
        }
    }

    /// DP8 acceptance: two concurrent sessions write to DISTINCT worktrees with no
    /// cross-contamination, each recorded as a durable `worktree.created` row.
    #[test]
    fn dp8_two_sessions_write_distinct_worktrees_no_cross_contamination() {
        let repo = init_repo("two-sessions");
        let worktrees = temp_root("two-sessions-wt");
        let state_root = temp_root("state");
        let controller = open_controller(&state_root);

        let scope_a = scope(&repo, &worktrees, "session-a", None);
        let scope_b = scope(&repo, &worktrees, "session-b", None);
        let a = controller
            .create_worktree(&scope_a)
            .expect("io")
            .expect("create a");
        let b = controller
            .create_worktree(&scope_b)
            .expect("io")
            .expect("create b");

        assert_ne!(a.worktree.worktree_path, b.worktree.worktree_path);
        assert_ne!(a.worktree.branch, b.worktree.branch);
        assert_ne!(a.worktree_id, b.worktree_id);

        // Each session writes a DISTINCT file in its own worktree.
        std::fs::write(a.worktree.worktree_path.join("a-only.txt"), b"a\n").unwrap();
        std::fs::write(b.worktree.worktree_path.join("b-only.txt"), b"b\n").unwrap();

        // No cross-contamination, and neither leaks into the operator's live tree.
        assert!(!b.worktree.worktree_path.join("a-only.txt").exists());
        assert!(!a.worktree.worktree_path.join("b-only.txt").exists());
        assert!(!repo.join("a-only.txt").exists());
        assert!(!repo.join("b-only.txt").exists());

        // Both worktrees are durable rows for their sessions.
        let rows_a = controller
            .worktrees_for_session(&scope_a.session_id)
            .expect("rows a");
        assert_eq!(rows_a.len(), 1);
        assert_eq!(rows_a[0].status, WorktreeProjection::ACTIVE);
    }

    /// DP8 acceptance: a rollback restores ONE worktree to its pre-write
    /// checkpoint without disturbing the other (composition with the SG8
    /// shadow-git checkpoint, scoped per-worktree).
    #[test]
    fn dp8_rollback_restores_one_worktree_without_disturbing_the_other() {
        let repo = init_repo("rollback");
        let worktrees = temp_root("rollback-wt");
        let shadow = temp_root("rollback-shadow");
        let state_root = temp_root("state");
        let controller = open_controller(&state_root);

        let scope_a = scope(&repo, &worktrees, "session-a", None);
        let scope_b = scope(&repo, &worktrees, "session-b", None);
        let a = controller
            .create_worktree(&scope_a)
            .expect("io")
            .expect("a");
        let b = controller
            .create_worktree(&scope_b)
            .expect("io")
            .expect("b");

        // Pre-write state in each worktree, then a per-worktree checkpoint.
        std::fs::write(a.worktree.worktree_path.join("doc.txt"), b"a-before\n").unwrap();
        std::fs::write(b.worktree.worktree_path.join("doc.txt"), b"b-before\n").unwrap();

        let cp_scope_a = CheckpointScope {
            task_id: scope_a.task_id.clone(),
            agent_id: scope_a.agent_id.clone(),
            session_id: scope_a.session_id.clone(),
            run_id: scope_a.run_id.clone(),
            turn_id: TurnId::new("turn-1"),
            workspace_root: a.worktree.worktree_path.display().to_string(),
            shadow_git_root: shadow.display().to_string(),
        };
        let cp_a = controller
            .create_checkpoint(&cp_scope_a)
            .expect("io")
            .expect("checkpoint a");

        // Write AFTER the checkpoint in BOTH worktrees.
        std::fs::write(a.worktree.worktree_path.join("doc.txt"), b"a-after\n").unwrap();
        std::fs::write(b.worktree.worktree_path.join("doc.txt"), b"b-after\n").unwrap();

        // Roll back ONLY worktree a.
        controller
            .restore_checkpoint(&cp_scope_a, &cp_a.checkpoint_id)
            .expect("io")
            .expect("restore a");

        // a is back to its pre-write state; b's independent write is untouched.
        assert_eq!(
            std::fs::read_to_string(a.worktree.worktree_path.join("doc.txt")).unwrap(),
            "a-before\n",
            "worktree a rolled back to its checkpoint"
        );
        assert_eq!(
            std::fs::read_to_string(b.worktree.worktree_path.join("doc.txt")).unwrap(),
            "b-after\n",
            "worktree b is undisturbed by a's rollback"
        );
    }

    /// DP8 acceptance: worktree lifecycle events rebuild after a restart and the
    /// worktree is REATTACHABLE (not orphaned). Create -> reconcile -> restart ->
    /// rebuild projects identically, and a continued goal reattaches to the same
    /// worktree row + path.
    #[test]
    fn dp8_lifecycle_rebuilds_after_restart_and_worktree_is_reattachable() {
        let repo = init_repo("restart");
        let worktrees = temp_root("restart-wt");
        let state_root = temp_root("state");
        let controller = open_controller(&state_root);

        let scope = scope(&repo, &worktrees, "session-g", Some("goal-1"));
        let created = controller
            .create_worktree(&scope)
            .expect("io")
            .expect("create");
        assert!(!created.reattached);
        std::fs::write(created.worktree.worktree_path.join("work.txt"), b"x\n").unwrap();

        let reconciled = controller
            .reconcile_worktree(&scope, &created.worktree)
            .expect("io")
            .expect("reconcile");
        assert_eq!(reconciled.projection.status, WorktreeProjection::RECONCILED);
        assert!(reconciled.projection.reconciled_at.is_some());

        // Restart: reopen the store and rebuild projections from the event log.
        let reopened = SqliteStateStore::open(&state_root).expect("reopen");
        reopened.rebuild_projections().expect("rebuild");
        let rebuilt = reopened
            .worktree_by_id(&created.worktree_id)
            .expect("query")
            .expect("present after rebuild");
        assert_eq!(rebuilt.worktree_path, created.projection.worktree_path);
        assert_eq!(rebuilt.branch, created.projection.branch);
        assert_eq!(rebuilt.status, WorktreeProjection::RECONCILED);
        assert!(rebuilt.is_live(), "a reconciled worktree is still live");

        // Per-goal reattach: a continued goal looks the worktree up by its goal
        // binding and reattaches to the SAME row + path rather than orphaning it.
        let for_goal = reopened
            .worktrees_for_goal(&GoalId::new("goal-1"))
            .expect("for goal");
        assert_eq!(for_goal.len(), 1);
        assert_eq!(for_goal[0].worktree_id, created.worktree_id);

        let controller2 = open_controller(&state_root);
        let reattached = controller2
            .create_worktree(&scope)
            .expect("io")
            .expect("reattach");
        assert!(reattached.reattached, "continued goal reattaches");
        assert_eq!(reattached.worktree_id, created.worktree_id);
        assert_eq!(
            reattached.worktree.worktree_path,
            created.worktree.worktree_path
        );
        assert!(
            reattached.worktree.worktree_path.join("work.txt").exists(),
            "reattached worktree keeps its files"
        );
    }

    /// DP8: teardown removes the worktree, stamps the terminal status on the
    /// durable row, and is idempotent (a worktree is never silently abandoned).
    #[test]
    fn dp8_teardown_stamps_terminal_status_and_is_idempotent() {
        let repo = init_repo("teardown");
        let worktrees = temp_root("teardown-wt");
        let state_root = temp_root("state");
        let controller = open_controller(&state_root);

        let scope = scope(&repo, &worktrees, "session-t", None);
        let created = controller
            .create_worktree(&scope)
            .expect("io")
            .expect("create");
        assert!(created.worktree.worktree_path.exists());

        let torn = controller
            .teardown_worktree(&scope, &created.worktree)
            .expect("io")
            .expect("teardown");
        assert_eq!(torn.projection.status, WorktreeProjection::TORN_DOWN);
        assert!(!created.worktree.worktree_path.exists());

        let row = controller
            .worktree(&created.worktree_id)
            .expect("query")
            .expect("present");
        assert_eq!(row.status, WorktreeProjection::TORN_DOWN);
        assert!(!row.is_live());

        // Idempotent: a retried teardown after the worktree is gone still
        // completes and keeps the terminal status.
        controller
            .teardown_worktree(&scope, &created.worktree)
            .expect("io")
            .expect("idempotent teardown");
    }

    /// DP8: a worktree create against a non-git root is a typed error, never a
    /// silent fall-through to the operator's tree (no row is recorded).
    #[test]
    fn dp8_create_against_non_git_root_is_typed_error_with_no_row() {
        let not_repo = temp_root("not-a-repo");
        let worktrees = temp_root("not-a-repo-wt");
        let state_root = temp_root("state");
        let controller = open_controller(&state_root);

        let scope = scope(&not_repo, &worktrees, "session-x", None);
        let error = controller
            .create_worktree(&scope)
            .expect("io")
            .expect_err("must refuse a non-git root");
        assert!(error.detail().contains("not a git repository"));
        assert!(
            controller
                .worktrees_for_session(&scope.session_id)
                .expect("rows")
                .is_empty(),
            "a refused create records no worktree row"
        );
    }
}
