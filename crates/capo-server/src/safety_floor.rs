//! RTL6: the minimal safety floor for the first real workspace-write.
//!
//! `real-turn-loop` ships a live workspace-WRITE adapter in phase 1 while full
//! `PermissionPolicy`/`VerificationRunner`/shadow-git land later in
//! `safety-gates`. A minimal floor must therefore exist the moment the first
//! real write does, making that write **confined**, **reversible**, and **never
//! unattended**:
//!
//! - Confinement: the write target is confined to the workspace root via the
//!   shared path-containment engine ([`capo_tools::confine_write_path`], which
//!   reuses the runtime tool wrappers' `ensure_under_workspace`). A write
//!   outside the confined workspace is rejected *before any process is spawned*.
//! - Reversibility: a pre-write shadow-git checkpoint of the confined workspace
//!   is captured (via the controller-owned mechanism, see the SG8 UPGRADE note
//!   below) and recorded via a `checkpoint.created` event, so any RTL live write
//!   is reversible by one documented restore command.
//! - Never unattended: diff-preview/dry-run is the DEFAULT. A live write
//!   requires an explicit opt-in env gate (mirroring `CAPO_SERVER_RUN_CODEX_LIVE`)
//!   AND must not be unattended.
//!
//! The controller-owned hard kill ([`CapoServer::hard_kill_run`]) terminates the
//! run's process group mid-run (reusing the runtime process-group kill path) and
//! records the abort as a `run.hard_killed` event.
//!
//! Out of scope here (kept in `safety-gates`): full `PermissionPolicy`
//! enforcement and the `VerificationRunner`. The per-run resource ceiling and
//! its `run.aborted` event are RTL7; orphan reaping
//! (`run.orphaned`/`run.recovered`) is RTL10.
//!
//! SG8 UPGRADE: the pre-write checkpoint is no longer a directory copy under the
//! artifact root. The floor now delegates to the controller-owned shadow-git
//! checkpoint (`capo_controller::FakeBoundaryController::create_checkpoint`), so
//! there is exactly ONE checkpoint mechanism and ONE `checkpoint.created` payload
//! contract across the floor and the loop, and the pre-write snapshot is
//! restorable per-turn and survives a restart (the directory copy was neither).

use std::path::{Path, PathBuf};

use capo_controller::{CheckpointError as ControllerCheckpointError, CheckpointScope};
use capo_core::{AgentId, CommandIntent, CommandTarget, RunId, SessionId, TaskId, TurnId};
use capo_runtime::{LocalProcessRunner, LocalRunningProcess};
use capo_state::{EventKind, NewEvent, RedactionState};
use capo_tools::confine_write_path;

use crate::util::{command_identity_hash, stable_hash};
use crate::{CapoServer, ServerClientOrigin, ServerError, ServerResult};

/// The synthetic task/agent identity a floor-driven (RTL6) workspace-write
/// checkpoint is scoped to.
///
/// The floor's [`RunTurnRef`] carries only `(session, run, turn)` -- the
/// load-bearing scope for finding and restoring a checkpoint, which is keyed by
/// `checkpoint_id`. The controller's [`CheckpointScope`] also stamps a task/agent
/// on the `checkpoint.created` event for audit attribution; the floor stamps
/// these stable synthetic ids so a floor checkpoint is distinguishable on the log
/// from a loop checkpoint while still funnelling through the SAME shadow-git path.
const FLOOR_CHECKPOINT_TASK_ID: &str = "task-workspace-write-floor";
const FLOOR_CHECKPOINT_AGENT_ID: &str = "agent-workspace-write-floor";

/// The env gate that opts a workspace-write turn into a real live write.
///
/// Without it (the default), the write path stays diff-preview/dry-run.
pub const LIVE_WRITE_OPT_IN_ENV: &str = "CAPO_SERVER_RUN_CODEX_LIVE";

/// Whether the write adapter previews the diff (default) or applies it live.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteMode {
    /// Diff-preview/dry-run. No edit is applied. This is the DEFAULT.
    DryRun,
    /// Live write: an edit is applied inside the confined, checkpointed
    /// workspace. Reached only with an explicit opt-in AND attended execution.
    LiveWrite,
}

impl WriteMode {
    pub fn is_dry_run(self) -> bool {
        matches!(self, Self::DryRun)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry_run_diff_preview",
            Self::LiveWrite => "live_write",
        }
    }
}

/// Resolve the write mode for a workspace-write turn.
///
/// A live write requires BOTH an explicit caller opt-in (`live_execution_opt_in`,
/// the same opt-in the live-provider execution gate already requires) AND the
/// process env gate ([`LIVE_WRITE_OPT_IN_ENV`]) set to `1`, AND the run must be
/// attended (`unattended == false`). Anything short of all three falls back to
/// the dry-run/diff-preview default. Unattended continuation is `goal-autonomy`
/// only, on the `safety-gates` substrate -- it must never reach a live write
/// here.
pub fn resolve_write_mode(live_execution_opt_in: bool, unattended: bool) -> WriteMode {
    let env_opt_in = std::env::var(LIVE_WRITE_OPT_IN_ENV).as_deref() == Ok("1");
    resolve_write_mode_with_env(live_execution_opt_in, env_opt_in, unattended)
}

/// Pure write-mode decision with the env gate injected as a bool.
///
/// [`resolve_write_mode`] reads the process env gate and delegates here; tests
/// (and the injectable [`CapoServer::run_workspace_write_turn_with_env_gate`]
/// seam) call this directly so the live-write decision -- and the live arm of
/// `run_workspace_write_turn` that takes the checkpoint -- can be exercised
/// deterministically without mutating process-global env.
pub fn resolve_write_mode_with_env(
    live_execution_opt_in: bool,
    env_opt_in: bool,
    unattended: bool,
) -> WriteMode {
    if live_execution_opt_in && env_opt_in && !unattended {
        WriteMode::LiveWrite
    } else {
        WriteMode::DryRun
    }
}

/// A single pre-write checkpoint of a confined workspace, taken via the
/// controller-owned shadow-git mechanism (SG8).
///
/// The checkpoint is a commit in a per-workspace shadow `.git` (a separate
/// `GIT_DIR` under the controller's state root; the workspace's own `.git` is
/// never touched). The restorable ref is the shadow commit SHA. It is reversible
/// by one documented command: [`Self::restore_command`] returns that command, and
/// [`CapoServer::restore_pre_write_checkpoint`] performs it programmatically
/// (through the SAME controller `restore_checkpoint` path the loop uses). The
/// checkpoint is recorded via a `checkpoint.created` event + a durable
/// `CheckpointProjection`, so it survives restart/replay and is restorable
/// per-turn -- which the prior directory-copy snapshot was not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCheckpoint {
    pub checkpoint_id: String,
    pub workspace_root: PathBuf,
    /// The per-workspace shadow `.git` directory the checkpoint commit lives in.
    pub shadow_git_dir: PathBuf,
    /// The shadow-repo commit SHA the checkpoint is restorable to.
    pub commit_ref: String,
    /// The shadow commit's tree SHA -- a content fingerprint of the checkpointed
    /// workspace.
    pub content_hash: String,
}

impl WorkspaceCheckpoint {
    /// The single documented command that restores the workspace to its
    /// pre-write state. Recorded in the event payload and asserted by tests.
    ///
    /// This is the shadow-git restore: check the checkpoint commit's tree out
    /// over the workspace, then remove files added after the checkpoint, with
    /// `GIT_DIR`/`GIT_WORK_TREE` pinned so the workspace's own `.git` is never
    /// consulted.
    pub fn restore_command(&self) -> String {
        format!(
            "GIT_DIR={git_dir} GIT_WORK_TREE={workspace} git checkout --force {commit} -- . && \
             GIT_DIR={git_dir} GIT_WORK_TREE={workspace} git clean -fdx",
            git_dir = self.shadow_git_dir.display(),
            workspace = self.workspace_root.display(),
            commit = self.commit_ref,
        )
    }
}

/// The `(session, run, turn)` identity a safety-floor event is keyed to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunTurnRef<'a> {
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub turn_id: &'a str,
}

/// One workspace-write turn's safety-floor inputs.
pub struct WorkspaceWriteRequest<'a> {
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub turn_id: &'a str,
    pub workspace_root: &'a str,
    pub artifact_root: &'a str,
    /// The path the write would touch (confined before anything runs).
    pub write_target: &'a str,
    /// The caller's explicit live-write opt-in (mirrors the live-provider
    /// execution opt-in). Required, but not sufficient, for a live write.
    pub live_execution_opt_in: bool,
    /// Whether this turn is running unattended. Unattended turns can never reach
    /// a live write here; that is `goal-autonomy` territory.
    pub unattended: bool,
}

/// What the safety floor decided for a workspace-write turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceWriteOutcome {
    /// The confined, canonical write target. Resolved before any process runs.
    pub confined_write_target: PathBuf,
    /// Dry-run by default; live only with opt-in + env gate + attended.
    pub write_mode: WriteMode,
    /// The pre-write checkpoint, present only when the floor reached a live
    /// write (a dry run touches nothing, so no checkpoint is taken).
    pub checkpoint: Option<WorkspaceCheckpoint>,
}

impl CapoServer {
    /// Drive one workspace-write turn through the RTL6 safety floor.
    ///
    /// Resolves the write mode (dry-run/diff-preview by default) and then hands
    /// off to [`Self::confine_and_checkpoint_for_write`], the single confine ->
    /// checkpoint-on-live sequence the live spawn path
    /// (`run_live_provider_local`) also drives. The actual provider spawn is
    /// RTL9; the safety-floor ordering lives in that one shared method so the
    /// two paths cannot drift.
    pub fn run_workspace_write_turn(
        &self,
        origin: &ServerClientOrigin,
        request: WorkspaceWriteRequest<'_>,
    ) -> ServerResult<WorkspaceWriteOutcome> {
        let env_opt_in = std::env::var(LIVE_WRITE_OPT_IN_ENV).as_deref() == Ok("1");
        self.run_workspace_write_turn_with_env_gate(origin, request, env_opt_in)
    }

    /// Same as [`Self::run_workspace_write_turn`] but with the live-write env
    /// gate injected as a bool, so the live-write arm (the only branch that takes
    /// the pre-write checkpoint) can be exercised deterministically in tests
    /// without mutating the process-global `CAPO_SERVER_RUN_CODEX_LIVE` env. The
    /// public entry point reads the env gate and delegates here.
    pub fn run_workspace_write_turn_with_env_gate(
        &self,
        origin: &ServerClientOrigin,
        request: WorkspaceWriteRequest<'_>,
        env_opt_in: bool,
    ) -> ServerResult<WorkspaceWriteOutcome> {
        // Dry-run/diff-preview is the default. A live write needs the opt-in,
        // the env gate, AND an attended run.
        let write_mode = resolve_write_mode_with_env(
            request.live_execution_opt_in,
            env_opt_in,
            request.unattended,
        );
        // Hand off to the ONE confine -> checkpoint-on-live sequence that the
        // live spawn path (`run_live_provider_local`) also drives, so the
        // safety-floor ordering lives in exactly one place.
        self.confine_and_checkpoint_for_write(
            origin,
            RunTurnRef {
                session_id: request.session_id,
                run_id: request.run_id,
                turn_id: request.turn_id,
            },
            request.workspace_root,
            request.artifact_root,
            request.write_target,
            write_mode,
        )
    }

    /// The single confine -> checkpoint-on-live sequence every workspace-write
    /// turn passes through, whatever resolved the [`WriteMode`].
    ///
    /// Order is load-bearing: CONFINE first (a write that escapes the workspace
    /// is rejected here, before any process is spawned), THEN -- only on a real
    /// live write -- capture the pre-write checkpoint that makes the write
    /// reversible. A dry run touches nothing, so it takes no checkpoint. Both
    /// [`Self::run_workspace_write_turn_with_env_gate`] (which resolves the write
    /// mode itself) and the live spawn arm (which receives an already-resolved
    /// write mode) call THIS method, so the sequencing exists in exactly one
    /// place and cannot drift between the two paths.
    pub fn confine_and_checkpoint_for_write(
        &self,
        origin: &ServerClientOrigin,
        run_turn: RunTurnRef<'_>,
        workspace_root: &str,
        artifact_root: &str,
        write_target: &str,
        write_mode: WriteMode,
    ) -> ServerResult<WorkspaceWriteOutcome> {
        // 1. Confinement, before anything runs.
        let confined_write_target = self.confine_workspace_write(workspace_root, write_target)?;

        // 2. Only a live write touches the workspace, so only a live write needs
        //    the reversibility checkpoint -- taken BEFORE the write.
        let checkpoint = match write_mode {
            WriteMode::DryRun => None,
            WriteMode::LiveWrite => Some(self.create_pre_write_checkpoint(
                origin,
                run_turn,
                workspace_root,
                artifact_root,
            )?),
        };

        Ok(WorkspaceWriteOutcome {
            confined_write_target,
            write_mode,
            checkpoint,
        })
    }

    /// Confine a workspace-write target to the workspace root before any process
    /// runs.
    ///
    /// Wires the shared path-containment engine: the workspace must exist and be
    /// the confined boundary, and `write_target` (absolute or workspace-relative,
    /// possibly not-yet-created) must resolve under it. A target that escapes
    /// the workspace (via `..`, a symlinked prefix, or an unrelated absolute
    /// path) is rejected here -- the caller has not spawned a process yet.
    pub fn confine_workspace_write(
        &self,
        workspace_root: &str,
        write_target: &str,
    ) -> ServerResult<PathBuf> {
        let root = Path::new(workspace_root);
        confine_write_path(Path::new(write_target), root).map_err(|reason| {
            ServerError::AdapterFixture(format!(
                "workspace-write confinement rejected `{write_target}`: {reason}"
            ))
        })
    }

    /// Capture and record a single pre-write checkpoint of the confined
    /// workspace.
    ///
    /// SG8: this delegates to the controller-owned shadow-git checkpoint
    /// (`FakeBoundaryController::create_checkpoint`) -- the SAME mechanism the
    /// loop uses -- rather than the prior directory copy under the artifact root.
    /// The workspace is confined again here (defense in depth), then a commit is
    /// taken in a per-workspace shadow `.git` under the controller's state root
    /// (the workspace's own `.git` is never touched). A `checkpoint.created`
    /// event + durable `CheckpointProjection` record the restorable commit SHA,
    /// so any later live write is reversible per-turn and the checkpoint survives
    /// restart. The checkpoint id is keyed on `(run, turn, content tree SHA)`, so
    /// re-capturing the same pre-write state is idempotent.
    ///
    /// `artifact_root` is unused by the shadow-git path (the shadow repo lives
    /// under the controller state root, not the artifact root) but is retained on
    /// the signature for the floor's confine-the-artifact-root defense in depth
    /// and so the call sites do not churn.
    pub fn create_pre_write_checkpoint(
        &self,
        _origin: &ServerClientOrigin,
        run_turn: RunTurnRef<'_>,
        workspace_root: &str,
        artifact_root: &str,
    ) -> ServerResult<WorkspaceCheckpoint> {
        let RunTurnRef {
            session_id,
            run_id,
            turn_id,
        } = run_turn;
        let confined_workspace = self.confine_workspace_write(workspace_root, workspace_root)?;
        // Defense in depth: reject an artifact root that escapes via `..`, even
        // though the shadow repo no longer lives under it.
        if Path::new(artifact_root)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(ServerError::AdapterFixture(format!(
                "checkpoint artifact root rejected: {artifact_root}"
            )));
        }

        let scope = CheckpointScope {
            task_id: TaskId::new(FLOOR_CHECKPOINT_TASK_ID),
            agent_id: AgentId::new(FLOOR_CHECKPOINT_AGENT_ID),
            session_id: SessionId::new(session_id),
            run_id: RunId::new(run_id),
            turn_id: TurnId::new(turn_id),
            workspace_root: confined_workspace.display().to_string(),
            shadow_git_root: self.controller.shadow_git_root().display().to_string(),
        };
        let created = self
            .controller
            .create_checkpoint(&scope)
            .map_err(ServerError::State)?
            .map_err(server_checkpoint_error)?;

        Ok(WorkspaceCheckpoint {
            checkpoint_id: created.checkpoint_id,
            workspace_root: confined_workspace,
            shadow_git_dir: PathBuf::from(created.projection.shadow_git_dir),
            commit_ref: created.commit_ref,
            content_hash: created.content_hash,
        })
    }

    /// SG8: restore a pre-write checkpoint by id -- the floor's `Restore` command.
    ///
    /// Delegates to the controller's `restore_checkpoint` (the SAME path the loop
    /// uses), so the floor and the loop share one restore mechanism. Returns the
    /// workspace to the exact state captured by the checkpoint's shadow commit and
    /// records an auditable `checkpoint.restored` event. The checkpoint is read
    /// back from the durable projection, so this works after a restart.
    pub fn restore_pre_write_checkpoint(
        &self,
        run_turn: RunTurnRef<'_>,
        workspace_root: &str,
        checkpoint_id: &str,
    ) -> ServerResult<()> {
        let RunTurnRef {
            session_id,
            run_id,
            turn_id,
        } = run_turn;
        let scope = CheckpointScope {
            task_id: TaskId::new(FLOOR_CHECKPOINT_TASK_ID),
            agent_id: AgentId::new(FLOOR_CHECKPOINT_AGENT_ID),
            session_id: SessionId::new(session_id),
            run_id: RunId::new(run_id),
            turn_id: TurnId::new(turn_id),
            workspace_root: workspace_root.to_string(),
            shadow_git_root: self.controller.shadow_git_root().display().to_string(),
        };
        self.controller
            .restore_checkpoint(&scope, checkpoint_id)
            .map_err(ServerError::State)?
            .map_err(server_checkpoint_error)?;
        Ok(())
    }

    /// Controller-owned hard kill of a live run.
    ///
    /// Terminates the run's process group mid-run (reusing the runtime's
    /// process-group kill path) and records the abort as a `run.hard_killed`
    /// event. This is the floor's emergency stop -- distinct from the RTL7
    /// resource-ceiling `run.aborted` and from the RTL10 orphan recovery.
    pub fn hard_kill_run(
        &self,
        origin: &ServerClientOrigin,
        runner: &LocalProcessRunner,
        process: &mut LocalRunningProcess,
        run_turn: RunTurnRef<'_>,
        reason: &str,
    ) -> ServerResult<()> {
        let RunTurnRef {
            session_id,
            run_id,
            turn_id,
        } = run_turn;
        let runtime_process_ref = process.process.runtime_process_ref.clone();
        let external_pid = process.process.external_pid;
        runner
            .kill_running_process_group(process)
            .map_err(|error| ServerError::AdapterFixture(format!("hard kill failed: {error:?}")))?;

        let session = SessionId::new(session_id);
        let run = RunId::new(run_id);
        let event = NewEvent {
            event_id: format!(
                "event-run-hard-killed-{}",
                stable_hash(format!("{run_id}:{turn_id}:{reason}").as_bytes())
            ),
            kind: EventKind::RunHardKilled,
            actor: origin.actor_id.clone(),
            project_id: Some(self.project_id.clone()),
            task_id: None,
            agent_id: None,
            session_id: Some(session.clone()),
            run_id: Some(run.clone()),
            turn_id: Some(turn_id.to_string()),
            item_id: Some(runtime_process_ref.clone()),
            payload_json: serde_json::json!({
                "runtime_process_ref": runtime_process_ref,
                "external_pid": external_pid,
                "kill_kind": "controller_hard_kill_process_group",
                "reason": reason,
                "status": "killed",
            })
            .to_string(),
            idempotency_key: Some(format!(
                "run-hard-killed:{}:{}:{}",
                self.project_id, run, turn_id
            )),
            redaction_state: RedactionState::Safe,
        };
        // Record the abort through a request-handled audit envelope so the kill
        // is attributable, then append the hard-kill event itself.
        let command_hash = command_identity_hash(format!("hard_kill_run:{run_id}:{turn_id}"));
        let command = self.command_envelope(
            &format!("hard-kill-{run_id}-{turn_id}"),
            origin,
            &command_hash,
            CommandTarget::Session(session),
            CommandIntent::InterruptSession,
            Some(reason.to_string()),
        );
        self.controller
            .state()
            .append_event(event, &[])
            .map_err(ServerError::State)?;
        self.record_server_request_handled(
            &command,
            origin,
            "hard_kill_run",
            None,
            Some(serde_json::json!({
                "run_id": run_id,
                "turn_id": turn_id,
                "kill_kind": "controller_hard_kill_process_group",
                "reason": reason,
            })),
        )
        .map_err(ServerError::State)?;
        Ok(())
    }
}

/// Map a controller [`CheckpointError`](ControllerCheckpointError) into the
/// server's error type, preserving the agent-readable message.
fn server_checkpoint_error(error: ControllerCheckpointError) -> ServerError {
    ServerError::AdapterFixture(format!(
        "shadow-git checkpoint failed: {}",
        error.agent_message()
    ))
}
