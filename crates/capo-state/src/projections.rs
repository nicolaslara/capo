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
    pub updated_sequence: i64,
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
    pub updated_sequence: i64,
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
