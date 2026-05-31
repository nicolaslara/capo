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
    CheckpointCreated,
    RunHardKilled,
    RunAborted,
    RunOrphaned,
    RunRecovered,
}

/// The terminal outcome a projected turn-ending event carries, in the
/// projected-event vocabulary (`evidence.recorded`/`session.interrupted`/
/// `session.stopped`/`run.exited`).
///
/// This is the single owner of "which projected kinds end a turn and what they
/// mean". Both the controller's event-sourced turn re-derivation
/// (`reconstruct_turn_finished`) and the thread read-model projection map their
/// own outcome type from this one, so the two read models cannot disagree about
/// a turn's terminal status or drift in which kinds are terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectedTurnOutcome {
    Completed,
    Interrupted,
    Stopped,
    Failed,
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
            Self::CheckpointCreated => "checkpoint.created",
            Self::RunHardKilled => "run.hard_killed",
            Self::RunAborted => "run.aborted",
            Self::RunOrphaned => "run.orphaned",
            Self::RunRecovered => "run.recovered",
        }
    }

    /// Parse a persisted projected-kind string back into the typed kind, or
    /// `None` for an unrecognized kind. The inverse of [`Self::as_str`]; callers
    /// that read the persisted `EventRecord.kind` string classify through this so
    /// they share one vocabulary with the append side instead of re-listing kind
    /// literals.
    pub fn from_wire(kind: &str) -> Option<Self> {
        // Enumerate the variants so this stays exhaustive with `as_str`: adding
        // a kind to the enum makes this match it by construction.
        const ALL: &[EventKind] = &[
            EventKind::ProjectRegistered,
            EventKind::TaskDiscovered,
            EventKind::AgentRegistered,
            EventKind::SessionStarted,
            EventKind::SessionRedirected,
            EventKind::SessionSummaryUpdated,
            EventKind::RunStarted,
            EventKind::RunExited,
            EventKind::PermissionRequested,
            EventKind::PermissionDecided,
            EventKind::PermissionApprovalQueued,
            EventKind::CapabilityGrantCreated,
            EventKind::CapabilityGrantUsed,
            EventKind::ConnectivityExposureRequested,
            EventKind::ConnectivityExposureChanged,
            EventKind::ConnectivityExposureRevoked,
            EventKind::ConnectivityHealthChanged,
            EventKind::RuntimeTargetRegistered,
            EventKind::RuntimeTargetStatusChanged,
            EventKind::AdapterReadinessChecked,
            EventKind::AdapterSmokeRecorded,
            EventKind::AdapterDispatchPlanned,
            EventKind::AdapterDispatchGateChecked,
            EventKind::AdapterDispatchReplayed,
            EventKind::AdapterDispatchExecutionRequested,
            EventKind::AdapterDispatchExecuted,
            EventKind::AdapterDispatchPromptSourceRecorded,
            EventKind::AdapterDispatchPromptMaterialized,
            EventKind::ToolCallRequested,
            EventKind::ToolInvocationStarted,
            EventKind::ToolObservationRecorded,
            EventKind::ToolOutputArtifactRecorded,
            EventKind::ToolOutputObserved,
            EventKind::ToolCallCompleted,
            EventKind::ToolResultDelivered,
            EventKind::MemoryPacketBuilt,
            EventKind::MemoryRecordIngested,
            EventKind::MemoryRecordInvalidated,
            EventKind::TaskOutcomeReportGenerated,
            EventKind::ReviewFindingRecorded,
            EventKind::EvidenceRecorded,
            EventKind::WorkpadIndexed,
            EventKind::WorkpadTaskImported,
            EventKind::WorkpadProposalWritten,
            EventKind::ServerRequestHandled,
            EventKind::RecoveryStarted,
            EventKind::RecoveryCompleted,
            EventKind::SessionInterrupted,
            EventKind::SessionStopped,
            EventKind::CheckpointCreated,
            EventKind::RunHardKilled,
            EventKind::RunAborted,
            EventKind::RunOrphaned,
            EventKind::RunRecovered,
        ];
        ALL.iter()
            .copied()
            .find(|candidate| candidate.as_str() == kind)
    }

    /// `true` for the projected `tool.*` kinds the dispatch/replay path emits for
    /// one tool call -- the request, the start, the recorded observation, the
    /// observed runtime output, the recorded output artifact, the completion, and
    /// the delivered result. Single owner of the projected tool-kind set, shared
    /// by the controller's turn re-derivation and the thread read model so the
    /// two cannot disagree about which kinds are tool content.
    ///
    /// This is the projected-event counterpart of
    /// `capo_adapters::NormalizedAdapterEvent::is_tool_event` (which classifies
    /// the upstream `adapter.tool_call_*` events the replay path maps onto these
    /// kinds).
    pub const fn is_tool_event(self) -> bool {
        matches!(
            self,
            Self::ToolCallRequested
                | Self::ToolInvocationStarted
                | Self::ToolObservationRecorded
                | Self::ToolOutputObserved
                | Self::ToolOutputArtifactRecorded
                | Self::ToolCallCompleted
                | Self::ToolResultDelivered
        )
    }

    /// `true` for the projected kind the replay path emits for assistant
    /// output/summary content (`session.summary_updated`).
    pub const fn is_summary_event(self) -> bool {
        matches!(self, Self::SessionSummaryUpdated)
    }

    /// The terminal turn outcome this projected kind carries, or `None` for a
    /// non-terminal kind. Single owner of the turn-terminal taxonomy over the
    /// projected event log.
    pub const fn terminal_turn_outcome(self) -> Option<ProjectedTurnOutcome> {
        match self {
            Self::EvidenceRecorded => Some(ProjectedTurnOutcome::Completed),
            Self::SessionInterrupted => Some(ProjectedTurnOutcome::Interrupted),
            Self::SessionStopped => Some(ProjectedTurnOutcome::Stopped),
            Self::RunExited => Some(ProjectedTurnOutcome::Failed),
            _ => None,
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

    /// Parse a stored/wire `redaction_state` string back into the enum, the
    /// inverse of [`Self::as_str`]. Returns `None` for an unrecognized value so
    /// the egress guard can treat an unknown classification as not-safe.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "safe" => Some(Self::Safe),
            "redacted" => Some(Self::Redacted),
            "unknown" => Some(Self::Unknown),
            "contains_sensitive" => Some(Self::ContainsSensitive),
            _ => None,
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
