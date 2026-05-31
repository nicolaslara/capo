//! SG5: the controller-owned single-writer workspace lock (a session-scoped
//! write lease) -- the primitive + decide-style gate the real loop's write path
//! drives. SG5 builds the lock and its gate seam ([`FakeBoundaryController::
//! gate_workspace_write`]); `goal-autonomy` `GO8` is the consumer that wires it
//! as its "no conflicting workspace lock" continuation precondition. The
//! server's process-global `WriteSerializer` (`capo-server::transport`) remains
//! the ACTIVE in-process write serializer; this session-scoped lease is the
//! finer-grained primitive that path can later subsume, not a second serializer
//! running today.
//!
//! Three behaviors land here:
//!
//! 1. ACQUIRE ([`FakeBoundaryController::acquire_workspace_write_lease`]): a
//!    session takes the write lease for a workspace key (a collision-free
//!    encoding of the NORMALIZED workspace root -- see [`lease_key_segment`]).
//!    While no one holds it, the acquire succeeds and emits
//!    `workspace.lease_acquired`. While the SAME session already holds it, the
//!    re-acquire is idempotent. While ANOTHER session holds it, the acquire is
//!    REJECTED with a typed [`WorkspaceLockConflict`] -- it is never interleaved
//!    or silently queued.
//!
//! 2. GATE ([`FakeBoundaryController::gate_workspace_write`]): the decide-style
//!    seam a write path calls before a tool write/workspace mutation proceeds. A
//!    write is allowed
//!    only when the requesting session holds the lease (acquiring it first if
//!    free); a write from a session that is not the holder is denied with the
//!    same typed conflict. READS are not gated -- read-only tools pass through
//!    untouched, even while another session holds the write lease. SG5 builds
//!    and exercises this gate; `GO8` is the consumer that drives it from the
//!    live loop's write classification (SG5 does not itself rewrite
//!    `dispatch_tool_call`).
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

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};
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
    /// The workspace root the lease is keyed on. It is NORMALIZED (lexically:
    /// `.`/`..`/`//`/trailing-separator resolved) and then encoded
    /// collision-free into the lease key via [`lease_key_segment`], so the SAME
    /// directory spelled two ways (`/w/capo` vs `/w/x/../capo` vs `/w/capo/`)
    /// keys the SAME lease, and two genuinely distinct roots NEVER collide to
    /// one key. One single-writer lease exists per workspace key; two sessions
    /// over the same workspace contend for the same lease.
    ///
    /// Normalization is lexical, not `fs::canonicalize`: it does not resolve
    /// symlinks or require the path to exist on disk (the controller keys the
    /// lease before any write touches the filesystem). A caller that wants
    /// symlink identity should canonicalize before constructing the scope.
    pub workspace_root: String,
}

impl WorkspaceLeaseScope {
    /// The stable lease key for this workspace root, scoped to the project so two
    /// projects sharing a path string never collide.
    fn lease_id(&self, project_id: &ProjectId) -> String {
        format!(
            "workspace-lease-{project_id}-{}",
            lease_key_segment(&self.workspace_root)
        )
    }
}

/// Lexically normalize a workspace-root path: resolve `.`/`..`/`//` and strip a
/// trailing separator, WITHOUT touching the filesystem. So `/w/capo`,
/// `/w/capo/`, `/w/x/../capo`, and `/w/./capo` all normalize to `/w/capo`. A
/// leading `..` that would escape the root is kept verbatim (there is nothing to
/// pop), and a relative path stays relative.
fn normalize_workspace_root(workspace_root: &str) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in Path::new(workspace_root).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop the last NORMAL segment; keep `..` if there is nothing to
                // pop (a relative path that climbs above its start) or if the
                // prefix/root is all that precedes it.
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

/// Derive the COLLISION-FREE lease-key segment for a workspace root.
///
/// SG5 review-fix: the previous `slug(workspace_root)` was wrong for a security
/// boundary key -- `slug` is built for human-readable registration labels and
/// DROPS every non-alphanumeric char including the `/` separator, so
/// `/srv/a/b`, `/srv/ab`, and `/work/space/capo` all collapsed to one key
/// (false single-writer scoping / spurious conflicts), while the same root
/// spelled differently produced different keys. Here we instead lower-hex the
/// raw bytes of the normalized path, which is injective: two roots produce the
/// same segment IFF their normalized paths are byte-identical. The result is
/// ASCII-only (safe in the event id / DB key) and reversible, with no extra
/// dependency.
fn lease_key_segment(workspace_root: &str) -> String {
    let normalized = normalize_workspace_root(workspace_root);
    let bytes = normalized.as_os_str().as_encoded_bytes();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Infallible: writing to a String never errors.
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
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
    ///
    /// CONCURRENCY: this read-then-write is NOT atomic at the DB level -- the
    /// read-back and the `append_event` write run on separate connections with
    /// no `BEGIN IMMEDIATE` and no `status='held'` uniqueness constraint. It is
    /// safe ONLY because the transport serializes writers in-process (one
    /// handler call at a time -- see `capo-state::Store::connect`). Across
    /// independent processes this lock is NOT a hard mutual-exclusion guarantee;
    /// the SG9 liveness-aware recovery path is what reclaims a stale lease from a
    /// dead holder. Until that lands, treat this as in-process-serialized only.
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
                // SG5 review-fix: the event id MUST be unique per acquisition.
                // It previously was `event-lease-acquired-{lease}-{session}`,
                // deterministic per (lease, session) -- so an acquire -> release
                // -> re-acquire by the SAME session produced the identical
                // idempotency_key as the first acquire, and `append_event` hit
                // its idempotency early-return: it committed nothing, the HELD
                // projection was NOT re-applied, yet this function still returned
                // `Acquired` (phantom acquire breaking single-writer). Including
                // `acquired_at` (mirroring the release event id) makes each
                // acquisition's id distinct so the re-acquire actually re-holds.
                &format!(
                    "event-lease-acquired-{}-{}-{}",
                    lease_id, scope.session_id, acquired_at
                ),
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

    /// SG9: reclaim every held workspace lease whose holding run is in
    /// `dead_run_ids` (a run the liveness probe classified as exited/orphaned
    /// during restart recovery), emitting `workspace.lease_released` with a
    /// recovery reason so the freed lease is distinguishable from an explicit
    /// release. A live (reattached) holder's lease is left untouched.
    ///
    /// This reclaims directly from the durable lease projection (it does not need
    /// the original workspace-root string), so it works after a restart where the
    /// only durable handle is the rebuilt lease row. Returns the lease ids
    /// reclaimed. Idempotent: a lease already released is skipped, so a repeated
    /// recovery pass reclaims nothing further.
    pub fn reclaim_stale_workspace_leases(
        &self,
        dead_run_ids: &[RunId],
        reason: &str,
    ) -> StateResult<Vec<String>> {
        let mut reclaimed = Vec::new();
        for lease in self.state.workspace_leases(&self.project_id)? {
            if !lease.is_held() {
                continue;
            }
            let holder_is_dead = lease
                .holder_run_id
                .as_ref()
                .is_some_and(|run_id| dead_run_ids.contains(run_id));
            if !holder_is_dead {
                continue;
            }

            let released_at = epoch_millis().to_string();
            let mut released = lease.clone();
            released.status = WorkspaceLeaseProjection::RELEASED.to_string();
            released.released_at = Some(released_at.clone());
            released.release_reason = reason.to_string();
            released.updated_sequence = 0;

            let payload = serde_json::json!({
                "workspace_lease_id": lease.workspace_lease_id,
                "holder_session_id": lease.holder_session_id.as_str(),
                "holder_run_id": lease.holder_run_id.as_ref().map(RunId::as_str),
                "released_at": released_at,
                "reason": reason,
                "reclaimed_from_dead_holder": true,
            })
            .to_string();

            // Reclaim is event-sourced through the holder's own session/run
            // provenance (the dead holder). A STABLE event id + idempotency key
            // (no wall-clock) keep a repeated recovery pass over the same lease
            // from appending a second event.
            let event = NewEvent {
                event_id: format!("event-lease-reclaimed-{}", lease.workspace_lease_id),
                kind: EventKind::WorkspaceLeaseReleased,
                actor: "capo-recovery".to_string(),
                project_id: Some(self.project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(lease.holder_session_id.clone()),
                run_id: lease.holder_run_id.clone(),
                turn_id: None,
                item_id: Some(lease.workspace_lease_id.clone()),
                payload_json: payload,
                idempotency_key: Some(format!(
                    "recovery:lease-reclaim:{}",
                    lease.workspace_lease_id
                )),
                redaction_state: RedactionState::Safe,
            };

            self.state
                .append_event(event, &[ProjectionRecord::WorkspaceLease(released)])?;
            reclaimed.push(lease.workspace_lease_id);
        }
        Ok(reclaimed)
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
