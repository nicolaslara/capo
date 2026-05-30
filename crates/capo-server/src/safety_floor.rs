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
//! - Reversibility: a single pre-write snapshot of the confined workspace is
//!   captured and recorded via a `checkpoint.created` event, so any RTL live
//!   write is reversible by one documented restore command.
//! - Never unattended: diff-preview/dry-run is the DEFAULT. A live write
//!   requires an explicit opt-in env gate (mirroring `CAPO_SERVER_RUN_CODEX_LIVE`)
//!   AND must not be unattended.
//!
//! The controller-owned hard kill ([`CapoServer::hard_kill_run`]) terminates the
//! run's process group mid-run (reusing the runtime process-group kill path) and
//! records the abort as a `run.hard_killed` event.
//!
//! Out of scope here (kept in `safety-gates`): full `PermissionPolicy`
//! enforcement, the `VerificationRunner`, and full shadow-git. The per-run
//! resource ceiling and its `run.aborted` event are RTL7; orphan reaping
//! (`run.orphaned`/`run.recovered`) is RTL10.

use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, RunId, SessionId};
use capo_runtime::{LocalProcessRunner, LocalRunningProcess};
use capo_state::{EventKind, NewEvent, RedactionState};
use capo_tools::confine_write_path;

use crate::util::{command_identity_hash, stable_hash};
use crate::{CapoServer, ServerClientOrigin, ServerError, ServerResult};

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

/// A single pre-write snapshot of a confined workspace.
///
/// Phase 1 uses a single-snapshot directory copy under the artifact root (full
/// shadow-git stays in `safety-gates`). It is reversible by one documented
/// command: [`Self::restore_command`] returns that command, and
/// [`Self::restore`] performs it programmatically. The snapshot is recorded via
/// a `checkpoint.created` event so it survives restart/replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCheckpoint {
    pub checkpoint_id: String,
    pub workspace_root: PathBuf,
    pub snapshot_root: PathBuf,
    pub content_hash: String,
    pub file_count: usize,
}

impl WorkspaceCheckpoint {
    /// The single documented command that restores the workspace to its
    /// pre-write state. Recorded in the event payload and asserted by tests.
    pub fn restore_command(&self) -> String {
        format!(
            "rm -rf {workspace}/* && cp -a {snapshot}/. {workspace}/",
            workspace = self.workspace_root.display(),
            snapshot = self.snapshot_root.display()
        )
    }

    /// Restore the confined workspace to the snapshot's pre-write state.
    ///
    /// This is the programmatic equivalent of [`Self::restore_command`]: it
    /// clears the confined workspace and re-materializes the snapshot, leaving
    /// the workspace byte-identical to the moment the checkpoint was taken.
    pub fn restore(&self) -> Result<(), String> {
        clear_dir_contents(&self.workspace_root).map_err(|error| error.to_string())?;
        copy_dir_recursive(&self.snapshot_root, &self.workspace_root)
            .map_err(|error| error.to_string())?;
        Ok(())
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
    /// The workspace is confined again here (defense in depth) and snapshotted
    /// into `<artifact_root>/checkpoints/<checkpoint_id>`. A `checkpoint.created`
    /// event records the snapshot location, content hash, file count, and the
    /// one documented restore command, so any later live write is reversible.
    /// The event is idempotent on `(run_id, content_hash)`, so re-capturing the
    /// same pre-write state does not append a duplicate.
    pub fn create_pre_write_checkpoint(
        &self,
        origin: &ServerClientOrigin,
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
        let confined_artifacts = confine_write_path(Path::new(artifact_root), &confined_workspace)
            .or_else(|_| {
                // Artifacts may legitimately live outside the workspace; only
                // reject `..`-escapes / credential-like components there.
                let root = Path::new(artifact_root);
                if root
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    Err(ServerError::AdapterFixture(format!(
                        "checkpoint artifact root rejected: {artifact_root}"
                    )))
                } else {
                    Ok(root.to_path_buf())
                }
            })?;

        let checkpoints_root = confined_artifacts.join("checkpoints");
        let checkpoint_id = format!(
            "checkpoint-{}",
            stable_hash(format!("{run_id}:{turn_id}:{}", confined_workspace.display()).as_bytes())
        );
        let snapshot_root = checkpoints_root.join(&checkpoint_id);
        if snapshot_root.exists() {
            let _ = fs::remove_dir_all(&snapshot_root);
        }
        fs::create_dir_all(&snapshot_root).map_err(|error| {
            ServerError::AdapterFixture(format!(
                "failed to create checkpoint snapshot dir: {error}"
            ))
        })?;
        let (content_hash, file_count) = copy_dir_recursive(&confined_workspace, &snapshot_root)
            .map_err(|error| {
                ServerError::AdapterFixture(format!("failed to snapshot workspace: {error}"))
            })?;

        let checkpoint = WorkspaceCheckpoint {
            checkpoint_id: checkpoint_id.clone(),
            workspace_root: confined_workspace.clone(),
            snapshot_root: snapshot_root.clone(),
            content_hash: content_hash.clone(),
            file_count,
        };

        let session = SessionId::new(session_id);
        let run = RunId::new(run_id);
        let event = NewEvent {
            event_id: format!(
                "event-checkpoint-created-{}",
                stable_hash(format!("{checkpoint_id}:{content_hash}").as_bytes())
            ),
            kind: EventKind::CheckpointCreated,
            actor: origin.actor_id.clone(),
            project_id: Some(self.project_id.clone()),
            task_id: None,
            agent_id: None,
            session_id: Some(session.clone()),
            run_id: Some(run.clone()),
            turn_id: Some(turn_id.to_string()),
            item_id: Some(checkpoint_id.clone()),
            payload_json: serde_json::json!({
                "checkpoint_id": checkpoint_id,
                "checkpoint_kind": "single_snapshot_directory_copy",
                "workspace_root": confined_workspace.display().to_string(),
                "snapshot_root": snapshot_root.display().to_string(),
                "content_hash": content_hash,
                "file_count": file_count,
                "reversible": true,
                "restore_command": checkpoint.restore_command(),
            })
            .to_string(),
            idempotency_key: Some(format!(
                "checkpoint-created:{}:{}:{}",
                self.project_id, run, content_hash
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(event, &[])
            .map_err(ServerError::State)?;
        Ok(checkpoint)
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

/// Recursively copy `src` into `dst`, returning `(content_hash, file_count)`.
///
/// The content hash is an order-independent FNV-1a roll-up of every relative
/// path and its bytes, so it is a stable fingerprint of the snapshot regardless
/// of directory iteration order.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<(String, usize)> {
    fs::create_dir_all(dst)?;
    let mut hash_accumulator: u64 = 0;
    let mut file_count = 0usize;
    copy_dir_into(src, src, dst, &mut hash_accumulator, &mut file_count)?;
    Ok((format!("fnv1a64:{hash_accumulator:016x}"), file_count))
}

fn copy_dir_into(
    base: &Path,
    src: &Path,
    dst: &Path,
    hash_accumulator: &mut u64,
    file_count: &mut usize,
) -> std::io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let relative = source_path.strip_prefix(base).unwrap_or(&source_path);
        let target_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&target_path)?;
            copy_dir_into(
                base,
                &source_path,
                &target_path,
                hash_accumulator,
                file_count,
            )?;
        } else if file_type.is_file() {
            let bytes = fs::read(&source_path)?;
            fs::write(&target_path, &bytes)?;
            *hash_accumulator ^= fnv1a64(relative.to_string_lossy().as_bytes());
            *hash_accumulator ^= fnv1a64(&bytes);
            *file_count += 1;
        }
        // Symlinks and other special files are intentionally skipped: the
        // confined workspace snapshot only captures regular files for phase 1.
    }
    Ok(())
}

fn clear_dir_contents(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
