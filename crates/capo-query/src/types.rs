use capo_core::{ProjectId, SessionId};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    AgentProjection, ConnectivityExposureProjection, EventRecord, EvidenceProjection,
    MemoryPacketProjection, ReviewFindingProjection, RunProjection, RuntimeTargetProjection,
    SessionProjection, TaskOutcomeReportProjection, ToolCallProjection, ToolObservationProjection,
    WorkpadTaskProjection,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDashboard {
    pub project_id: ProjectId,
    pub agents: Vec<AgentDashboardRow>,
    pub project_evidence: Vec<EvidenceProjection>,
    pub review_findings: Vec<ReviewFindingProjection>,
    pub task_outcome_reports: Vec<TaskOutcomeReportProjection>,
    pub runtime_targets: Vec<RuntimeTargetProjection>,
    pub connectivity_exposures: Vec<ConnectivityExposureProjection>,
    pub adapter_readiness: Vec<AdapterReadinessProjection>,
    pub adapter_smoke_reports: Vec<AdapterSmokeReportProjection>,
    pub adapter_dispatch_plans: Vec<AdapterDispatchPlanProjection>,
    pub adapter_dispatch_gates: Vec<AdapterDispatchGateProjection>,
    pub adapter_dispatch_replays: Vec<AdapterDispatchReplayProjection>,
    pub adapter_dispatch_execution_requests: Vec<AdapterDispatchExecutionRequestProjection>,
    pub adapter_dispatch_executions: Vec<AdapterDispatchExecutionProjection>,
    pub adapter_dispatch_prompt_sources: Vec<AdapterDispatchPromptSourceProjection>,
    pub adapter_dispatch_prompt_materializations:
        Vec<AdapterDispatchPromptMaterializationProjection>,
    pub adapter_dogfood_gate: AdapterDogfoodGate,
    pub workpad_tasks: Vec<WorkpadTaskProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDashboardRow {
    pub agent: AgentProjection,
    pub session: Option<SessionDashboardRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeTargetControlReadiness {
    pub runtime_target_id: String,
    pub runner_kind: String,
    pub target_status: String,
    pub target_ready: bool,
    pub control_exposure_ready: bool,
    pub control_exposure_id: String,
    pub control_exposure_status: String,
    pub control_exposure_scope: String,
    pub control_exposure_permission_scope: String,
    pub control_exposure_reachable: bool,
    pub ready: bool,
    pub blockers: String,
    pub next_action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDashboardRow {
    pub session: SessionProjection,
    pub run: Option<RunProjection>,
    pub evidence: Vec<EvidenceProjection>,
    pub tool_calls: Vec<ToolCallProjection>,
    pub tool_observations: Vec<ToolObservationProjection>,
    pub memory_packets: Vec<MemoryPacketProjection>,
    pub review_findings: Vec<ReviewFindingProjection>,
    pub task_outcome_reports: Vec<TaskOutcomeReportProjection>,
    pub recent_events: Vec<EventRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolActivitySummary {
    pub agent_count: usize,
    pub active_session_count: usize,
    pub tool_call_count: usize,
    pub tool_observation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDogfoodGate {
    pub ready: bool,
    pub status: String,
    pub required_adapters: Vec<String>,
    pub proven_adapters: Vec<String>,
    pub blocked_adapters: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchStatus {
    pub dispatch_plan_id: String,
    pub adapter_kind: String,
    pub agent_name: String,
    pub session_id: String,
    pub run_id: String,
    pub plan_status: String,
    pub provider_kind: String,
    pub credential_scope: String,
    pub runtime_program: String,
    pub runtime_arg_count: i64,
    pub runtime_prompt_policy: String,
    pub provider_cli_executed: bool,
    pub dogfood_gate_status: String,
    pub latest_dispatch_gate_id: String,
    pub latest_gate_status: String,
    pub latest_gate_provider_cli_execution_allowed: bool,
    pub latest_gate_reasons: String,
    pub latest_dispatch_replay_id: String,
    pub latest_replay_appended_events: i64,
    pub latest_replay_raw_content_policy: String,
    pub latest_dispatch_execution_id: String,
    pub latest_execution_status: String,
    pub latest_execution_provider_cli_execution_allowed: bool,
    pub latest_execution_provider_cli_executed: bool,
    pub latest_execution_credential_scan_status: String,
    pub latest_execution_stdout_artifact_id: String,
    pub latest_execution_stderr_artifact_id: String,
    pub latest_execution_reasons: String,
    pub next_action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDogfoodReadiness {
    pub ready: bool,
    pub status: String,
    pub real_agent_connector_ready: bool,
    pub runtime_target_ready: bool,
    pub workpad_bridge_ready: bool,
    pub dispatch_chain_ready: bool,
    pub runtime_target_count: usize,
    pub available_runtime_target_count: usize,
    pub workpad_task_count: usize,
    pub observed_workpad_task_count: usize,
    pub imported_workpad_task_count: usize,
    pub dispatch_plan_count: usize,
    pub ready_dispatch_gate_count: usize,
    pub dispatch_replay_count: usize,
    pub dispatch_execution_count: usize,
    pub connector_evidence_refs: Vec<String>,
    pub runtime_target_refs: Vec<String>,
    pub workpad_task_refs: Vec<String>,
    pub dispatch_chain_refs: Vec<String>,
    pub project_evidence_refs: Vec<String>,
    pub blockers: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDashboardQuery {
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub status: Option<String>,
    pub workpad_path: Option<String>,
    pub workpad_status: Option<String>,
    pub recent_event_limit: usize,
}

impl ProjectDashboardQuery {
    pub fn new(project_id: ProjectId) -> Self {
        Self {
            project_id,
            session_id: None,
            status: None,
            workpad_path: None,
            workpad_status: None,
            recent_event_limit: 5,
        }
    }

    pub fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    pub fn with_workpad_path(mut self, path: impl Into<String>) -> Self {
        self.workpad_path = Some(path.into());
        self
    }

    pub fn with_workpad_status(mut self, status: impl Into<String>) -> Self {
        self.workpad_status = Some(status.into());
        self
    }
}
