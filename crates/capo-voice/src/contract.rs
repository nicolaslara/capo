//! Public voice control contract types.

use capo_core::{CommandEnvelope, ProjectId};
pub const VOICE_TRANSCRIPT_RETENTION_DEFAULT: TranscriptRetentionPolicy =
    TranscriptRetentionPolicy::DoNotRetainRaw;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptRetentionPolicy {
    DoNotRetainRaw,
    RetainRedactedSummary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryIngestionPolicy {
    None,
    ReviewedRedactedSummaryOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceIntentKind {
    AgentStatus,
    AdapterSmokeStatus,
    ConnectivityStatus,
    DashboardSummary,
    DispatchStatus,
    DogfoodReadiness,
    NextWork,
    RecentWork,
    ReviewNeeds,
    RedirectSession,
    RuntimeTargetReadiness,
    RuntimeTargetStatus,
    StartNextWork,
    InterruptSession,
    StopSession,
    ToolActivity,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceTranscriptInput {
    pub voice_session_id: String,
    pub actor_id: String,
    pub project_id: ProjectId,
    pub transcript_text: String,
    pub asr_confidence: Option<i64>,
    pub retention_policy: TranscriptRetentionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceCommandPlan {
    pub intent_kind: VoiceIntentKind,
    pub command: Option<CommandEnvelope>,
    pub read_contract: VoiceReadContract,
    pub transcript_policy: VoiceTranscriptPolicy,
    pub requires_visible_confirmation: bool,
    pub assistant_reply_hint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceReadContract {
    pub query_scope: VoiceReadScope,
    pub required_fields: Vec<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceReadScope {
    ProjectDashboard,
    ProjectLatestConnectivityExposure {
        owner_kind: Option<String>,
        owner_id: Option<String>,
        channel_kind: Option<String>,
    },
    ProjectRuntimeTargetStatus {
        runtime_target_id: String,
    },
    ProjectRuntimeTargetControlReadiness {
        runtime_target_id: String,
    },
    ProjectLatestRuntimeTargetStatus {
        runner_kind: Option<String>,
        status: Option<String>,
    },
    ProjectAdapterSmokeReportStatus {
        smoke_report_id: String,
    },
    ProjectLatestAdapterSmokeReport {
        adapter_kind: Option<String>,
    },
    ProjectDispatchStatus {
        dispatch_plan_id: String,
    },
    ProjectLatestDispatchStatus {
        agent_name: Option<String>,
    },
    ProjectDogfoodReadiness,
    ProjectNextWork,
    ProjectRecentWork,
    ProjectReviewNeeds,
    ProjectToolActivity,
    AgentToolActivity {
        agent_name: String,
    },
    Agent {
        agent_name: String,
    },
    SessionForAgent {
        agent_name: String,
    },
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceTranscriptPolicy {
    pub retain_raw_transcript: bool,
    pub redaction_required: bool,
    pub memory_ingestion: MemoryIngestionPolicy,
    pub audit_note: &'static str,
}
