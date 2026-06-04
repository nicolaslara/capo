//! SG8: controller-owned checkpoint/rollback as shadow-git.
//!
//! This graduates the designed `checkpoint.created` / `checkpoint.restored`
//! events and the `checkpoints` projection/table (`state-model.md:894-896,1042`)
//! from design to code, and UPGRADES the `real-turn-loop` single-snapshot safety
//! floor: the RTL pre-write snapshot (a directory copy under the artifact root --
//! `capo-server::safety_floor::WorkspaceCheckpoint`) is replaced by per-turn
//! shadow-git checkpoints that are restorable per-turn and survive a restart.
//!
//! Mechanism (the SG8 open question, RESOLVED): a SEPARATE shadow `.git`
//! directory, NOT a stash-ring. Each workspace gets a shadow repo whose `GIT_DIR`
//! lives under the controller's state root (`<shadow_git_root>/<workspace-key>`)
//! and whose `GIT_WORK_TREE` is the workspace itself. A checkpoint is a commit in
//! that shadow repo; restore is a `git checkout` + `git clean` of that commit's
//! tree. This choice satisfies the two hard requirements:
//!
//! - Restorable per-turn: every checkpoint is its own commit, so the loop can
//!   take one per turn and roll back to any of them independently.
//! - Survives restart: the shadow repo and its commits live on disk, and the
//!   restorable commit SHA is recorded in the durable [`CheckpointProjection`] +
//!   `checkpoint.created` event, so a checkpoint taken before a restart is still
//!   restorable after the controller rebuilds projections from the log.
//!
//! Why a separate git dir and not the workspace's own `.git`: the workspace may
//! be a real user repository. Committing into its `.git` would pollute the user's
//! history, index, and refs. Pointing `GIT_DIR` at a shadow directory keeps every
//! checkpoint commit out of the user's repository entirely while still using git's
//! battle-tested content-addressed storage and tree/checkout machinery.
//!
//! Auditability: both create and restore emit events
//! (`checkpoint.created`/`checkpoint.restored`) and update the durable
//! projection, so a rollback is fully auditable on the event log.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use capo_state::CheckpointProjection;

use super::*;

/// The checkpoint mechanism marker recorded on the projection/event, so a later
/// mechanism change (e.g. a depth-workpad worktree isolation) is distinguishable
/// on the log.
const CHECKPOINT_KIND_SHADOW_GIT: &str = "shadow_git";

/// The committer/author identity the shadow repo commits under, so a checkpoint
/// commit never inherits (or requires) the operator's global git identity.
const SHADOW_GIT_IDENTITY_NAME: &str = "capo-checkpoint";
const SHADOW_GIT_IDENTITY_EMAIL: &str = "checkpoint@capo.local";

/// Wall-clock millis-since-epoch, the instant a checkpoint is created/restored.
/// Clamped to 0 before the epoch. Shared shape with the SG3 grant lifecycle and
/// SG5 lease timestamps so all lifecycle timestamps compare on the same basis.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a checkpoint hangs on the loop's scope tree, plus the workspace it
/// covers and the shadow-git root the commit lives under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    /// The workspace root the checkpoint covers. Lexically normalized
    /// (`.`/`..`/`//`/trailing-separator resolved) before it keys the shadow repo,
    /// so the same directory spelled two ways shares one shadow repo.
    pub workspace_root: String,
    /// The root under which per-workspace shadow `.git` directories live (a
    /// controller-owned location under the state root). The shadow git dir for a
    /// workspace is `<shadow_git_root>/<workspace-key>`.
    pub shadow_git_root: String,
}

impl CheckpointScope {
    /// The shadow `.git` directory for this scope's workspace -- a collision-free
    /// per-workspace location under the shadow-git root.
    fn shadow_git_dir(&self) -> PathBuf {
        Path::new(&self.shadow_git_root).join(workspace_key_segment(&self.workspace_root))
    }
}

/// A typed reason a checkpoint create/restore could not proceed, surfaced to the
/// loop as a decide-style outcome (not a panic) so the loop can reflect on it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointError {
    pub message: String,
}

impl CheckpointError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn agent_message(&self) -> &str {
        &self.message
    }
}

/// SG8: the typed outcome of creating a shadow-git checkpoint before a workspace
/// write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointCreated {
    pub checkpoint_id: String,
    /// The shadow-repo commit SHA the checkpoint is restorable to.
    pub commit_ref: String,
    /// The shadow commit's tree SHA -- a content fingerprint of the checkpointed
    /// workspace.
    pub content_hash: String,
    /// True when this exact pre-write state was already checkpointed for the scope
    /// (idempotent re-checkpoint, no new event/commit row appended).
    pub already_checkpointed: bool,
    pub projection: CheckpointProjection,
}

/// SG8: the typed outcome of restoring a checkpoint (the `Restore` command).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointRestored {
    pub checkpoint_id: String,
    /// The commit SHA the workspace was returned to.
    pub commit_ref: String,
    pub projection: CheckpointProjection,
}

impl FakeBoundaryController {
    /// SG8: create a shadow-git checkpoint of the workspace BEFORE a real write,
    /// so any subsequent write is reversible by one [`Self::restore_checkpoint`]
    /// (`Restore`) command.
    ///
    /// Initializes the per-workspace shadow repo on first use (a separate
    /// `GIT_DIR` under the state root; the user's own `.git` is never touched),
    /// stages the entire workspace, commits it (allowing an empty commit so an
    /// unchanged tree still produces a restorable ref), and records the commit SHA
    /// as the restorable ref on a durable `checkpoint.created` event +
    /// [`CheckpointProjection`]. Re-checkpointing the SAME pre-write tree for the
    /// same scope is idempotent (same checkpoint id, no duplicate event/row).
    pub fn create_checkpoint(
        &self,
        scope: &CheckpointScope,
    ) -> StateResult<Result<CheckpointCreated, CheckpointError>> {
        let workspace = Path::new(&scope.workspace_root);
        if !workspace.is_dir() {
            return Ok(Err(CheckpointError::new(format!(
                "checkpoint workspace root does not exist or is not a directory: {}",
                scope.workspace_root
            ))));
        }
        let shadow_git_dir = scope.shadow_git_dir();
        if let Err(error) = ensure_shadow_repo(&shadow_git_dir) {
            return Ok(Err(error));
        }

        // Stage the whole workspace and commit. `--allow-empty` so an unchanged
        // tree (a no-op turn) still produces a restorable ref rather than failing.
        if let Err(error) = git_in_shadow(&shadow_git_dir, workspace, &["add", "-A"]) {
            return Ok(Err(error));
        }
        let commit_message = format!(
            "capo checkpoint run={} turn={}",
            scope.run_id, scope.turn_id
        );
        if let Err(error) = git_in_shadow(
            &shadow_git_dir,
            workspace,
            &[
                // Neutralize the operator's global git config that could break a
                // checkpoint commit: GPG signing (no secret key in an unattended
                // run) and any commit hooks. These `-c` overrides apply to THIS
                // invocation only and never write to the operator's config.
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "--no-verify",
                "--quiet",
                "-m",
                &commit_message,
            ],
        ) {
            return Ok(Err(error));
        }
        let commit_ref = match git_capture(&shadow_git_dir, workspace, &["rev-parse", "HEAD"]) {
            Ok(sha) => sha.trim().to_string(),
            Err(error) => return Ok(Err(error)),
        };
        // The tree SHA is git's own content fingerprint of the checkpointed
        // workspace: two checkpoints of byte-identical content share a tree SHA.
        let content_hash =
            match git_capture(&shadow_git_dir, workspace, &["rev-parse", "HEAD^{tree}"]) {
                Ok(tree) => tree.trim().to_string(),
                Err(error) => return Ok(Err(error)),
            };

        // Key the checkpoint id on (run, turn, content tree) so a re-checkpoint of
        // the SAME pre-write state for the same turn is idempotent and re-projects
        // identically, while a changed tree gets a distinct id.
        let checkpoint_id = format!(
            "checkpoint-{}-{}",
            scope.run_id,
            stable_checkpoint_hash(&format!(
                "{}:{}:{}",
                scope.run_id, scope.turn_id, content_hash
            ))
        );
        let already_checkpointed = self.state.checkpoint_by_id(&checkpoint_id)?.is_some();

        let created_at = epoch_millis().to_string();
        let projection = CheckpointProjection {
            checkpoint_id: checkpoint_id.clone(),
            project_id: self.project_id.clone(),
            session_id: scope.session_id.clone(),
            run_id: scope.run_id.clone(),
            turn_id: Some(scope.turn_id.to_string()),
            kind: CHECKPOINT_KIND_SHADOW_GIT.to_string(),
            commit_ref: commit_ref.clone(),
            workspace_root: scope.workspace_root.clone(),
            shadow_git_dir: shadow_git_dir.display().to_string(),
            content_hash: content_hash.clone(),
            created_at: Some(created_at.clone()),
            restored_at: None,
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "checkpoint_id": checkpoint_id,
            "checkpoint_kind": CHECKPOINT_KIND_SHADOW_GIT,
            "commit_ref": commit_ref,
            "workspace_root": scope.workspace_root,
            "shadow_git_dir": shadow_git_dir.display().to_string(),
            "content_hash": content_hash,
            "created_at": created_at,
            "reversible": true,
        })
        .to_string();

        self.state.append_event(
            scoped_event(
                &format!("event-checkpoint-created-{checkpoint_id}"),
                EventKind::CheckpointCreated,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(checkpoint_id.clone())
            .with_payload(payload),
            &[ProjectionRecord::Checkpoint(projection.clone())],
        )?;

        Ok(Ok(CheckpointCreated {
            checkpoint_id,
            commit_ref,
            content_hash,
            already_checkpointed,
            projection,
        }))
    }

    /// SG8: restore a checkpoint -- the `Restore` command. Returns the workspace
    /// to the exact state captured by the checkpoint's shadow commit.
    ///
    /// Reads the durable checkpoint back by id (so it works even after a restart,
    /// reading the commit SHA from the rebuilt projection), checks the shadow
    /// commit's tree out over the workspace, and runs `git clean` so files created
    /// AFTER the checkpoint are removed -- leaving the workspace byte-identical to
    /// the checkpointed state. Emits an auditable `checkpoint.restored` event and
    /// re-emits the projection with `restored_at` stamped. Restoring a missing
    /// checkpoint is a typed error, not a panic.
    pub fn restore_checkpoint(
        &self,
        scope: &CheckpointScope,
        checkpoint_id: &str,
    ) -> StateResult<Result<CheckpointRestored, CheckpointError>> {
        let Some(checkpoint) = self.state.checkpoint_by_id(checkpoint_id)? else {
            return Ok(Err(CheckpointError::new(format!(
                "cannot restore unknown checkpoint: {checkpoint_id}"
            ))));
        };
        let workspace = Path::new(&checkpoint.workspace_root);
        if !workspace.is_dir() {
            return Ok(Err(CheckpointError::new(format!(
                "restore target workspace does not exist: {}",
                checkpoint.workspace_root
            ))));
        }
        let shadow_git_dir = PathBuf::from(&checkpoint.shadow_git_dir);
        if !shadow_git_dir.is_dir() {
            return Ok(Err(CheckpointError::new(format!(
                "shadow git dir for checkpoint {checkpoint_id} is missing: {}",
                checkpoint.shadow_git_dir
            ))));
        }

        // Restore the committed tree over the workspace, then remove anything
        // added after the checkpoint so the workspace is byte-identical to the
        // checkpointed state (a checkout alone would leave post-checkpoint files).
        if let Err(error) = git_in_shadow(
            &shadow_git_dir,
            workspace,
            &["checkout", "--force", &checkpoint.commit_ref, "--", "."],
        ) {
            return Ok(Err(error));
        }
        if let Err(error) = git_in_shadow(&shadow_git_dir, workspace, &["clean", "-fdx"]) {
            return Ok(Err(error));
        }

        let restored_at = epoch_millis().to_string();
        let mut restored = checkpoint.clone();
        restored.restored_at = Some(restored_at.clone());
        restored.updated_sequence = 0;

        let payload = serde_json::json!({
            "checkpoint_id": checkpoint.checkpoint_id,
            "checkpoint_kind": checkpoint.kind,
            "commit_ref": checkpoint.commit_ref,
            "workspace_root": checkpoint.workspace_root,
            "shadow_git_dir": checkpoint.shadow_git_dir,
            "restored_at": restored_at,
            "result": "restored",
        })
        .to_string();

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-checkpoint-restored-{}-{}",
                    checkpoint.checkpoint_id, restored_at
                ),
                EventKind::CheckpointRestored,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(checkpoint.checkpoint_id.clone())
            .with_payload(payload),
            &[ProjectionRecord::Checkpoint(restored.clone())],
        )?;

        Ok(Ok(CheckpointRestored {
            checkpoint_id: checkpoint.checkpoint_id,
            commit_ref: checkpoint.commit_ref,
            projection: restored,
        }))
    }

    /// SG8: read one checkpoint back by id (`None` when absent), so the loop /
    /// recovery / audit path can find the restorable commit ref. Reads the durable
    /// projection, so it reflects a rebuild from the event log.
    pub fn checkpoint(&self, checkpoint_id: &str) -> StateResult<Option<CheckpointProjection>> {
        self.state.checkpoint_by_id(checkpoint_id)
    }

    /// SG8: every checkpoint for a run, oldest first -- the per-turn checkpoint
    /// ring.
    pub fn checkpoints_for_run(&self, run_id: &RunId) -> StateResult<Vec<CheckpointProjection>> {
        self.state.checkpoints_for_run(run_id)
    }
}

/// Initialize the per-workspace shadow repo if it does not yet exist.
///
/// `git init --bare=false` with a SEPARATE git dir: the repo's object/ref storage
/// lives at `shadow_git_dir` while its work tree is the workspace (supplied per
/// command via `GIT_WORK_TREE`). Idempotent: an already-initialized shadow dir is
/// left untouched.
fn ensure_shadow_repo(shadow_git_dir: &Path) -> Result<(), CheckpointError> {
    if shadow_git_dir.join("HEAD").is_file() {
        return Ok(());
    }
    if let Some(parent) = shadow_git_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CheckpointError::new(format!("failed to create shadow-git root: {error}"))
        })?;
    }
    // `git init --bare <dir>` puts the repository database DIRECTLY at <dir> (a
    // bare repo has no work tree of its own), so `GIT_DIR=<dir>` resolves it. We
    // then supply the work tree explicitly per command via `GIT_WORK_TREE`, so the
    // workspace's own `.git` (if any) is never consulted.
    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(shadow_git_dir)
        .output()
        .map_err(|error| CheckpointError::new(format!("failed to spawn git init: {error}")))?;
    if !output.status.success() {
        return Err(CheckpointError::new(format!(
            "git init failed for shadow repo {}: {}",
            shadow_git_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Run a git subcommand against the shadow repo, with `GIT_DIR` pinned to the
/// shadow directory and `GIT_WORK_TREE` pinned to the workspace, so the command
/// never touches the workspace's own `.git`.
fn git_in_shadow(
    shadow_git_dir: &Path,
    work_tree: &Path,
    args: &[&str],
) -> Result<(), CheckpointError> {
    let output = shadow_git_command(shadow_git_dir, work_tree, args)
        .output()
        .map_err(|error| {
            CheckpointError::new(format!("failed to spawn git {}: {error}", args.join(" ")))
        })?;
    if !output.status.success() {
        return Err(CheckpointError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Run a git subcommand against the shadow repo and capture stdout (trimmed by
/// the caller), e.g. `rev-parse` to read a commit/tree SHA.
fn git_capture(
    shadow_git_dir: &Path,
    work_tree: &Path,
    args: &[&str],
) -> Result<String, CheckpointError> {
    let output = shadow_git_command(shadow_git_dir, work_tree, args)
        .output()
        .map_err(|error| {
            CheckpointError::new(format!("failed to spawn git {}: {error}", args.join(" ")))
        })?;
    if !output.status.success() {
        return Err(CheckpointError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Build a `git` command pinned to the shadow repo and workspace work tree, with
/// a deterministic committer identity injected via env (so a checkpoint commit
/// never depends on -- or pollutes -- the operator's global git config).
fn shadow_git_command(shadow_git_dir: &Path, work_tree: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_DIR", shadow_git_dir)
        .env("GIT_WORK_TREE", work_tree)
        // Pin a deterministic identity so commit never fails on a missing global
        // user.name/user.email and the operator's identity is never recorded.
        .env("GIT_AUTHOR_NAME", SHADOW_GIT_IDENTITY_NAME)
        .env("GIT_AUTHOR_EMAIL", SHADOW_GIT_IDENTITY_EMAIL)
        .env("GIT_COMMITTER_NAME", SHADOW_GIT_IDENTITY_NAME)
        .env("GIT_COMMITTER_EMAIL", SHADOW_GIT_IDENTITY_EMAIL)
        // Do not let a user's global hooks/config interfere with the shadow repo.
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args);
    command
}

/// Derive a collision-free per-workspace key segment for the shadow git dir, the
/// same injective lower-hex-of-normalized-path encoding the SG5 workspace lock
/// uses, so the same workspace spelled differently shares one shadow repo and two
/// distinct roots never collide.
fn workspace_key_segment(workspace_root: &str) -> String {
    use std::fmt::Write as _;
    let normalized = normalize_workspace_root(workspace_root);
    let bytes = normalized.as_os_str().as_encoded_bytes();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

/// Lexically normalize a workspace root (`.`/`..`/`//`/trailing-separator
/// resolved) WITHOUT touching the filesystem -- same rule as the SG5 lock so the
/// checkpoint shadow repo and the write lease key the same workspace identically.
fn normalize_workspace_root(workspace_root: &str) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in Path::new(workspace_root).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    if normalized.as_os_str().is_empty() {
        normalized.push(".");
    }
    normalized
}

/// FNV-1a hash for stable checkpoint ids (no extra dependency; same shape as the
/// other SG stable-id hashes).
fn stable_checkpoint_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {

    use capo_state::SqliteStateStore;

    use super::*;

    fn temp_root(name: &str) -> capo_tmptest::TempRoot {
        capo_tmptest::TempRoot::new(&format!("capo-sg8-{name}"))
    }

    /// A controller plus a fresh workspace dir and a shadow-git root dir.
    fn fixture() -> (
        FakeBoundaryController,
        capo_tmptest::TempRoot,
        capo_tmptest::TempRoot,
        capo_tmptest::TempRoot,
    ) {
        let state_root = temp_root("state");
        let workspace = temp_root("workspace");
        let shadow_git_root = temp_root("shadow");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &state_root)
            .expect("controller");
        (controller, workspace, shadow_git_root, state_root)
    }

    fn scope(workspace: &Path, shadow_git_root: &Path, turn: &str) -> CheckpointScope {
        CheckpointScope {
            task_id: TaskId::new("task-sg8"),
            agent_id: AgentId::new("agent-sg8"),
            session_id: SessionId::new("session-sg8"),
            run_id: RunId::new("run-sg8"),
            turn_id: TurnId::new(turn),
            workspace_root: workspace.display().to_string(),
            shadow_git_root: shadow_git_root.display().to_string(),
        }
    }

    fn write_file(workspace: &Path, name: &str, contents: &str) {
        std::fs::write(workspace.join(name), contents).expect("write workspace file");
    }

    fn read_file(workspace: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(workspace.join(name)).ok()
    }

    /// SG8 acceptance: create-checkpoint, write, restore returns the workspace to
    /// the prior state -- including removing a file CREATED after the checkpoint
    /// and reverting a MODIFIED file.
    #[test]
    fn sg8_create_write_restore_returns_workspace_to_prior_state() {
        let (controller, workspace, shadow, _state_root) = fixture();
        // Pre-write state.
        write_file(&workspace, "keep.txt", "original\n");
        write_file(&workspace, "edit_me.txt", "before\n");

        let scope = scope(&workspace, &shadow, "turn-1");
        let created = controller
            .create_checkpoint(&scope)
            .expect("create checkpoint io")
            .expect("create checkpoint ok");
        assert!(!created.commit_ref.is_empty(), "commit ref recorded");
        assert!(!created.already_checkpointed, "first checkpoint is new");

        // Real workspace write AFTER the checkpoint: modify a file, add a new
        // file, delete an existing file.
        write_file(&workspace, "edit_me.txt", "after\n");
        write_file(&workspace, "added_after.txt", "should disappear\n");
        std::fs::remove_file(workspace.join("keep.txt")).expect("delete keep.txt");
        assert_eq!(
            read_file(&workspace, "edit_me.txt").as_deref(),
            Some("after\n")
        );
        assert!(read_file(&workspace, "keep.txt").is_none());

        // One Restore command returns the workspace to the checkpointed state.
        let restored = controller
            .restore_checkpoint(&scope, &created.checkpoint_id)
            .expect("restore io")
            .expect("restore ok");
        assert_eq!(restored.checkpoint_id, created.checkpoint_id);

        // The post-checkpoint modification is reverted, the deleted file is back,
        // and the file added after the checkpoint is gone.
        assert_eq!(
            read_file(&workspace, "edit_me.txt").as_deref(),
            Some("before\n"),
            "modified file reverted to checkpointed contents"
        );
        assert_eq!(
            read_file(&workspace, "keep.txt").as_deref(),
            Some("original\n"),
            "deleted file restored"
        );
        assert!(
            read_file(&workspace, "added_after.txt").is_none(),
            "file added after the checkpoint is removed by restore"
        );
    }

    /// SG8 acceptance: per-turn checkpoints are independently restorable.
    #[test]
    fn sg8_per_turn_checkpoints_are_independently_restorable() {
        let (controller, workspace, shadow, _state_root) = fixture();
        write_file(&workspace, "f.txt", "v1\n");
        let scope1 = scope(&workspace, &shadow, "turn-1");
        let cp1 = controller
            .create_checkpoint(&scope1)
            .expect("io")
            .expect("ok");

        write_file(&workspace, "f.txt", "v2\n");
        let scope2 = scope(&workspace, &shadow, "turn-2");
        let cp2 = controller
            .create_checkpoint(&scope2)
            .expect("io")
            .expect("ok");
        assert_ne!(
            cp1.checkpoint_id, cp2.checkpoint_id,
            "distinct per-turn checkpoints"
        );

        write_file(&workspace, "f.txt", "v3\n");

        // Restore to turn-1's checkpoint.
        controller
            .restore_checkpoint(&scope2, &cp1.checkpoint_id)
            .expect("io")
            .expect("ok");
        assert_eq!(read_file(&workspace, "f.txt").as_deref(), Some("v1\n"));

        // Then forward to turn-2's checkpoint.
        controller
            .restore_checkpoint(&scope2, &cp2.checkpoint_id)
            .expect("io")
            .expect("ok");
        assert_eq!(read_file(&workspace, "f.txt").as_deref(), Some("v2\n"));
    }

    /// SG8 acceptance: checkpoint refs survive restart, and a checkpoint taken
    /// before restart is still restorable after.
    #[test]
    fn sg8_checkpoint_survives_restart_and_is_restorable_after() {
        let (controller, workspace, shadow, state_root) = fixture();
        write_file(&workspace, "doc.txt", "pre-restart\n");
        let scope = scope(&workspace, &shadow, "turn-1");
        let created = controller
            .create_checkpoint(&scope)
            .expect("io")
            .expect("ok");

        // Simulate a restart: reopen the store and rebuild projections from the
        // event log. The checkpoint projection must reconstruct identically.
        let reopened = SqliteStateStore::open(&state_root).expect("reopen store");
        reopened.rebuild_projections().expect("rebuild");
        let rebuilt = reopened
            .checkpoint_by_id(&created.checkpoint_id)
            .expect("query")
            .expect("checkpoint present after rebuild");
        assert_eq!(rebuilt.commit_ref, created.commit_ref);
        assert_eq!(rebuilt.content_hash, created.content_hash);
        assert_eq!(rebuilt.workspace_root, created.projection.workspace_root);
        assert_eq!(rebuilt.shadow_git_dir, created.projection.shadow_git_dir);

        // A NEW controller over the rebuilt state can still restore the
        // pre-restart checkpoint, returning the workspace to the prior state.
        let controller2 = FakeBoundaryController::open(ProjectId::new("project-capo"), &state_root)
            .expect("controller after restart");
        write_file(&workspace, "doc.txt", "post-restart-edit\n");
        controller2
            .restore_checkpoint(&scope, &created.checkpoint_id)
            .expect("io")
            .expect("restore after restart ok");
        assert_eq!(
            read_file(&workspace, "doc.txt").as_deref(),
            Some("pre-restart\n"),
            "checkpoint taken before restart is still restorable after"
        );
    }

    /// SG8: re-checkpointing the SAME pre-write tree for the same turn is
    /// idempotent (same id, no duplicate event/row).
    #[test]
    fn sg8_recheckpoint_same_tree_is_idempotent() {
        let (controller, workspace, shadow, _state_root) = fixture();
        write_file(&workspace, "f.txt", "stable\n");
        let scope = scope(&workspace, &shadow, "turn-1");
        let first = controller
            .create_checkpoint(&scope)
            .expect("io")
            .expect("ok");
        let second = controller
            .create_checkpoint(&scope)
            .expect("io")
            .expect("ok");
        assert_eq!(first.checkpoint_id, second.checkpoint_id);
        assert!(second.already_checkpointed, "second is a re-checkpoint");

        // Exactly one checkpoint row for the run.
        let all = controller
            .checkpoints_for_run(&scope.run_id)
            .expect("list checkpoints");
        assert_eq!(all.len(), 1, "idempotent re-checkpoint did not duplicate");
    }

    /// SG8: the create event and restore event are recorded as auditable events,
    /// and the projection's `restored_at` is stamped after a restore.
    #[test]
    fn sg8_create_and_restore_are_auditable_events() {
        let (controller, workspace, shadow, _state_root) = fixture();
        write_file(&workspace, "f.txt", "x\n");
        let scope = scope(&workspace, &shadow, "turn-1");
        let created = controller
            .create_checkpoint(&scope)
            .expect("io")
            .expect("ok");

        // Before restore: projection has no restored_at.
        let before = controller
            .checkpoint(&created.checkpoint_id)
            .expect("query")
            .expect("present");
        assert!(!before.is_restored());

        controller
            .restore_checkpoint(&scope, &created.checkpoint_id)
            .expect("io")
            .expect("ok");

        // After restore: restored_at is stamped on the same row.
        let after = controller
            .checkpoint(&created.checkpoint_id)
            .expect("query")
            .expect("present");
        assert!(after.is_restored(), "restored_at stamped after restore");

        // Both events are on the log for the session.
        let events = controller
            .state()
            .recent_events_for_session(&scope.session_id, 1000)
            .expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::CheckpointCreated.as_str()),
            "checkpoint.created event recorded"
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::CheckpointRestored.as_str()),
            "checkpoint.restored event recorded"
        );
    }

    /// SG8: restoring an unknown checkpoint is a typed error, not a panic, and
    /// touches nothing.
    #[test]
    fn sg8_restore_unknown_checkpoint_is_typed_error() {
        let (controller, workspace, shadow, _state_root) = fixture();
        let scope = scope(&workspace, &shadow, "turn-1");
        let outcome = controller
            .restore_checkpoint(&scope, "checkpoint-does-not-exist")
            .expect("io");
        let error = outcome.expect_err("unknown checkpoint must be a typed error");
        assert!(error.agent_message().contains("unknown checkpoint"));
    }

    /// SG8: the shadow-git mechanism never touches the workspace's own `.git` --
    /// across BOTH create AND the destructive restore (`git clean -fdx`), which is
    /// the operation where a `.git`-deletion risk would actually live.
    ///
    /// This pins git's top-level `.git` protection (a `checkout`/`clean` over a
    /// work tree never removes the top-level `.git`) as a regression guard against
    /// the real destructive path, and documents the handling of a NESTED `.git`
    /// (not special-cased by git): a nested sub-repo's `.git` IS cleaned by
    /// `clean -fdx` because it is untracked content under the work tree. The
    /// workspace's OWN top-level `.git` survives regardless.
    #[test]
    fn sg8_shadow_git_does_not_touch_workspace_dot_git() {
        let (controller, workspace, shadow, _state_root) = fixture();
        // Give the workspace its own real top-level git repo with a sentinel,
        // plus a NESTED sub-repo `.git` (which git does NOT top-level-protect).
        let user_git = workspace.join(".git");
        std::fs::create_dir_all(&user_git).expect("user .git");
        std::fs::write(user_git.join("SENTINEL"), "do-not-touch\n").expect("sentinel");
        let nested = workspace.join("vendor").join("sub");
        let nested_git = nested.join(".git");
        std::fs::create_dir_all(&nested_git).expect("nested .git");
        std::fs::write(nested_git.join("SENTINEL"), "nested\n").expect("nested sentinel");
        write_file(&workspace, "f.txt", "v1\n");

        let scope = scope(&workspace, &shadow, "turn-1");
        let created = controller
            .create_checkpoint(&scope)
            .expect("io")
            .expect("ok");

        // After CREATE: the shadow git dir lives under the shadow root, NOT in
        // the workspace's own `.git`, and the user's top-level sentinel survives.
        assert!(
            shadow.exists(),
            "shadow git root is created under the controller-owned root"
        );
        assert_eq!(
            std::fs::read_to_string(user_git.join("SENTINEL"))
                .ok()
                .as_deref(),
            Some("do-not-touch\n"),
            "workspace's own .git is never touched by the shadow repo create"
        );

        // Mutate the workspace, then RESTORE -- exercising the destructive
        // `git checkout --force` + `git clean -fdx` path over a work tree that
        // contains the workspace's own `.git`.
        write_file(&workspace, "added_after.txt", "remove me\n");
        write_file(&workspace, "f.txt", "v2\n");
        controller
            .restore_checkpoint(&scope, &created.checkpoint_id)
            .expect("io")
            .expect("restore ok");

        // The most dangerous operation (clean -fdx) did NOT remove the
        // workspace's own top-level `.git`: git special-cases the top-level
        // `.git` during checkout+clean, and we lock that in here.
        assert_eq!(
            std::fs::read_to_string(user_git.join("SENTINEL"))
                .ok()
                .as_deref(),
            Some("do-not-touch\n"),
            "workspace's own top-level .git survives the destructive restore (checkout + clean -fdx)"
        );
        // The checkpointed content is restored and post-checkpoint files removed.
        assert_eq!(read_file(&workspace, "f.txt").as_deref(), Some("v1\n"));
        assert!(
            read_file(&workspace, "added_after.txt").is_none(),
            "files added after the checkpoint are removed by restore"
        );
        // Documented behavior for a NESTED `.git` (git does not top-level-protect
        // it): it is untracked content under the work tree, so `clean -fdx`
        // removes it during restore. This is intentional and asserted so the
        // behavior is locked in rather than silently relied upon.
        assert!(
            !nested_git.exists(),
            "a nested sub-repo .git is untracked content and is removed by restore's clean -fdx"
        );
    }
}
