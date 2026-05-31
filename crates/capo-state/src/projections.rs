use capo_core::{
    AgentId, EvidenceId, MemoryPacketId, ProjectId, RunId, SessionId, TaskId, ToolCallId,
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
    SourceBinding(SourceBindingProjection),
    WorkpadIndexReset(WorkpadIndexResetProjection),
    WorkpadFile(WorkpadFileProjection),
    WorkpadTask(WorkpadTaskProjection),
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
