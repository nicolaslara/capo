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
    CapabilityGrantRevoked,
    CapabilityGrantExpired,
    WorkspaceLeaseAcquired,
    WorkspaceLeaseReleased,
    ConnectivityExposureRequested,
    ConnectivityExposureChanged,
    ConnectivityExposureRevoked,
    ConnectivityHealthChanged,
    // CT1 (connectivity-tunnel): promotion of the effective `ExposurePolicy`
    // ceiling (Loopback -> Private/Public) is itself an audited fact, separate
    // from the per-exposure `connectivity.exposure_requested` trail: it records
    // WHY a private/public exposure became possible (old/new ceiling, the opt-in
    // source, a timestamp), carries no secret, and is replay-stable.
    //
    // FORWARD-COMPATIBLE STUB (CT1): the codec round-trips this kind, but CT1 has
    // NO live emitter — nothing in the live bind/connect/expose-stub path emits
    // `connectivity.policy_changed` yet (the only policy constructed live is the
    // default loopback-only one, which is never a promotion). The opt-in promotion
    // CLI path that emits this event lands in CT3/CT5. A future reader must not
    // assume a populated policy-change audit history exists before then.
    ConnectivityPolicyChanged,
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
    // DP6 (memory-architecture.md): the extraction/index/staleness MemoryJob
    // lifecycle. `job_requested`/`job_completed` bracket an `extract_facts` /
    // `index_fts` / `invalidate` / `rebuild` job; `record_superseded` and
    // `record_promoted` are the staleness/review transitions a job emits. A
    // generated record can never supersede a reviewed workpad decision without an
    // explicit `record_promoted`.
    MemoryJobRequested,
    MemoryJobCompleted,
    MemoryRecordSuperseded,
    MemoryRecordPromoted,
    TaskOutcomeReportGenerated,
    ReviewFindingRecorded,
    EvidenceRecorded,
    RunScored,
    WorkpadIndexed,
    WorkpadTaskImported,
    WorkpadProposalWritten,
    ServerRequestHandled,
    RecoveryStarted,
    RecoveryCompleted,
    SessionInterrupted,
    SessionStopped,
    CheckpointCreated,
    CheckpointRestored,
    // DP8 (git worktree isolation per session/goal): the worktree lifecycle as
    // events -- create-on-session-start, reconcile/merge-back point, and teardown
    // -- recorded so a worktree can be reconstructed/inspected after restart and
    // never silently abandoned.
    WorktreeCreated,
    WorktreeReconciled,
    WorktreeTornDown,
    RunHardKilled,
    RunAborted,
    RunOrphaned,
    RunRecovered,
    // GA1 (goal-orchestration GO1/GO3): the append-only goal lifecycle,
    // requirement-status, continuation-decision, and delegated-provider-goal
    // events. These CITE the `goal-orchestration` schema and do NOT redefine
    // evidence/review -- those reuse `EvidenceRecorded`/`ReviewFindingRecorded`
    // and the `tools-aci` `agent_reported` report events.
    GoalCreated,
    GoalUpdated,
    GoalPaused,
    GoalResumed,
    GoalBlocked,
    GoalCleared,
    RequirementStatusChanged,
    GoalReportRecorded,
    ContinuationDecisionRecorded,
    DelegatedProviderGoalObserved,
    // GA5 (goal-orchestration GO9): the evidence-gated completion auditor's
    // decision. The auditor is the ONLY path to a Capo goal-complete verdict;
    // it decides on OBSERVED evidence, never on agent prose or model confidence.
    GoalAuditDecisionRecorded,
    // DP2 (acp-replay-dedupe.md): the ACP attach/replay lifecycle event kinds. An
    // `adapter.attach_*` pair brackets a `session/resume` reconnect (which creates
    // NO message/item replay events); an `adapter.replay_*` pair brackets a
    // `session/load` import/reconciliation, with `replay_duplicate_detected` /
    // `replay_ambiguous` markers for low-confidence matches. Raw ACP updates are
    // persisted (`adapter.raw_update_observed`) before normalization and never
    // mutate read models directly.
    AdapterAttachStarted,
    AdapterAttachCompleted,
    // The attach-failure terminal kind: emitted when a `session/resume` reconnect
    // fails. The happy-path producer (`capo-controller::ingest_acp_replay_plan`)
    // brackets a successful attach with `started`/`completed`; this is the
    // fail-closed terminal the runtime/health-probe path stamps when the reconnect
    // errors (its `as_str`/`from_wire` round-trip is covered by
    // `dp2_acp_replay_event_kinds_round_trip`).
    AdapterAttachFailed,
    AdapterReplayStarted,
    AdapterRawUpdateObserved,
    AdapterReplayDuplicateDetected,
    AdapterReplayAmbiguous,
    AdapterReplayCompleted,
    // RR1 (remote-runtime): the remote process lifecycle over a
    // `connectivity-tunnel`-provided channel. These PROMOTE the pre-RR1 stub's
    // bare `runtime.remote_target_resolved` / `runtime.remote_process_started`
    // strings (`capo-runtime` loopback decorator) to first-class kinds alongside
    // the `runtime.*` family, each round-trippable through the codec.
    //
    // `RuntimeRemoteTargetResolved` records the proven remote target identity
    // (channel fingerprint from `connectivity-tunnel`) BEFORE a launch; it is the
    // append-first "we are about to cross the boundary to a verified peer" fact.
    // `RuntimeRemoteProcessStarted` is the remote analogue of
    // `runtime.process_started` once the remote spawn returned a remote pid + boot
    // identity. `RuntimeRemoteStartRequested` / `RuntimeRemoteProcessStartFailed`
    // bracket a remote launch (idempotency-keyed pending request; typed launch
    // failure with retryability). The interrupt/terminate/kill kinds are the
    // remote escalation analogues of the local `runtime.{interrupt,terminate,
    // kill}_sent` family. None carries a secret: identity is the derived channel
    // fingerprint, never a raw credential, and details pass redaction.
    RuntimeRemoteStartRequested,
    RuntimeRemoteTargetResolved,
    RuntimeRemoteProcessStarted,
    RuntimeRemoteProcessStartFailed,
    RuntimeRemoteInterruptSent,
    RuntimeRemoteTerminateSent,
    RuntimeRemoteKillSent,
    RuntimeRemoteCleanupCompleted,
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
            Self::CapabilityGrantRevoked => "capability.grant_revoked",
            Self::CapabilityGrantExpired => "capability.grant_expired",
            Self::WorkspaceLeaseAcquired => "workspace.lease_acquired",
            Self::WorkspaceLeaseReleased => "workspace.lease_released",
            Self::ConnectivityExposureRequested => "connectivity.exposure_requested",
            Self::ConnectivityExposureChanged => "connectivity.exposure_changed",
            Self::ConnectivityExposureRevoked => "connectivity.exposure_revoked",
            Self::ConnectivityHealthChanged => "connectivity.health_changed",
            Self::ConnectivityPolicyChanged => "connectivity.policy_changed",
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
            Self::MemoryJobRequested => "memory.job_requested",
            Self::MemoryJobCompleted => "memory.job_completed",
            Self::MemoryRecordSuperseded => "memory.record_superseded",
            Self::MemoryRecordPromoted => "memory.record_promoted",
            Self::TaskOutcomeReportGenerated => "task.outcome_report_generated",
            Self::ReviewFindingRecorded => "review.finding_recorded",
            Self::EvidenceRecorded => "evidence.recorded",
            Self::RunScored => "run.scored",
            Self::WorkpadIndexed => "workpad.indexed",
            Self::WorkpadTaskImported => "workpad.task_imported",
            Self::WorkpadProposalWritten => "workpad.proposal_written",
            Self::ServerRequestHandled => "server.request_handled",
            Self::RecoveryStarted => "recovery.started",
            Self::RecoveryCompleted => "recovery.completed",
            Self::SessionInterrupted => "session.interrupted",
            Self::SessionStopped => "session.stopped",
            Self::CheckpointCreated => "checkpoint.created",
            Self::CheckpointRestored => "checkpoint.restored",
            Self::WorktreeCreated => "worktree.created",
            Self::WorktreeReconciled => "worktree.reconciled",
            Self::WorktreeTornDown => "worktree.torn_down",
            Self::RunHardKilled => "run.hard_killed",
            Self::RunAborted => "run.aborted",
            Self::RunOrphaned => "run.orphaned",
            Self::RunRecovered => "run.recovered",
            Self::GoalCreated => "goal.created",
            Self::GoalUpdated => "goal.updated",
            Self::GoalPaused => "goal.paused",
            Self::GoalResumed => "goal.resumed",
            Self::GoalBlocked => "goal.blocked",
            Self::GoalCleared => "goal.cleared",
            Self::RequirementStatusChanged => "goal.requirement_status_changed",
            Self::GoalReportRecorded => "goal.report_recorded",
            Self::ContinuationDecisionRecorded => "goal.continuation_decision_recorded",
            Self::DelegatedProviderGoalObserved => "goal.delegated_provider_observed",
            Self::GoalAuditDecisionRecorded => "goal.audit_decision_recorded",
            Self::AdapterAttachStarted => "adapter.attach_started",
            Self::AdapterAttachCompleted => "adapter.attach_completed",
            Self::AdapterAttachFailed => "adapter.attach_failed",
            Self::AdapterReplayStarted => "adapter.replay_started",
            Self::AdapterRawUpdateObserved => "adapter.raw_update_observed",
            Self::AdapterReplayDuplicateDetected => "adapter.replay_duplicate_detected",
            Self::AdapterReplayAmbiguous => "adapter.replay_ambiguous",
            Self::AdapterReplayCompleted => "adapter.replay_completed",
            Self::RuntimeRemoteStartRequested => "runtime.remote_start_requested",
            Self::RuntimeRemoteTargetResolved => "runtime.remote_target_resolved",
            Self::RuntimeRemoteProcessStarted => "runtime.remote_process_started",
            Self::RuntimeRemoteProcessStartFailed => "runtime.remote_process_start_failed",
            Self::RuntimeRemoteInterruptSent => "runtime.remote_interrupt_sent",
            Self::RuntimeRemoteTerminateSent => "runtime.remote_terminate_sent",
            Self::RuntimeRemoteKillSent => "runtime.remote_kill_sent",
            Self::RuntimeRemoteCleanupCompleted => "runtime.remote_cleanup_completed",
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
            EventKind::CapabilityGrantRevoked,
            EventKind::CapabilityGrantExpired,
            EventKind::WorkspaceLeaseAcquired,
            EventKind::WorkspaceLeaseReleased,
            EventKind::ConnectivityExposureRequested,
            EventKind::ConnectivityExposureChanged,
            EventKind::ConnectivityExposureRevoked,
            EventKind::ConnectivityHealthChanged,
            EventKind::ConnectivityPolicyChanged,
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
            EventKind::MemoryJobRequested,
            EventKind::MemoryJobCompleted,
            EventKind::MemoryRecordSuperseded,
            EventKind::MemoryRecordPromoted,
            EventKind::TaskOutcomeReportGenerated,
            EventKind::ReviewFindingRecorded,
            EventKind::EvidenceRecorded,
            EventKind::RunScored,
            EventKind::WorkpadIndexed,
            EventKind::WorkpadTaskImported,
            EventKind::WorkpadProposalWritten,
            EventKind::ServerRequestHandled,
            EventKind::RecoveryStarted,
            EventKind::RecoveryCompleted,
            EventKind::SessionInterrupted,
            EventKind::SessionStopped,
            EventKind::CheckpointCreated,
            EventKind::CheckpointRestored,
            EventKind::WorktreeCreated,
            EventKind::WorktreeReconciled,
            EventKind::WorktreeTornDown,
            EventKind::RunHardKilled,
            EventKind::RunAborted,
            EventKind::RunOrphaned,
            EventKind::RunRecovered,
            EventKind::GoalCreated,
            EventKind::GoalUpdated,
            EventKind::GoalPaused,
            EventKind::GoalResumed,
            EventKind::GoalBlocked,
            EventKind::GoalCleared,
            EventKind::RequirementStatusChanged,
            EventKind::GoalReportRecorded,
            EventKind::ContinuationDecisionRecorded,
            EventKind::DelegatedProviderGoalObserved,
            EventKind::GoalAuditDecisionRecorded,
            EventKind::AdapterAttachStarted,
            EventKind::AdapterAttachCompleted,
            EventKind::AdapterAttachFailed,
            EventKind::AdapterReplayStarted,
            EventKind::AdapterRawUpdateObserved,
            EventKind::AdapterReplayDuplicateDetected,
            EventKind::AdapterReplayAmbiguous,
            EventKind::AdapterReplayCompleted,
            EventKind::RuntimeRemoteStartRequested,
            EventKind::RuntimeRemoteTargetResolved,
            EventKind::RuntimeRemoteProcessStarted,
            EventKind::RuntimeRemoteProcessStartFailed,
            EventKind::RuntimeRemoteInterruptSent,
            EventKind::RuntimeRemoteTerminateSent,
            EventKind::RuntimeRemoteKillSent,
            EventKind::RuntimeRemoteCleanupCompleted,
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
