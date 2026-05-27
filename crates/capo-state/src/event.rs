use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    ProjectRegistered,
    TaskDiscovered,
    AgentRegistered,
    SessionStarted,
    SessionRedirected,
    SessionSummaryUpdated,
    RunStarted,
    RunExited,
    PermissionRequested,
    PermissionDecided,
    PermissionApprovalQueued,
    CapabilityGrantCreated,
    CapabilityGrantUsed,
    ConnectivityExposureRequested,
    ConnectivityExposureChanged,
    ConnectivityExposureRevoked,
    ConnectivityHealthChanged,
    RuntimeTargetRegistered,
    RuntimeTargetStatusChanged,
    AdapterReadinessChecked,
    AdapterSmokeRecorded,
    AdapterDispatchPlanned,
    AdapterDispatchGateChecked,
    AdapterDispatchReplayed,
    AdapterDispatchExecutionRequested,
    AdapterDispatchExecuted,
    AdapterDispatchPromptSourceRecorded,
    AdapterDispatchPromptMaterialized,
    ToolCallRequested,
    ToolInvocationStarted,
    ToolObservationRecorded,
    ToolOutputArtifactRecorded,
    ToolOutputObserved,
    ToolCallCompleted,
    ToolResultDelivered,
    MemoryPacketBuilt,
    MemoryRecordIngested,
    MemoryRecordInvalidated,
    TaskOutcomeReportGenerated,
    ReviewFindingRecorded,
    EvidenceRecorded,
    WorkpadIndexed,
    WorkpadTaskImported,
    WorkpadProposalWritten,
    ServerRequestHandled,
    RecoveryStarted,
    RecoveryCompleted,
    SessionInterrupted,
    SessionStopped,
}

impl EventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRegistered => "project.registered",
            Self::TaskDiscovered => "task.discovered",
            Self::AgentRegistered => "agent.registered",
            Self::SessionStarted => "session.started",
            Self::SessionRedirected => "session.redirected",
            Self::SessionSummaryUpdated => "session.summary_updated",
            Self::RunStarted => "run.started",
            Self::RunExited => "run.exited",
            Self::PermissionRequested => "permission.requested",
            Self::PermissionDecided => "permission.decided",
            Self::PermissionApprovalQueued => "permission.approval_queued",
            Self::CapabilityGrantCreated => "capability.grant_created",
            Self::CapabilityGrantUsed => "capability.grant_used",
            Self::ConnectivityExposureRequested => "connectivity.exposure_requested",
            Self::ConnectivityExposureChanged => "connectivity.exposure_changed",
            Self::ConnectivityExposureRevoked => "connectivity.exposure_revoked",
            Self::ConnectivityHealthChanged => "connectivity.health_changed",
            Self::RuntimeTargetRegistered => "runtime.target_registered",
            Self::RuntimeTargetStatusChanged => "runtime.target_status_changed",
            Self::AdapterReadinessChecked => "adapter.readiness_checked",
            Self::AdapterSmokeRecorded => "adapter.smoke_recorded",
            Self::AdapterDispatchPlanned => "adapter.dispatch_planned",
            Self::AdapterDispatchGateChecked => "adapter.dispatch_gate_checked",
            Self::AdapterDispatchReplayed => "adapter.dispatch_replayed",
            Self::AdapterDispatchExecutionRequested => "adapter.dispatch_execution_requested",
            Self::AdapterDispatchExecuted => "adapter.dispatch_executed",
            Self::AdapterDispatchPromptSourceRecorded => "adapter.dispatch_prompt_source_recorded",
            Self::AdapterDispatchPromptMaterialized => "adapter.dispatch_prompt_materialized",
            Self::ToolCallRequested => "tool.call_requested",
            Self::ToolInvocationStarted => "tool.invocation_started",
            Self::ToolObservationRecorded => "tool.observation_recorded",
            Self::ToolOutputArtifactRecorded => "tool.output_artifact_recorded",
            Self::ToolOutputObserved => "tool.output_observed",
            Self::ToolCallCompleted => "tool.call_completed",
            Self::ToolResultDelivered => "tool.result_delivered",
            Self::MemoryPacketBuilt => "memory.packet_built",
            Self::MemoryRecordIngested => "memory.record_ingested",
            Self::MemoryRecordInvalidated => "memory.record_invalidated",
            Self::TaskOutcomeReportGenerated => "task.outcome_report_generated",
            Self::ReviewFindingRecorded => "review.finding_recorded",
            Self::EvidenceRecorded => "evidence.recorded",
            Self::WorkpadIndexed => "workpad.indexed",
            Self::WorkpadTaskImported => "workpad.task_imported",
            Self::WorkpadProposalWritten => "workpad.proposal_written",
            Self::ServerRequestHandled => "server.request_handled",
            Self::RecoveryStarted => "recovery.started",
            Self::RecoveryCompleted => "recovery.completed",
            Self::SessionInterrupted => "session.interrupted",
            Self::SessionStopped => "session.stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionState {
    Safe,
    Redacted,
    Unknown,
    ContainsSensitive,
}

impl RedactionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Redacted => "redacted",
            Self::Unknown => "unknown",
            Self::ContainsSensitive => "contains_sensitive",
        }
    }

    pub const fn is_persistable_artifact(self) -> bool {
        matches!(self, Self::Safe | Self::Redacted)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewEvent {
    pub event_id: String,
    pub kind: EventKind,
    pub actor: String,
    pub project_id: Option<ProjectId>,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload_json: String,
    pub idempotency_key: Option<String>,
    pub redaction_state: RedactionState,
}

impl NewEvent {
    pub fn new(event_id: impl Into<String>, kind: EventKind, actor: impl Into<String>) -> Self {
        Self {
            event_id: event_id.into(),
            kind,
            actor: actor.into(),
            project_id: None,
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: None,
            payload_json: "{}".to_string(),
            idempotency_key: None,
            redaction_state: RedactionState::Safe,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventRecord {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub actor: String,
    pub project_id: Option<ProjectId>,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload_json: String,
    pub idempotency_key: Option<String>,
    pub redaction_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub project_id: Option<ProjectId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub kind: String,
    pub uri: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub redaction_state: RedactionState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryAttempt {
    pub recovery_attempt_id: String,
    pub status: String,
    pub started_sequence: i64,
    pub completed_sequence: Option<i64>,
}
