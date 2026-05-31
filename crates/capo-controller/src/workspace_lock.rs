//! SG5: the controller-owned single-writer workspace lock (a session-scoped
//! write lease) that gates every tool write and workspace mutation in the real
//! loop.
//!
//! Three behaviors land here:
//!
//! 1. ACQUIRE ([`FakeBoundaryController::acquire_workspace_write_lease`]): a
//!    session takes the write lease for a workspace key (the canonicalized
//!    workspace root). While no one holds it, the acquire succeeds and emits
//!    `workspace.lease_acquired`. While the SAME session already holds it, the
//!    re-acquire is idempotent. While ANOTHER session holds it, the acquire is
//!    REJECTED with a typed [`WorkspaceLockConflict`] -- it is never interleaved
//!    or silently queued.
//!
//! 2. GATE ([`FakeBoundaryController::gate_workspace_write`]): the loop calls
//!    this before any tool write/workspace mutation proceeds. A write is allowed
//!    only when the requesting session holds the lease (acquiring it first if
//!    free); a write from a session that is not the holder is denied with the
//!    same typed conflict. READS are not gated -- read-only tools pass through
//!    untouched, even while another session holds the write lease.
//!
//! 3. RELEASE ([`FakeBoundaryController::release_workspace_write_lease`]): the
//!    holder frees the lease, emitting `workspace.lease_released`; the next
//!    writer's acquire then succeeds. Release records a reason so a lease
//!    reclaimed from a dead holder during recovery (SG9) is distinguishable from
//!    an explicit release.
//!
//! Acquire/release is event-sourced (the lease lives in the
//! `WorkspaceLeaseProjection`, upserted by the two events), so the lock survives
//! restart and rebuilds identically from the event log. This is the primitive
//! `goal-autonomy` `GO8` consumes as its "no conflicting workspace lock"
//! continuation precondition: `GO8` names the lock, `safety-gates` builds it.

use std::time::{SystemTime, UNIX_EPOCH};

use capo_state::WorkspaceLeaseProjection;

use super::*;

/// Wall-clock millis-since-epoch, the instant a lease is acquired/released.
/// Clamped to 0 before the epoch. Shared shape with the SG3 grant lifecycle so
/// lease and grant timestamps compare on the same basis.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a workspace-lease acquire/release hangs on the loop's scope tree, so
/// the persisted `workspace.lease_*` events carry the same task/agent/session/
/// run/turn provenance the rest of the loop uses, and so the lock keys the lease
/// on the workspace root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLeaseScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    /// The canonicalized workspace root the lease is keyed on. One single-writer
    /// lease exists per workspace key; two sessions over the same workspace
    /// contend for the same lease.
    pub workspace_root: String,
}

impl WorkspaceLeaseScope {
    /// The stable lease key for this workspace root, scoped to the project so two
    /// projects sharing a path string never collide.
    fn lease_id(&self, project_id: &ProjectId) -> String {
        format!(
            "workspace-lease-{project_id}-{}",
            slug(&self.workspace_root)
        )
    }
}

/// SG5: a typed rejection when a write is requested while another session holds
/// the workspace write lease. Surfaced to the loop as a decide-style outcome (not
/// an error) so the loop can reflect on the conflict rather than crashing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLockConflict {
    /// The session that currently holds the lease and blocked this request.
    pub held_by_session_id: SessionId,
    /// The run that holds the lease, when one is associated.
    pub held_by_run_id: Option<RunId>,
    /// When the holder acquired the lease.
    pub held_since: Option<String>,
    /// The lease key the conflict is over.
    pub workspace_lease_id: String,
    /// A structured, agent-readable message the loop can reflect on.
    pub message: String,
}

impl WorkspaceLockConflict {
    /// The structured refusal message surfaced to the agent/loop.
    pub fn agent_message(&self) -> &str {
        &self.message
    }
}

/// SG5: the typed outcome of requesting the single-writer workspace write lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceWriteLeaseOutcome {
    /// The requesting session now holds the lease: it was free and a new
    /// `workspace.lease_acquired` event was emitted.
    Acquired { workspace_lease_id: String },
    /// The requesting session already held the lease; the re-acquire was
    /// idempotent and emitted no new event.
    AlreadyHeldBySelf { workspace_lease_id: String },
    /// Another session holds the lease, so the request was rejected. No event was
    /// emitted and no write may proceed.
    Conflict(WorkspaceLockConflict),
}

impl WorkspaceWriteLeaseOutcome {
    /// Whether the requesting session may proceed with a write after this
    /// outcome: true iff it now holds (or already held) the lease.
    pub fn may_write(&self) -> bool {
        matches!(self, Self::Acquired { .. } | Self::AlreadyHeldBySelf { .. })
    }

    /// The conflict, when the request was rejected.
    pub fn conflict(&self) -> Option<&WorkspaceLockConflict> {
        match self {
            Self::Conflict(conflict) => Some(conflict),
            _ => None,
        }
    }
}

/// SG5: the typed outcome of gating one tool/workspace operation through the
/// single-writer lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceWriteGate {
    /// A read-only operation: the write lease does not gate reads, so the
    /// operation proceeds regardless of who (if anyone) holds the lease.
    ReadAllowed,
    /// A write the requesting session is cleared to perform because it holds the
    /// lease (acquiring it here if it was free).
    WriteAllowed { outcome: WorkspaceWriteLeaseOutcome },
    /// A write rejected because another session holds the lease.
    WriteDenied(WorkspaceLockConflict),
}

impl WorkspaceWriteGate {
    /// Whether the gated operation may proceed.
    pub fn allowed(&self) -> bool {
        matches!(self, Self::ReadAllowed | Self::WriteAllowed { .. })
    }

    /// The conflict, when a write was denied.
    pub fn conflict(&self) -> Option<&WorkspaceLockConflict> {
        match self {
            Self::WriteDenied(conflict) => Some(conflict),
            _ => None,
        }
    }
}

impl FakeBoundaryController {
    /// SG5: acquire the single-writer workspace write lease for the scope's
    /// session.
    ///
    /// Read-back FIRST: if a lease already exists for the workspace key and is
    /// HELD by another session, the request is rejected with a typed
    /// [`WorkspaceLockConflict`] -- never interleaved or queued. If it is held by
    /// the SAME session, the acquire is idempotent ([`WorkspaceWriteLeaseOutcome::
    /// AlreadyHeldBySelf`], no new event). Otherwise (free, released, or never
    /// acquired) the lease is taken: a `workspace.lease_acquired` event is
    /// appended and the lease projection upserts to `held` with the holder/
    /// acquired_at stamped.
    pub fn acquire_workspace_write_lease(
        &self,
        scope: &WorkspaceLeaseScope,
    ) -> StateResult<WorkspaceWriteLeaseOutcome> {
        let lease_id = scope.lease_id(&self.project_id);
        if let Some(existing) = self.state.workspace_lease_by_id(&lease_id)?
            && existing.is_held()
        {
            if existing.holder_session_id == scope.session_id {
                return Ok(WorkspaceWriteLeaseOutcome::AlreadyHeldBySelf {
                    workspace_lease_id: lease_id,
                });
            }
            return Ok(WorkspaceWriteLeaseOutcome::Conflict(workspace_conflict(
                &existing, &lease_id,
            )));
        }

        let acquired_at = epoch_millis().to_string();
        let lease = WorkspaceLeaseProjection {
            workspace_lease_id: lease_id.clone(),
            project_id: self.project_id.clone(),
            holder_session_id: scope.session_id.clone(),
            holder_run_id: Some(scope.run_id.clone()),
            status: WorkspaceLeaseProjection::HELD.to_string(),
            acquired_at: Some(acquired_at.clone()),
            released_at: None,
            release_reason: String::new(),
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "workspace_lease_id": lease_id,
            "workspace_root": scope.workspace_root,
            "holder_session_id": scope.session_id.as_str(),
            "holder_run_id": scope.run_id.as_str(),
            "acquired_at": acquired_at,
        })
        .to_string();

        self.state.append_event(
            scoped_event(
                &format!("event-lease-acquired-{}-{}", lease_id, scope.session_id),
                EventKind::WorkspaceLeaseAcquired,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(lease_id.clone())
            .with_payload(payload),
            &[ProjectionRecord::WorkspaceLease(lease)],
        )?;

        Ok(WorkspaceWriteLeaseOutcome::Acquired {
            workspace_lease_id: lease_id,
        })
    }

    /// SG5: release the single-writer workspace write lease the scope's session
    /// holds, emitting `workspace.lease_released` with a reason.
    ///
    /// Re-emits the SAME lease row with `status = released`, `released_at`
    /// stamped, and the reason recorded; the next writer's acquire then succeeds.
    /// Releasing a lease held by a DIFFERENT session is rejected with a typed
    /// conflict (a session cannot release another session's lease); releasing a
    /// free/never-acquired lease is a no-op that records nothing. The `reason`
    /// distinguishes an explicit release from a lease reclaimed during recovery.
    pub fn release_workspace_write_lease(
        &self,
        scope: &WorkspaceLeaseScope,
        reason: &str,
    ) -> StateResult<WorkspaceWriteLeaseOutcome> {
        let lease_id = scope.lease_id(&self.project_id);
        let existing = match self.state.workspace_lease_by_id(&lease_id)? {
            Some(existing) if existing.is_held() => existing,
            // Nothing to release: free, already released, or never acquired.
            _ => {
                return Ok(WorkspaceWriteLeaseOutcome::AlreadyHeldBySelf {
                    workspace_lease_id: lease_id,
                });
            }
        };
        if existing.holder_session_id != scope.session_id {
            return Ok(WorkspaceWriteLeaseOutcome::Conflict(workspace_conflict(
                &existing, &lease_id,
            )));
        }

        let released_at = epoch_millis().to_string();
        let mut released = existing.clone();
        released.status = WorkspaceLeaseProjection::RELEASED.to_string();
        released.released_at = Some(released_at.clone());
        released.release_reason = reason.to_string();
        released.updated_sequence = 0;

        let payload = serde_json::json!({
            "workspace_lease_id": lease_id,
            "workspace_root": scope.workspace_root,
            "holder_session_id": scope.session_id.as_str(),
            "released_at": released_at,
            "reason": reason,
        })
        .to_string();

        self.state.append_event(
            scoped_event(
                &format!("event-lease-released-{}-{}", lease_id, released_at),
                EventKind::WorkspaceLeaseReleased,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(lease_id.clone())
            .with_payload(payload),
            &[ProjectionRecord::WorkspaceLease(released)],
        )?;

        Ok(WorkspaceWriteLeaseOutcome::Acquired {
            workspace_lease_id: lease_id,
        })
    }

    /// SG5: gate one tool/workspace operation through the single-writer lock.
    ///
    /// `is_write` is the loop's per-tool classification (an ACI write/edit/patch
    /// tool or any workspace mutation is a write; everything else is a read).
    ///
    /// - A READ ([`WorkspaceWriteGate::ReadAllowed`]) is never blocked by the
    ///   write lease, even while another session holds it.
    /// - A WRITE is allowed only when the requesting session holds the lease.
    ///   This acquires the lease for the session if it is free (so the first
    ///   writer of a turn takes it transparently), and rejects with a typed
    ///   [`WorkspaceLockConflict`] when another session already holds it.
    pub fn gate_workspace_write(
        &self,
        scope: &WorkspaceLeaseScope,
        is_write: bool,
    ) -> StateResult<WorkspaceWriteGate> {
        if !is_write {
            return Ok(WorkspaceWriteGate::ReadAllowed);
        }
        let outcome = self.acquire_workspace_write_lease(scope)?;
        match outcome {
            WorkspaceWriteLeaseOutcome::Conflict(conflict) => {
                Ok(WorkspaceWriteGate::WriteDenied(conflict))
            }
            allowed => Ok(WorkspaceWriteGate::WriteAllowed { outcome: allowed }),
        }
    }

    /// SG5: the current holder of the workspace write lease for the scope's
    /// workspace key, or `None` when the lease is free/never acquired/released.
    ///
    /// Reads the lease back from the durable projection (so it reflects a
    /// rebuild from the event log), exposed for the recovery path (SG9) to find a
    /// stale lease held by a dead run and for tests/inspection.
    pub fn workspace_lease_holder(
        &self,
        scope: &WorkspaceLeaseScope,
    ) -> StateResult<Option<WorkspaceLeaseProjection>> {
        let lease_id = scope.lease_id(&self.project_id);
        let lease = self
            .state
            .workspace_lease_by_id(&lease_id)?
            .filter(WorkspaceLeaseProjection::is_held);
        Ok(lease)
    }
}

/// Build the typed conflict surfaced when a write contends with a lease another
/// session holds.
fn workspace_conflict(
    existing: &WorkspaceLeaseProjection,
    lease_id: &str,
) -> WorkspaceLockConflict {
    WorkspaceLockConflict {
        held_by_session_id: existing.holder_session_id.clone(),
        held_by_run_id: existing.holder_run_id.clone(),
        held_since: existing.acquired_at.clone(),
        workspace_lease_id: lease_id.to_string(),
        message: format!(
            "workspace write lease is held by session {}; a second concurrent writer is rejected (single-writer lock)",
            existing.holder_session_id
        ),
    }
}
