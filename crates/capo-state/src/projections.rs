use capo_core::{
    AgentId, EvidenceId, GoalId, MemoryPacketId, ProjectId, RequirementId, RunId, SessionId,
    TaskId, ToolCallId,
};

use crate::RedactionState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionRecord {
    Project(ProjectProjection),
    Task(TaskProjection),
    Agent(AgentProjection),
    Session(SessionProjection),
    Run(RunProjection),
    CapabilityGrant(CapabilityGrantProjection),
    WorkspaceLease(WorkspaceLeaseProjection),
    PermissionApproval(PermissionApprovalProjection),
    ConnectivityExposure(ConnectivityExposureProjection),
    RuntimeTarget(RuntimeTargetProjection),
    AdapterReadiness(AdapterReadinessProjection),
    AdapterSmokeReport(AdapterSmokeReportProjection),
    AdapterDispatchPlan(AdapterDispatchPlanProjection),
    AdapterDispatchGate(AdapterDispatchGateProjection),
    AdapterDispatchReplay(AdapterDispatchReplayProjection),
    AdapterDispatchExecutionRequest(AdapterDispatchExecutionRequestProjection),
    AdapterDispatchExecution(AdapterDispatchExecutionProjection),
    AdapterDispatchPromptSource(AdapterDispatchPromptSourceProjection),
    AdapterDispatchPromptMaterialization(AdapterDispatchPromptMaterializationProjection),
    ToolCall(ToolCallProjection),
    ToolObservation(ToolObservationProjection),
    MemoryPacketRef(MemoryPacketProjection),
    MemoryRecord(Box<MemoryRecordProjection>),
    MemorySource(MemorySourceProjection),
    TaskOutcomeReport(TaskOutcomeReportProjection),
    ReviewFinding(ReviewFindingProjection),
    Evidence(EvidenceProjection),
    RunScore(RunScoreProjection),
    Checkpoint(CheckpointProjection),
    SourceBinding(SourceBindingProjection),
    WorkpadIndexReset(WorkpadIndexResetProjection),
    WorkpadFile(WorkpadFileProjection),
    WorkpadTask(WorkpadTaskProjection),
    // GA1 (goal-orchestration GO1/GO3): the goal-domain read models.
    Goal(GoalProjection),
    RequirementLedger(RequirementLedgerProjection),
    GoalReport(GoalReportProjection),
    GoalContinuation(GoalContinuationProjection),
    DelegatedProviderGoal(DelegatedProviderGoalProjection),
    GoalAuditDecision(GoalAuditDecisionProjection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProjection {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskProjection {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub capo_execution_status: String,
    pub active_session_id: Option<SessionId>,
    pub latest_summary: Option<String>,
    pub evidence_id: Option<EvidenceId>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProjection {
    pub agent_id: AgentId,
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub current_session_id: Option<SessionId>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionProjection {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: AgentId,
    pub title: String,
    pub status: String,
    pub current_goal: String,
    pub latest_summary: Option<String>,
    pub latest_confidence: Option<i64>,
    pub latest_blocker: Option<String>,
    /// Adapter-owned external session handle this session is bound to.
    ///
    /// First-class on the read model so adapter-neutral re-derivation (e.g.
    /// `refs_for_agent_name`) reads the real injected value instead of baking
    /// in a concrete adapter's naming convention.
    pub external_session_ref: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunProjection {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub status: String,
    pub recovery_of_run_id: Option<RunId>,
    pub updated_sequence: i64,
}

/// How a restart observed a previously in-flight run's process group, after
/// probing (and, if alive, reaping) it (RTL10).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunReapKind {
    /// The process group was still alive on restart and was reaped (Capo cannot
    /// reattach in phase 1, so a live orphan is terminated and recorded). The
    /// run is recorded as `run.orphaned` then a terminal `run.exited`.
    AliveReaped,
    /// The process was already gone and no terminal event existed. The run is
    /// recorded as a terminal `run.exited` with unknown exit detail.
    AlreadyGone,
    /// No process was ever spawned for the run (e.g. a deterministic/mock run
    /// that crashed before spawning), so there is nothing to reap. The run is
    /// recorded as a terminal `run.exited` with unknown exit detail.
    NoProcess,
}

impl RunReapKind {
    pub const fn observation_kind(self) -> &'static str {
        match self {
            Self::AliveReaped => "alive_reaped",
            Self::AlreadyGone => "already_gone",
            Self::NoProcess => "no_process",
        }
    }
}

/// One run's reap observation, produced by the recovery layer after it probed
/// (and possibly reaped) the persisted process group via the runtime, and
/// consumed by [`crate::SqliteStateStore::reap_orphaned_runs`] to emit the
/// recovery events idempotently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReapObservation {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub previous_status: String,
    pub kind: RunReapKind,
    pub external_pid: Option<u32>,
    /// A stable hash over the observed runtime state, part of the recovery
    /// idempotency key so repeated restarts that observe the same state never
    /// emit a second recovery event.
    pub observed_runtime_state_hash: String,
}

/// SG9: how a restart's LIVENESS-AWARE probe classified a previously in-flight
/// run, replacing the blunt path that marked every live-looking run
/// `exited_unknown`.
///
/// Unlike [`RunReapKind`] (the RTL10 phase-1 reaper, which KILLS a live orphan),
/// this classification distinguishes a still-alive REATTACHABLE run (recovered,
/// left running) from a still-alive non-attachable run (orphaned) and a run that
/// terminated while Capo was down (exited).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunRecoveryKind {
    /// The run's process group is still alive AND Capo holds an attachable handle
    /// for it, so recovery REATTACHES to it in place (the run keeps running). The
    /// run is recorded `run.recovered`; this is distinct from relaunching a fresh
    /// run with `recovery_of_run_id`.
    Reattached,
    /// The run's process group is still alive but Capo has no attachable handle
    /// (e.g. the in-flight marker recorded a PID but no runtime process ref), so
    /// the live process is an unowned orphan. The run is recorded `run.orphaned`.
    Orphaned,
    /// The run terminated while Capo was down (the process group is gone, or its
    /// boot id could not be confirmed against the current boot, or no process was
    /// ever spawned). The run is recorded with a terminal `run.exited`.
    Exited,
}

impl RunRecoveryKind {
    /// The stable observation kind folded into the recovery idempotency key.
    pub const fn observation_kind(self) -> &'static str {
        match self {
            Self::Reattached => "reattached",
            Self::Orphaned => "orphaned",
            Self::Exited => "exited",
        }
    }

    /// The terminal/recovered run STATUS the reconciled `Run` projection carries
    /// AFTER the full recovery event sequence is applied.
    ///
    /// This must match the durable, rebuilt-from-log projection (the runs row uses
    /// last-write-wins on `status`):
    /// - `Reattached` emits a single `run.recovered` -> `recovered`.
    /// - `Orphaned` emits `run.orphaned` then `run.exited` then `run.recovered`,
    ///   so the run ends `recovered` (NOT the transient `orphaned`); returning
    ///   `orphaned` here would diverge from the rebuilt projection.
    /// - `Exited` emits `run.exited` then `run.recovered` -> `recovered`.
    pub const fn run_status(self) -> &'static str {
        match self {
            Self::Reattached | Self::Orphaned | Self::Exited => "recovered",
        }
    }
}

/// SG9: one run's LIVENESS-AWARE recovery observation, produced by the controller
/// after it probed the persisted process group via the runtime
/// (`RuntimeRunner` health probe) WITHOUT killing it, and consumed by
/// [`crate::SqliteStateStore::recover_inflight_runs`] to emit the
/// `run.recovered` / `run.orphaned` / `run.exited` events idempotently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecoveryObservation {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub previous_status: String,
    pub kind: RunRecoveryKind,
    pub external_pid: Option<u32>,
    /// The attachable runtime handle the spawn persisted, when one exists. A
    /// reattached run reattaches by this ref in place; its presence is what
    /// distinguishes a `Reattached` live run from an `Orphaned` one.
    pub runtime_process_ref: Option<String>,
    /// A stable hash over the observed runtime state, part of the recovery
    /// idempotency key so repeated restarts that observe the same state never
    /// emit a second recovery event.
    pub observed_runtime_state_hash: String,
}

/// A run that looked live at startup, paired with the PID/process-group
/// reference its spawning side persisted *before* the spawn returned (RTL10).
///
/// This is the durable in-flight handle the orphan reaper uses on restart:
/// Capo no longer owns the `Child`, so the persisted `external_pid` is the only
/// way to probe and reap the orphaned process group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InFlightRun {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub status: String,
    /// The PID persisted before the spawn returned, if one was recorded. A run
    /// with no persisted PID (e.g. a deterministic/mock run that never spawned a
    /// process) reaps as "no process to reap".
    pub external_pid: Option<u32>,
    /// The machine boot id observed when the process was spawned, persisted
    /// alongside the PID. Restart recovery only reaps the process group when this
    /// matches the current boot id, so a PID/PGID recycled after a reboot is
    /// never signalled. `None` when the boot id was unreadable at spawn time.
    pub boot_id: Option<String>,
    pub runtime_process_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityGrantProjection {
    pub capability_grant_id: String,
    pub capability_profile_id: String,
    pub scope_json: String,
    pub effect: String,
    pub subject_json: String,
    pub decision_source: String,
    pub persistence: String,
    pub explanation: String,
    /// SG3: when the grant was created. `None` for grants created before the
    /// lifecycle timestamp columns existed (back-compat default).
    pub created_at: Option<String>,
    /// SG3: when the grant expires, if it has a bounded lifetime
    /// (`persistence = until_time`). A grant past `expires_at` does not
    /// authorize even if never explicitly revoked.
    pub expires_at: Option<String>,
    /// SG3: when the grant was revoked, set by a `capability.grant_revoked`
    /// event. A revoked grant is treated as absent by decide-time read-back.
    pub revoked_at: Option<String>,
    pub updated_sequence: i64,
}

impl CapabilityGrantProjection {
    /// SG3: whether this grant authorizes a request AT the supplied wall-clock
    /// instant (`now`, an RFC3339/comparable timestamp string).
    ///
    /// A grant authorizes only when it is an `allow` grant that has neither been
    /// revoked nor passed its `expires_at`. A revoked or expired grant is treated
    /// as ABSENT for read-back, never as a standing authorization. A `deny` grant
    /// is never an authorization (it is a standing denial, surfaced separately).
    pub fn is_active_allow(&self, now: &str) -> bool {
        self.effect == "allow" && !self.is_revoked() && !self.is_expired(now)
    }

    /// SG3: whether this grant has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// SG3: whether this grant is past its `expires_at` at the supplied instant.
    /// A grant with no `expires_at` never expires on its own.
    ///
    /// `now` and `expires_at` are compared numerically when both parse as integer
    /// epoch timestamps (the controller stamps epoch-millis), so a shorter-but-
    /// larger value is never mis-ordered; otherwise they fall back to a lexical
    /// compare (suitable for fixed-width RFC3339).
    pub fn is_expired(&self, now: &str) -> bool {
        match &self.expires_at {
            Some(expires_at) => match (now.parse::<i64>(), expires_at.parse::<i64>()) {
                (Ok(now), Ok(expires_at)) => now >= expires_at,
                _ => now >= expires_at.as_str(),
            },
            None => false,
        }
    }
}

/// SG5: the controller-owned single-writer workspace lock, projected from the
/// `workspace.lease_acquired` / `workspace.lease_released` event pair.
///
/// One lease row per workspace key (a collision-free encoding of the normalized
/// workspace root, derived controller-side): while a
/// lease is `held`, only its holder session may write the workspace. A second
/// session/run requesting the write lease for the same key is rejected with a
/// typed conflict rather than interleaved. Acquire/release is event-sourced, so
/// the lease rebuilds from the log (`is_held`/`holder_session_id` survive
/// restart) and a stale lease from a dead holder is reclaimable through the
/// liveness-aware recovery path (SG9).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLeaseProjection {
    /// Stable per-workspace key (a collision-free encoding of the normalized
    /// workspace root). One lease row exists per key; re-acquire/release update
    /// this same row in place.
    pub workspace_lease_id: String,
    pub project_id: ProjectId,
    /// The session holding (or that last held) the lease.
    pub holder_session_id: SessionId,
    /// The run holding (or that last held) the lease, when one is associated.
    pub holder_run_id: Option<RunId>,
    /// `held` while a writer owns the lease, `released` once it is freed.
    pub status: String,
    /// When the current/last holder acquired the lease (epoch-millis string).
    pub acquired_at: Option<String>,
    /// When the lease was released, set by `workspace.lease_released`.
    pub released_at: Option<String>,
    /// Why the lease was released (explicit release, reclaimed from a dead
    /// holder during recovery, etc.). Empty while the lease is held.
    pub release_reason: String,
    pub updated_sequence: i64,
}

impl WorkspaceLeaseProjection {
    /// The `held` status string a live (un-released) lease carries.
    pub const HELD: &'static str = "held";
    /// The `released` status string a freed lease carries.
    pub const RELEASED: &'static str = "released";

    /// SG5: whether this lease is currently held by a writer.
    ///
    /// A lease is held when its status is `held` and it has not been released.
    /// A released lease (explicit release OR reclaimed from a dead holder) reads
    /// as free, so the next writer's acquire succeeds.
    pub fn is_held(&self) -> bool {
        self.status == Self::HELD && self.released_at.is_none()
    }

    /// SG5: whether `session_id` is the session currently holding this lease.
    pub fn is_held_by(&self, session_id: &SessionId) -> bool {
        self.is_held() && &self.holder_session_id == session_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionApprovalProjection {
    pub approval_id: String,
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub tool_call_id: Option<ToolCallId>,
    pub capability_profile_id: String,
    pub scope_json: String,
    pub subject_json: String,
    pub status: String,
    pub requested_by: String,
    pub reason: String,
    pub decision: Option<String>,
    pub capability_grant_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectivityExposureProjection {
    pub exposure_id: String,
    pub project_id: ProjectId,
    pub connectivity_endpoint_id: String,
    pub owner_kind: String,
    pub owner_id: String,
    pub channel_kind: String,
    pub exposure: String,
    pub permission_scope: String,
    pub status: String,
    pub capability_grant_id: Option<String>,
    pub health_status: String,
    pub reachable: bool,
    pub revoked_at: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeTargetProjection {
    pub runtime_target_id: String,
    pub project_id: ProjectId,
    pub name: String,
    pub runner_kind: String,
    pub workspace_root: String,
    pub artifact_root: String,
    pub default_cwd: String,
    pub capability_profile_id: String,
    pub connectivity_endpoint_id: Option<String>,
    pub status: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterReadinessProjection {
    pub adapter_kind: String,
    pub project_id: ProjectId,
    pub program: String,
    pub opt_in_env: String,
    pub opted_in: bool,
    pub smoke_status: String,
    pub credential_policy: String,
    pub expected_marker: String,
    pub env_allowlist_count: i64,
    pub redaction_rule_count: i64,
    pub output_limit_bytes: i64,
    pub dogfood_blocker: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterSmokeReportProjection {
    pub smoke_report_id: String,
    pub project_id: ProjectId,
    pub adapter_kind: String,
    pub smoke_status: String,
    pub credential_scan_status: String,
    pub marker_found: bool,
    pub artifact_root: Option<String>,
    pub reason: String,
    pub dogfood_readiness_effect: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchPlanProjection {
    pub dispatch_plan_id: String,
    pub project_id: ProjectId,
    pub adapter_kind: String,
    pub provider_kind: String,
    pub credential_scope: String,
    pub agent_id: AgentId,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_program: String,
    pub runtime_arg_count: i64,
    pub runtime_prompt_policy: String,
    pub runtime_cwd: String,
    pub artifact_root: String,
    pub request_env_count: i64,
    pub env_allowlist_count: i64,
    pub redaction_rule_count: i64,
    pub stdout_format: String,
    pub stderr_policy: String,
    pub provider_cli_executed: bool,
    pub status: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchGateProjection {
    pub dispatch_gate_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub adapter_kind: String,
    pub provider_cli_execution_allowed: bool,
    pub status: String,
    pub required_dogfood_gate: String,
    pub reason_codes: String,
    pub provider_cli_executed: bool,
    pub runtime_prompt_policy: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchReplayProjection {
    pub dispatch_replay_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub adapter_kind: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub fixture_path: String,
    pub fixture_hash: String,
    pub input_event_count: i64,
    pub appended_event_count: i64,
    pub tool_event_count: i64,
    pub summary_event_count: i64,
    pub completed_turn_count: i64,
    pub provider_cli_executed: bool,
    pub raw_content_policy: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchExecutionRequestProjection {
    pub execution_request_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub adapter_kind: String,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub opt_in_env: String,
    pub runtime_prompt_policy: String,
    pub reason_codes: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchExecutionProjection {
    pub dispatch_execution_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub execution_request_id: String,
    pub adapter_kind: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub exit_code: Option<i64>,
    pub runtime_process_ref: Option<String>,
    pub stdout_artifact_id: Option<String>,
    pub stderr_artifact_id: Option<String>,
    pub artifact_root: String,
    pub credential_scan_status: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub reason_codes: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchPromptSourceProjection {
    pub prompt_source_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub prompt_hash: String,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub source_hash: Option<String>,
    pub materialization_status: String,
    pub raw_prompt_policy: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchPromptMaterializationProjection {
    pub materialization_id: String,
    pub project_id: ProjectId,
    pub dispatch_plan_id: String,
    pub prompt_source_id: String,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub expected_source_hash: Option<String>,
    pub observed_source_hash: Option<String>,
    pub expected_prompt_hash: String,
    pub materialized_prompt_hash: Option<String>,
    pub status: String,
    pub raw_prompt_policy: String,
    pub reason_codes: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallProjection {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub tool_origin: String,
    pub status: String,
    pub input_artifact_id: Option<String>,
    pub output_artifact_id: Option<String>,
    /// Per-call provenance and wall-clock timing (ACI7). Bundled so the queryable
    /// `command -> turn -> permission -> tool -> artifact` chain is one field and
    /// the many existing construction sites default it with
    /// [`ToolCallProvenance::default`].
    pub provenance: ToolCallProvenance,
    pub updated_sequence: i64,
}

/// Per-tool-call provenance and timing (ACI7).
///
/// This is the queryable spine `tool-exposure.md` asks for: a `correlation_id`
/// ties the command -> turn -> permission -> tool -> artifact -> adapter-event
/// chain together, `permission_decision_id` and `capability_grant_use_id` pin
/// the authorization that allowed the call, and `started_at`/`completed_at` are
/// wall-clock millis-since-epoch (consistent with the ACI6 timing fields) for
/// later evaluation. Every field is optional so a call that has not yet been
/// authorized/started, and the many non-dispatch construction sites, default
/// cleanly; the real loop dispatch fills them in.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolCallProvenance {
    pub correlation_id: Option<String>,
    pub permission_decision_id: Option<String>,
    pub capability_grant_use_id: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolObservationProjection {
    pub tool_observation_id: String,
    pub session_id: SessionId,
    pub tool_call_id: Option<ToolCallId>,
    pub source: String,
    pub external_tool_ref: Option<String>,
    pub tool_name: String,
    pub observed_status: String,
    pub instrumentation_level: String,
    pub confidence: String,
    pub raw_event_hash: String,
    pub artifact_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacketProjection {
    pub memory_packet_id: MemoryPacketId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub packet_artifact_id: Option<String>,
    pub purpose: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecordProjection {
    pub memory_record_id: String,
    pub project_id: ProjectId,
    pub scope: String,
    pub scope_owner_ref: String,
    pub subject_ref: Option<String>,
    pub sensitivity_classification: String,
    pub record_kind: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub body: String,
    pub confidence: String,
    pub review_state: String,
    pub source_count: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub supersedes_memory_record_id: Option<String>,
    pub revoked_by_memory_record_id: Option<String>,
    pub redaction_state: String,
    pub invalidated_at: Option<String>,
    pub invalidation_reason: Option<String>,
    pub packet_item_ref: Option<String>,
    pub updated_sequence: i64,
}

impl MemoryRecordProjection {
    pub fn is_packet_eligible(&self) -> bool {
        self.review_state == "reviewed"
            && self.invalidated_at.is_none()
            && self.valid_until.is_none()
            && self.revoked_by_memory_record_id.is_none()
            && self.redaction_state != RedactionState::ContainsSensitive.as_str()
            && self.redaction_state != RedactionState::Unknown.as_str()
            && self.sensitivity_classification != "secret_derived"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySourceProjection {
    pub memory_source_id: String,
    pub memory_record_id: String,
    pub source_kind: String,
    pub source_event_id: Option<String>,
    pub source_artifact_id: Option<String>,
    pub source_path: Option<String>,
    pub source_anchor: Option<String>,
    pub source_content_hash: Option<String>,
    pub source_sequence: Option<i64>,
    pub quote_artifact_id: Option<String>,
    pub observed_at: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceProjection {
    pub evidence_id: EvidenceId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub kind: String,
    pub artifact_id: Option<String>,
    pub confidence: i64,
    pub updated_sequence: i64,
}

/// SG7: the durable, queryable outcome of `score_run` -- the run's pass/fail
/// signal scored from OBSERVED verification evidence only, plus real wall-clock
/// timing.
///
/// One row per `(run, scored inputs)`: the score id is keyed on the run and a
/// stable digest of the observed evidence it scored, so re-scoring the SAME
/// observed evidence is idempotent (same id, no duplicate) and a rebuild from
/// the event log reconstructs the score identically. The `score_inputs_json`
/// records exactly which observed verification verdicts fed the score, so the
/// outcome is auditable and reproducible; agent-reported claims never appear
/// here because `score_run` filters them out before they reach this projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunScoreProjection {
    pub run_score_id: String,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub session_id: SessionId,
    pub run_id: RunId,
    /// `passed` / `failed` / `inconclusive` -- the run outcome signal.
    pub outcome: String,
    /// True iff every required acceptance criterion was met by observed passing
    /// evidence. The single trustworthy pass/fail derived from observed evidence.
    pub passed: bool,
    /// How many acceptance criteria were required for this run.
    pub criteria_total: i64,
    /// How many of those criteria observed passing evidence satisfied.
    pub criteria_met: i64,
    /// How many OBSERVED verification verdicts were scored (agent-reported
    /// claims are excluded before scoring, so they never count here).
    pub observed_evidence_count: i64,
    /// Wall-clock millis-since-epoch the scored run started.
    pub started_at: i64,
    /// Wall-clock millis-since-epoch the scored run completed.
    pub completed_at: i64,
    /// `completed_at - started_at`, clamped at 0 -- the real run duration that
    /// replaces the descriptive event-sequence-delta "duration".
    pub duration_millis: i64,
    /// A stable JSON digest of the observed evidence the score consumed (each
    /// criterion, the observed verdict that matched it, and the source). The
    /// reproducibility anchor: the same digest always yields the same score.
    pub score_inputs_json: String,
    pub updated_sequence: i64,
}

/// SG8: the controller-owned shadow-git checkpoint, projected from the
/// `checkpoint.created` / `checkpoint.restored` event pair.
///
/// This graduates the designed `checkpoints` table
/// (`state-model.md:1042`) from design to code. One row per checkpoint id; the
/// `checkpoint.restored` event re-emits the SAME row with `restored_at` (and the
/// restore detail) stamped, so the rollback is auditable and the projection
/// rebuilds identically from the event log.
///
/// The checkpoint mechanism is a SEPARATE shadow `.git` directory (resolving the
/// SG8 open question): the workspace root is checkpointed by committing it into a
/// shadow repo whose `GIT_DIR` lives under the state root and whose
/// `GIT_WORK_TREE` is the workspace, so the user's real `.git` is never touched.
/// The commit SHA is the restorable ref ([`Self::commit_ref`]); it is durable on
/// disk in the shadow repo and recorded here, so a checkpoint taken before a
/// restart is still restorable after.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointProjection {
    /// Stable per-checkpoint key. One row per checkpoint; the restore event
    /// updates this same row in place.
    pub checkpoint_id: String,
    pub project_id: ProjectId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: Option<String>,
    /// The checkpoint mechanism kind (`shadow_git` for SG8). Recorded so a later
    /// mechanism change is distinguishable on the log.
    pub kind: String,
    /// The shadow-repo commit SHA this checkpoint is restorable to. This is the
    /// durable, restart-surviving ref: it lives in the shadow `.git` on disk and
    /// is recorded here so restore can `git checkout` it after a restart.
    pub commit_ref: String,
    /// The workspace root the checkpoint covers.
    pub workspace_root: String,
    /// The shadow `.git` directory the checkpoint commit lives in.
    pub shadow_git_dir: String,
    /// A content fingerprint of the checkpointed tree (the shadow commit's tree
    /// SHA), so two checkpoints of identical content share a fingerprint and a
    /// rebuild reconstructs the row identically.
    pub content_hash: String,
    /// When the checkpoint was created (epoch-millis string).
    pub created_at: Option<String>,
    /// When the checkpoint was last restored, set by `checkpoint.restored`.
    /// `None` until a `Restore` command targets this checkpoint.
    pub restored_at: Option<String>,
    pub updated_sequence: i64,
}

impl CheckpointProjection {
    /// The shadow-repo commit SHA this checkpoint is restorable to.
    pub fn commit_ref(&self) -> &str {
        &self.commit_ref
    }

    /// SG8: whether this checkpoint has been restored at least once.
    pub fn is_restored(&self) -> bool {
        self.restored_at.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskOutcomeReportProjection {
    pub task_outcome_report_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub outcome_status: String,
    pub started_sequence: i64,
    pub completed_sequence: i64,
    pub duration_sequence_span: i64,
    pub action_count: i64,
    pub tool_call_count: i64,
    pub evidence_count: i64,
    pub memory_packet_count: i64,
    pub confidence: Option<i64>,
    pub blocker: Option<String>,
    pub review_outcome: String,
    pub report_artifact_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewFindingProjection {
    pub review_finding_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub tool_call_id: Option<ToolCallId>,
    pub workpad_task_id: Option<String>,
    pub reviewer: String,
    pub finding_kind: String,
    pub severity: String,
    pub summary: String,
    pub status: String,
    pub evidence_artifact_id: Option<String>,
    pub follow_up: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBindingProjection {
    pub source_binding_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub source_kind: String,
    pub source_task_id: String,
    pub source_path: String,
    pub source_anchor: String,
    pub source_hash: String,
    pub binding_status: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadIndexResetProjection {
    pub project_id: ProjectId,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadFileProjection {
    pub path: String,
    pub project_id: ProjectId,
    pub content_hash: String,
    pub headings: String,
    pub objective: Option<String>,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadTaskProjection {
    pub workpad_task_id: String,
    pub project_id: ProjectId,
    pub path: String,
    pub source_anchor: String,
    pub title: String,
    pub observed_status: String,
    pub capo_execution_status: String,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

/// GA1 (goal-orchestration GO1/GO3): the active-goal read model.
///
/// One row per Capo-owned goal. The lifecycle `status` is last-write-wins over
/// the `goal.created`/`updated`/`paused`/`resumed`/`blocked`/`cleared` event
/// stream, so the projection rebuilds identically from the log.
///
/// A goal links to its project, task, agent, session, and parent goal, and
/// carries the structured success criteria, constraints, verification surface,
/// budget, and stop conditions as JSON (GO6). It references its current dispatch
/// `RunId` as the goal-attempt run identity rather than introducing a second
/// run-completion notion: the dispatch execution-status projections
/// (`AdapterDispatchExecutionProjection`) remain the single owner of run exit.
///
/// IMPORTANT: `status` is NEVER `complete` by an ordinary lifecycle write. A
/// Capo goal-complete transition is reachable only through the GA5 auditor on
/// observed evidence; the lifecycle events here cannot flip a goal to complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalProjection {
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    /// The parent goal this goal is a subgoal of (GO11), when one exists.
    pub parent_goal_id: Option<GoalId>,
    /// The dispatch run identity for the current goal attempt (GO1 `GoalAttempt`).
    /// `None` before a run is dispatched. This is a REFERENCE to the existing
    /// dispatch run; the run's terminal status stays owned by the dispatch
    /// execution-status projection.
    pub attempt_run_id: Option<RunId>,
    pub objective: String,
    /// Lifecycle status: `active` / `paused` / `blocked` / `cleared`. Never
    /// `complete` from a lifecycle write (the auditor owns completion).
    pub status: String,
    /// Structured success criteria (GO6), as JSON.
    pub success_criteria_json: String,
    /// Structured constraints (GO6), as JSON.
    pub constraints_json: String,
    /// Structured verification surface (GO6), as JSON.
    pub verification_surface_json: String,
    /// Structured budget (GO6) -- e.g. the `GoalBudget` resource ceiling, as JSON.
    pub budget_json: String,
    /// Structured stop conditions (GO6), as JSON.
    pub stop_conditions_json: String,
    /// The most recent blocker reason while `status = blocked`, else empty (GO3
    /// current-blocker state).
    pub blocker_reason: String,
    pub updated_sequence: i64,
}

impl GoalProjection {
    /// The lifecycle statuses a goal can carry. `complete` is intentionally
    /// absent: completion is the auditor's verdict, not a lifecycle write.
    pub const ACTIVE: &'static str = "active";
    pub const PAUSED: &'static str = "paused";
    pub const BLOCKED: &'static str = "blocked";
    pub const CLEARED: &'static str = "cleared";

    /// Whether this goal is currently active (eligible for continuation).
    pub fn is_active(&self) -> bool {
        self.status == Self::ACTIVE
    }
}

/// GA1 (goal-orchestration GO3): the per-requirement status ledger.
///
/// One row per `(goal, requirement)`. The `status` is last-write-wins over the
/// `goal.requirement_status_changed` stream and distinguishes the requirement
/// states the auditor reasons over (GO9): `unverified`, `supported`,
/// `validated`, `reviewed`, `blocked`, `contradicted`. GA1 only RECORDS the
/// status transitions emitted for it; the GA5 auditor owns deciding which
/// transition is warranted from observed evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementLedgerProjection {
    pub requirement_id: RequirementId,
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    pub summary: String,
    /// `unverified` / `supported` / `validated` / `reviewed` / `blocked` /
    /// `contradicted` (GO9 requirement states).
    pub status: String,
    /// The source class that last drove this status, tagged exactly like the
    /// `tools-aci` evidence sources: `agent_reported` for a claim, or an observed
    /// source (`runtime_output` / `adapter_event`). A requirement is never
    /// `validated`/`reviewed` by an `agent_reported` source alone (the auditor
    /// enforces that); this field records provenance for the read model.
    pub last_status_source: String,
    pub updated_sequence: i64,
}

impl RequirementLedgerProjection {
    pub const UNVERIFIED: &'static str = "unverified";
    pub const SUPPORTED: &'static str = "supported";
    pub const VALIDATED: &'static str = "validated";
    pub const REVIEWED: &'static str = "reviewed";
    pub const BLOCKED: &'static str = "blocked";
    pub const CONTRADICTED: &'static str = "contradicted";

    /// Whether the LAST status was driven by OBSERVED evidence (vs an agent
    /// claim). Mirrors `capo_tools::source_is_observed_evidence` via the single
    /// in-crate classifier so the read model never shows a requirement validated
    /// by a claim alone.
    pub fn is_observed_evidence(&self) -> bool {
        crate::source_is_observed_evidence(&self.last_status_source)
    }
}

/// GA1 (goal-orchestration GO3): the per-goal agent-report / story ledger.
///
/// This is the queryable spine behind the agent-story, validation-ledger,
/// confidence/risk-summary, and current-blocker read surfaces (GO3/GO5). Each
/// row projects ONE recorded report against a goal -- a `goal.report_recorded`
/// event sourced from a `tools-aci` `agent_reported` report (intent, progress,
/// confidence, assumption, blocker, validation, completion claim) or an observed
/// evidence/review reference.
///
/// The load-bearing field is `source`: it is tagged exactly like the
/// `tools-aci` evidence sources (`agent_reported` vs `runtime_output` /
/// `adapter_event`), so a claim is NEVER stored indistinguishably from observed
/// evidence and completion is never reachable by agent assertion alone. The raw
/// report body lives in an artifact, not as authoritative read-model truth; this
/// row holds the structured summary plus references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalReportProjection {
    pub goal_report_id: String,
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub requirement_id: Option<RequirementId>,
    /// The `tools-aci` reporting tool that produced this report (e.g.
    /// `capo.report_progress`, `capo.record_validation`, `capo.raise_blocker`,
    /// `capo.complete_requirement`), or an observed-evidence kind.
    pub report_kind: String,
    /// `agent_reported` for a claim, or an observed source
    /// (`runtime_output` / `adapter_event`). See [`Self::is_observed_evidence`].
    pub source: String,
    /// The agent's self-declared confidence (0-100) for an `agent_reported`
    /// report; `None` for observed evidence (which carries no agent confidence).
    pub confidence: Option<i64>,
    /// A short structured summary of the report for the story read model. The
    /// full body is kept in `body_artifact_id`, not here.
    pub summary: String,
    /// The artifact holding the raw/full report body, preserved as an INPUT, not
    /// authoritative read-model truth (`state-model.md`).
    pub body_artifact_id: Option<String>,
    /// A reference to the observed `EvidenceRecorded` row this report cites, when
    /// the report points at observed evidence rather than restating it.
    pub evidence_id: Option<EvidenceId>,
    pub updated_sequence: i64,
}

impl GoalReportProjection {
    /// Whether this report row is OBSERVED evidence rather than an agent claim.
    /// Mirrors `capo_tools::source_is_observed_evidence` so the two surfaces
    /// classify a source identically.
    pub fn is_observed_evidence(&self) -> bool {
        crate::source_is_observed_evidence(&self.source)
    }

    /// Whether this report is an agent CLAIM (`agent_reported`). The auditor
    /// treats a claim as a proposal only.
    pub fn is_agent_reported(&self) -> bool {
        self.source == "agent_reported"
    }
}

/// GA1 (goal-orchestration GO3/GO8): a recorded continuation decision.
///
/// One row per `goal.continuation_decision_recorded` event. The scheduler that
/// PRODUCES the decision lives in GA4; GA1 owns only the durable record so the
/// "why did (or didn't) this goal continue?" answer is a derived read model. The
/// decision is one of `continue` / `pause` / `block` / `budget-limit` /
/// `no-progress-suppress`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalContinuationProjection {
    pub continuation_id: String,
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    /// The dispatch run this continuation decision was evaluated against, when a
    /// run exists. References the dispatch run identity, never a second one.
    pub attempt_run_id: Option<RunId>,
    /// `continue` / `pause` / `block` / `budget-limit` / `no-progress-suppress`.
    pub decision: String,
    /// A short machine reason code for the decision (e.g. `safe_boundary`,
    /// `input_queued`, `budget_exhausted`, `no_material_progress`).
    pub reason: String,
    pub updated_sequence: i64,
}

/// GA1 (goal-orchestration GO12): observed delegated-provider goal state.
///
/// When Capo mirrors a goal into a provider-native goal mode (e.g. Codex
/// `/goal`), the provider's reported goal state, command surface, and completion
/// are recorded here as OBSERVED-NOT-AUTHORITATIVE evidence. `source` is always
/// an agent-reported/observed tag, never an authoritative Capo completion: a
/// provider-native completion is a claim the GA5 auditor weighs, it does not flip
/// the Capo goal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegatedProviderGoalProjection {
    pub delegated_goal_id: String,
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    /// The provider whose native goal mode this mirrors (e.g. `codex`).
    pub provider_kind: String,
    /// The provider-native goal handle/ref, when the provider exposes one.
    pub provider_goal_ref: Option<String>,
    /// The provider's observed goal state string (provider vocabulary, recorded
    /// verbatim as observed input, not mapped onto Capo lifecycle states).
    pub provider_state: String,
    /// Always an `agent_reported`/observed tag -- provider-native completion is
    /// evidence the auditor weighs, never authoritative Capo completion.
    pub source: String,
    /// The artifact preserving the raw provider goal output, kept as an INPUT.
    pub body_artifact_id: Option<String>,
    pub updated_sequence: i64,
}

/// GA5 (goal-orchestration GO9): the evidence-gated completion auditor's verdict.
///
/// One row per `(goal, audit_id)`. The auditor is the ONLY path to a Capo
/// goal-complete verdict: it decides requirement-by-requirement on OBSERVED
/// evidence, validation, review, blocker, and confidence records, NEVER on agent
/// prose or model confidence. The `verdict` is `complete` only when every
/// requirement reached a satisfying state backed by observed evidence; otherwise
/// it is `incomplete`, and `blocking_reason` names the first reason it is not
/// complete. The per-requirement detail (each requirement's audited state and the
/// reason) lives in `requirement_detail_json`, so "why is this (not) complete?" is
/// a derived read model rather than hand-written prose. The decision rebuilds
/// identically from the event log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalAuditDecisionProjection {
    /// Stable per-audit key. Re-auditing the SAME observed state under the same
    /// `audit_id` re-records nothing (idempotent on `(goal, audit_id)`).
    pub audit_id: String,
    pub goal_id: GoalId,
    pub project_id: ProjectId,
    /// The dispatch run this audit was evaluated against, when a run exists.
    /// References the dispatch run identity, never a second one.
    pub attempt_run_id: Option<RunId>,
    /// `complete` / `incomplete` -- the auditor's goal-level verdict.
    pub verdict: String,
    /// A short machine reason code for the verdict (e.g. `all_requirements_met`,
    /// `requirement_blocked`, `requirement_unverified`, `requirement_claim_only`,
    /// `no_requirements`).
    pub reason: String,
    /// How many requirements the goal carries.
    pub requirements_total: i64,
    /// How many of those requirements the auditor judged complete (a satisfying
    /// state backed by observed evidence).
    pub requirements_complete: i64,
    /// A stable JSON array of the per-requirement audited detail (requirement id,
    /// audited state, observed-evidence flag, reason), so the verdict is fully
    /// explainable from the read model.
    pub requirement_detail_json: String,
    pub updated_sequence: i64,
}

impl GoalAuditDecisionProjection {
    /// The goal-level verdicts the auditor can record. `complete` is reachable
    /// ONLY here -- it is never a lifecycle write on [`GoalProjection`].
    pub const COMPLETE: &'static str = "complete";
    pub const INCOMPLETE: &'static str = "incomplete";

    /// Whether this verdict marks the goal complete.
    pub fn is_complete(&self) -> bool {
        self.verdict == Self::COMPLETE
    }
}
