//! Conversational voice control contract for Capo.
//!
//! P14 deliberately avoids audio capture or ASR integration. This crate takes
//! already-produced dummy transcript text and lowers it into the same command
//! envelope/read-model contract used by CLI and dashboard surfaces.

use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    RiskLevel,
};

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
    ConnectivityStatus,
    DashboardSummary,
    DispatchStatus,
    DogfoodReadiness,
    NextWork,
    RecentWork,
    ReviewNeeds,
    RedirectSession,
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

pub fn plan_dummy_transcript(input: VoiceTranscriptInput) -> VoiceCommandPlan {
    let normalized = normalize(&input.transcript_text);
    let policy = transcript_policy(input.retention_policy);

    if normalized == "show me the dashboard" || normalized == "what are my agents doing" {
        let mut command = voice_command(
            "voice-dashboard",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "dashboard".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::DashboardSummary,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectDashboard,
                required_fields: vec![
                    "agents",
                    "sessions",
                    "current_goal",
                    "status",
                    "blocker",
                    "confidence",
                    "evidence_refs",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint: "Summarize active agents, goals, blockers, and recent events."
                .to_string(),
        };
    }

    if is_dogfood_readiness_question(&normalized) {
        let mut command = voice_command(
            "voice-dogfood-readiness",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "dogfood_readiness".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::DogfoodReadiness,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectDogfoodReadiness,
                required_fields: vec![
                    "ready",
                    "status",
                    "real_agent_connector_ready",
                    "workpad_bridge_ready",
                    "dispatch_chain_ready",
                    "component_refs",
                    "blockers",
                    "next_actions",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer dogfood readiness from the shared project dashboard query.".to_string(),
        };
    }

    if let Some(dispatch_plan_id) = dispatch_status_plan(&normalized) {
        let mut command = voice_command(
            "voice-dispatch-status",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "dispatch_status".to_string()));
        command
            .structured_args
            .push(("dispatch_plan".to_string(), dispatch_plan_id.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::DispatchStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectDispatchStatus { dispatch_plan_id },
                required_fields: vec![
                    "dispatch_plan_id",
                    "adapter_kind",
                    "plan_status",
                    "dogfood_gate_status",
                    "latest_gate_status",
                    "latest_replay_appended_events",
                    "latest_execution_status",
                    "provider_cli_executed",
                    "next_action",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer dispatch-chain status from the shared project dashboard query.".to_string(),
        };
    }

    if let Some(agent_name) = latest_dispatch_status_agent(&normalized) {
        let mut command = voice_command(
            "voice-latest-dispatch-status",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "dispatch_status".to_string()));
        command
            .structured_args
            .push(("dispatch_selector".to_string(), "latest".to_string()));
        if let Some(agent_name) = &agent_name {
            command
                .structured_args
                .push(("agent".to_string(), agent_name.clone()));
        }
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::DispatchStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectLatestDispatchStatus { agent_name },
                required_fields: vec![
                    "dispatch_plan_id",
                    "adapter_kind",
                    "agent_name",
                    "plan_status",
                    "dogfood_gate_status",
                    "latest_gate_status",
                    "latest_replay_appended_events",
                    "latest_execution_status",
                    "provider_cli_executed",
                    "next_action",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer latest dispatch-chain status from the shared project dashboard query."
                    .to_string(),
        };
    }

    if let Some(filter) = latest_connectivity_exposure_filter(&normalized) {
        let mut command = voice_command(
            "voice-latest-connectivity-exposure",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command.structured_args.push((
            "view".to_string(),
            "connectivity_exposure_status".to_string(),
        ));
        command
            .structured_args
            .push(("connectivity_selector".to_string(), "latest".to_string()));
        if let Some(owner_kind) = &filter.owner_kind {
            command
                .structured_args
                .push(("owner_kind".to_string(), owner_kind.clone()));
        }
        if let Some(owner_id) = &filter.owner_id {
            command
                .structured_args
                .push(("owner_id".to_string(), owner_id.clone()));
        }
        if let Some(channel_kind) = &filter.channel_kind {
            command
                .structured_args
                .push(("channel".to_string(), channel_kind.clone()));
        }
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::ConnectivityStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectLatestConnectivityExposure {
                    owner_kind: filter.owner_kind,
                    owner_id: filter.owner_id,
                    channel_kind: filter.channel_kind,
                },
                required_fields: vec![
                    "exposure_id",
                    "endpoint",
                    "owner",
                    "channel",
                    "exposure_scope",
                    "permission_scope",
                    "status",
                    "health",
                    "reachable",
                    "grant",
                    "revoked_at",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer latest connectivity exposure status from shared dashboard read models."
                    .to_string(),
        };
    }

    if let Some(runtime_target_id) = runtime_target_status_id(&normalized) {
        let mut command = voice_command(
            "voice-runtime-target-status",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "runtime_target_status".to_string()));
        command
            .structured_args
            .push(("runtime_target".to_string(), runtime_target_id.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RuntimeTargetStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectRuntimeTargetStatus { runtime_target_id },
                required_fields: vec![
                    "runtime_target_id",
                    "name",
                    "runner",
                    "workspace",
                    "artifacts",
                    "default_cwd",
                    "capability_profile",
                    "endpoint",
                    "status",
                    "updated_sequence",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint: "Answer runtime target status from shared dashboard read models."
                .to_string(),
        };
    }

    if is_next_work_question(&normalized) {
        let mut command = voice_command(
            "voice-next-work",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "next_work".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::NextWork,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectNextWork,
                required_fields: vec![
                    "workpad_tasks",
                    "next_workpad_task",
                    "candidate_count",
                    "source_anchor",
                    "observed_status",
                    "capo_execution_status",
                    "default_task_id",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer the next workpad task from shared dashboard workpad read models."
                    .to_string(),
        };
    }

    if let Some(agent_name) = start_next_work_agent(&normalized) {
        let mut command = voice_command(
            "voice-start-next-work",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::SendTask,
            Some("voice start next work requested".to_string()),
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        command
            .structured_args
            .push(("action".to_string(), "start_next_workpad".to_string()));
        command
            .structured_args
            .push(("view".to_string(), "next_work".to_string()));
        command.risk = RiskLevel::Medium;
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::StartNextWork,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectNextWork,
                required_fields: vec![
                    "agent",
                    "workpad_tasks",
                    "next_workpad_task",
                    "candidate_count",
                    "source_anchor",
                    "observed_status",
                    "capo_execution_status",
                    "default_task_id",
                    "session_id",
                    "run_id",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: true,
            assistant_reply_hint:
                "Ask for visible confirmation before importing and starting the next workpad task."
                    .to_string(),
        };
    }

    if is_project_recent_work_question(&normalized) {
        let mut command = voice_command(
            "voice-recent-work",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "recent_work".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RecentWork,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectRecentWork,
                required_fields: vec![
                    "agents",
                    "sessions",
                    "latest_summary",
                    "evidence_refs",
                    "tool_calls",
                    "tool_observations",
                    "recent_events",
                    "project_evidence",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Summarize what agents have done from shared read models and evidence refs."
                    .to_string(),
        };
    }

    if is_project_tool_activity_question(&normalized) {
        let mut command = voice_command(
            "voice-tool-activity",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "tool_activity".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::ToolActivity,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectToolActivity,
                required_fields: vec!["agents", "tool_calls", "tool_observations"],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Summarize governed and observed-only tool activity from shared read models."
                    .to_string(),
        };
    }

    if is_review_needs_question(&normalized) {
        let mut command = voice_command(
            "voice-review-needs",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "review_needs".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::ReviewNeeds,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectReviewNeeds,
                required_fields: vec![
                    "review_findings",
                    "open_review_findings",
                    "review_blockers",
                    "task_outcome_reports",
                    "reports_with_findings",
                    "latest_review_outcome",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Summarize review blockers and task outcomes from shared dashboard read models."
                    .to_string(),
        };
    }

    if let Some(agent_name) = tool_activity_agent(&normalized) {
        let mut command = voice_command(
            "voice-agent-tool-activity",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        command
            .structured_args
            .push(("view".to_string(), "tool_activity".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::ToolActivity,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::AgentToolActivity { agent_name },
                required_fields: vec!["agent_status", "tool_calls", "tool_observations"],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Summarize the agent's governed and observed-only tool activity from read models."
                    .to_string(),
        };
    }

    if let Some(agent_name) = recent_work_agent(&normalized) {
        let mut command = voice_command(
            "voice-agent-recent-work",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        command
            .structured_args
            .push(("view".to_string(), "recent_work".to_string()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RecentWork,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::Agent { agent_name },
                required_fields: vec![
                    "agent_status",
                    "session_status",
                    "run_status",
                    "current_goal",
                    "latest_summary",
                    "evidence_refs",
                    "tool_calls",
                    "tool_observations",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Summarize the agent's recent work from session read models and evidence refs."
                    .to_string(),
        };
    }

    if let Some(agent_name) = status_agent(&normalized) {
        let mut command = voice_command(
            "voice-agent-status",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::AgentStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::Agent { agent_name },
                required_fields: vec![
                    "agent_status",
                    "session_status",
                    "run_status",
                    "current_goal",
                    "latest_summary",
                    "blocker",
                    "confidence",
                    "evidence_refs",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint: "Answer from the agent/session read models only.".to_string(),
        };
    }

    if let Some((agent_name, goal)) = redirect_agent(&normalized) {
        let mut command = voice_command(
            "voice-redirect",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::RedirectSession,
            Some(goal.clone()),
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RedirectSession,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::SessionForAgent { agent_name },
                required_fields: vec![
                    "session_id",
                    "current_goal",
                    "latest_summary",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint: "Confirm the steering command and summarize the updated goal."
                .to_string(),
        };
    }

    if let Some((agent_name, reason)) = stop_agent(&normalized) {
        let mut command = voice_command(
            "voice-stop",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::InterruptSession,
            Some(reason),
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        command.risk = RiskLevel::Medium;
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::StopSession,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::SessionForAgent { agent_name },
                required_fields: vec![
                    "session_id",
                    "session_status",
                    "run_status",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: true,
            assistant_reply_hint: "Ask for visible confirmation before stopping the session."
                .to_string(),
        };
    }

    if let Some((agent_name, reason)) = interrupt_agent(&normalized) {
        let mut command = voice_command(
            "voice-interrupt",
            &input,
            CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
            CommandIntent::InterruptSession,
            Some(reason),
        );
        command
            .structured_args
            .push(("agent".to_string(), agent_name.clone()));
        command.risk = RiskLevel::Medium;
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::InterruptSession,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::SessionForAgent { agent_name },
                required_fields: vec![
                    "session_id",
                    "session_status",
                    "run_status",
                    "recent_events",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: true,
            assistant_reply_hint: "Ask for visible confirmation before interrupting the session."
                .to_string(),
        };
    }

    VoiceCommandPlan {
        intent_kind: VoiceIntentKind::Unknown,
        command: None,
        read_contract: VoiceReadContract {
            query_scope: VoiceReadScope::None,
            required_fields: Vec::new(),
        },
        transcript_policy: policy,
        requires_visible_confirmation: false,
        assistant_reply_hint: "Ask a clarifying question; do not mutate Capo state.".to_string(),
    }
}

fn voice_command(
    slug: &str,
    input: &VoiceTranscriptInput,
    target: CommandTarget,
    intent: CommandIntent,
    text: Option<String>,
) -> CommandEnvelope {
    let mut command = CommandEnvelope::new(
        CommandId::new(format!("cmd-{slug}-{}", input.voice_session_id)),
        InputOrigin::Voice,
        input.actor_id.clone(),
        input.project_id.clone(),
        target,
        intent,
    );
    if let Some(text) = text {
        command = command.with_text(text);
    }
    command.structured_args.push((
        "voice_session_id".to_string(),
        input.voice_session_id.clone(),
    ));
    command.structured_args.push((
        "transcript_retention".to_string(),
        "raw_not_retained".to_string(),
    ));
    command
}

fn transcript_policy(retention_policy: TranscriptRetentionPolicy) -> VoiceTranscriptPolicy {
    match retention_policy {
        TranscriptRetentionPolicy::DoNotRetainRaw => VoiceTranscriptPolicy {
            retain_raw_transcript: false,
            redaction_required: true,
            memory_ingestion: MemoryIngestionPolicy::None,
            audit_note: "store normalized command plus voice-derived marker; do not retain raw transcript",
        },
        TranscriptRetentionPolicy::RetainRedactedSummary => VoiceTranscriptPolicy {
            retain_raw_transcript: false,
            redaction_required: true,
            memory_ingestion: MemoryIngestionPolicy::ReviewedRedactedSummaryOnly,
            audit_note: "store reviewed redacted summary only; raw transcript remains transient",
        },
    }
}

fn status_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is ")
        .and_then(|rest| rest.strip_suffix(" doing"))
        .or_else(|| normalized.strip_prefix("status for "))
        .map(agent_slug)
}

fn recent_work_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what has ")
        .and_then(|rest| rest.strip_suffix(" done"))
        .or_else(|| {
            normalized
                .strip_prefix("what did ")
                .and_then(|rest| rest.strip_suffix(" do"))
        })
        .map(agent_slug)
}

fn start_next_work_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("start next task with ")
        .or_else(|| normalized.strip_prefix("start the next task with "))
        .or_else(|| normalized.strip_prefix("start next work with "))
        .or_else(|| {
            normalized
                .strip_prefix("have ")
                .and_then(|rest| rest.strip_suffix(" start next task"))
        })
        .map(agent_slug)
}

fn redirect_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("steer ")?;
    let (agent, goal) = rest.split_once(" to ")?;
    Some((agent_slug(agent), goal.trim().to_string()))
}

fn stop_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("stop ")?;
    let (agent, reason) = rest
        .split_once(" because ")
        .map(|(agent, reason)| (agent, reason.to_string()))
        .unwrap_or((rest, "voice stop requested".to_string()));
    Some((agent_slug(agent), reason.trim().to_string()))
}

fn interrupt_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("interrupt ")?;
    let (agent, reason) = rest
        .split_once(" because ")
        .map(|(agent, reason)| (agent, reason.to_string()))
        .unwrap_or((rest, "voice interrupt requested".to_string()));
    Some((agent_slug(agent), reason.trim().to_string()))
}

fn is_dogfood_readiness_question(input: &str) -> bool {
    matches!(
        input,
        "are we ready to dogfood"
            | "are we ready for dogfood"
            | "can we dogfood capo"
            | "can capo dogfood itself"
            | "is capo ready to dogfood"
            | "what is dogfood readiness"
    )
}

fn is_next_work_question(input: &str) -> bool {
    matches!(
        input,
        "what should we do next"
            | "what is next"
            | "what's next"
            | "what is the next task"
            | "what should capo do next"
            | "show next work"
    )
}

fn dispatch_status_plan(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is dispatch status for ")
        .or_else(|| normalized.strip_prefix("what's dispatch status for "))
        .or_else(|| normalized.strip_prefix("show dispatch status for "))
        .or_else(|| normalized.strip_prefix("dispatch status for "))
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(ToString::to_string)
}

fn latest_dispatch_status_agent(normalized: &str) -> Option<Option<String>> {
    if matches!(
        normalized,
        "what is the latest dispatch status"
            | "what's the latest dispatch status"
            | "show latest dispatch status"
            | "latest dispatch status"
    ) {
        return Some(None);
    }

    normalized
        .strip_prefix("what is the latest dispatch status for ")
        .or_else(|| normalized.strip_prefix("what's the latest dispatch status for "))
        .or_else(|| normalized.strip_prefix("show latest dispatch status for "))
        .or_else(|| normalized.strip_prefix("latest dispatch status for "))
        .map(agent_slug)
        .filter(|agent_name| !agent_name.is_empty())
        .map(Some)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectivityExposureVoiceFilter {
    pub owner_kind: Option<String>,
    pub owner_id: Option<String>,
    pub channel_kind: Option<String>,
}

fn latest_connectivity_exposure_filter(
    normalized: &str,
) -> Option<ConnectivityExposureVoiceFilter> {
    if matches!(
        normalized,
        "what is latest connectivity exposure status"
            | "what is the latest connectivity exposure status"
            | "what's latest connectivity exposure status"
            | "what's the latest connectivity exposure status"
            | "show latest connectivity exposure status"
            | "latest connectivity exposure status"
            | "what is latest remote control exposure"
            | "what is the latest remote control exposure"
            | "show latest remote control exposure"
            | "latest remote control exposure"
    ) {
        return Some(ConnectivityExposureVoiceFilter::default());
    }

    let rest = normalized
        .strip_prefix("what is latest connectivity exposure status for ")
        .or_else(|| normalized.strip_prefix("what is the latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what's latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what's the latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("show latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what is latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("what is the latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("show latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("latest remote control exposure for "))?
        .trim();

    if let Some(owner_id) = rest.strip_prefix("runtime target ") {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: Some("runtime_target".to_string()),
            owner_id: Some(agent_slug(owner_id)),
            channel_kind: None,
        });
    }
    if let Some(owner_id) = rest.strip_prefix("capo server ") {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: Some("capo_server".to_string()),
            owner_id: Some(agent_slug(owner_id)),
            channel_kind: None,
        });
    }
    if matches!(
        rest,
        "control" | "stdio" | "logs" | "dashboard" | "artifact"
    ) {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: None,
            owner_id: None,
            channel_kind: Some(rest.to_string()),
        });
    }

    None
}

fn runtime_target_status_id(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is the runtime target status for ")
        .or_else(|| normalized.strip_prefix("what's the runtime target status for "))
        .or_else(|| normalized.strip_prefix("show runtime target status for "))
        .or_else(|| normalized.strip_prefix("runtime target status for "))
        .or_else(|| normalized.strip_prefix("what is the status of runtime target "))
        .or_else(|| normalized.strip_prefix("what's the status of runtime target "))
        .map(agent_slug)
        .filter(|runtime_target_id| !runtime_target_id.is_empty())
}

fn is_project_recent_work_question(input: &str) -> bool {
    matches!(
        input,
        "what have my agents done"
            | "what have the agents done"
            | "what did my agents do"
            | "what did the agents do"
            | "what has the team done"
            | "summarize agent work"
    )
}

fn is_project_tool_activity_question(input: &str) -> bool {
    matches!(
        input,
        "what tools have my agents used"
            | "what tools have the agents used"
            | "what tools did my agents use"
            | "what tools did the agents use"
            | "show agent tool activity"
            | "show tool activity"
    )
}

fn tool_activity_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what tools has ")
        .and_then(|rest| rest.strip_suffix(" used"))
        .or_else(|| {
            normalized
                .strip_prefix("what tools did ")
                .and_then(|rest| rest.strip_suffix(" use"))
        })
        .or_else(|| normalized.strip_prefix("show tool activity for "))
        .map(agent_slug)
}

fn is_review_needs_question(input: &str) -> bool {
    matches!(
        input,
        "what needs review"
            | "what are the review blockers"
            | "show review blockers"
            | "what outcomes need attention"
            | "what needs attention"
            | "summarize review blockers"
    )
}

fn normalize(input: &str) -> String {
    input
        .trim()
        .trim_end_matches(['.', '?', '!'])
        .to_ascii_lowercase()
}

fn agent_slug(input: &str) -> String {
    input
        .trim()
        .trim_start_matches("agent ")
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch == '-' || ch == '_' || ch.is_ascii_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_question_lowers_to_voice_query_command_and_read_contract() {
        let plan = plan_dummy_transcript(input("What is fake-codex doing?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::AgentStatus);
        assert!(!plan.requires_visible_confirmation);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::Agent {
                agent_name: "fake-codex".to_string()
            }
        );
        assert!(plan.read_contract.required_fields.contains(&"current_goal"));
        let command = plan.command.expect("voice command");
        assert_eq!(command.origin, InputOrigin::Voice);
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Agent(AgentId::new("agent-fake-codex"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "transcript_retention")
                .map(|(_, value)| value.as_str()),
            Some("raw_not_retained")
        );
    }

    #[test]
    fn steering_transcript_lowers_to_redirect_without_raw_retention() {
        let plan = plan_dummy_transcript(input(
            "Steer fake-reviewer to focus only on dogfood blockers.",
        ));

        assert_eq!(plan.intent_kind, VoiceIntentKind::RedirectSession);
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        assert_eq!(
            plan.transcript_policy.memory_ingestion,
            MemoryIngestionPolicy::None
        );
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::RedirectSession);
        assert_eq!(
            command.text.as_deref(),
            Some("focus only on dogfood blockers")
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "agent")
                .map(|(_, value)| value.as_str()),
            Some("fake-reviewer")
        );
    }

    #[test]
    fn stop_transcript_requires_visible_confirmation() {
        let plan = plan_dummy_transcript(input("Stop fake-codex because smoke is done"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::StopSession);
        assert!(plan.requires_visible_confirmation);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::InterruptSession);
        assert_eq!(command.risk, RiskLevel::Medium);
        assert_eq!(command.text.as_deref(), Some("smoke is done"));
    }

    #[test]
    fn interrupt_transcript_requires_visible_confirmation() {
        let plan = plan_dummy_transcript(input("Interrupt fake-codex because output is stale"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::InterruptSession);
        assert!(plan.requires_visible_confirmation);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::InterruptSession);
        assert_eq!(command.risk, RiskLevel::Medium);
        assert_eq!(command.text.as_deref(), Some("output is stale"));
    }

    #[test]
    fn dashboard_question_reads_project_dashboard_without_mutation() {
        let plan = plan_dummy_transcript(input("What are my agents doing?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::DashboardSummary);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectDashboard
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"recent_events")
        );
        assert_eq!(
            plan.command.expect("voice command").intent,
            CommandIntent::QueryStatus
        );
    }

    #[test]
    fn dogfood_readiness_question_reads_project_readiness_without_mutation() {
        let plan = plan_dummy_transcript(input("Are we ready to dogfood?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::DogfoodReadiness);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectDogfoodReadiness
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"real_agent_connector_ready")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("dogfood_readiness")
        );
    }

    #[test]
    fn dispatch_status_question_reads_dispatch_status_without_mutation() {
        let plan = plan_dummy_transcript(input(
            "What is dispatch status for adapter-dispatch-plan-codex?",
        ));

        assert_eq!(plan.intent_kind, VoiceIntentKind::DispatchStatus);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectDispatchStatus {
                dispatch_plan_id: "adapter-dispatch-plan-codex".to_string()
            }
        );
        assert!(plan.read_contract.required_fields.contains(&"next_action"));
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("dispatch_status")
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "dispatch_plan")
                .map(|(_, value)| value.as_str()),
            Some("adapter-dispatch-plan-codex")
        );
    }

    #[test]
    fn latest_dispatch_status_question_reads_latest_dispatch_without_mutation() {
        let plan = plan_dummy_transcript(input("What is the latest dispatch status?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::DispatchStatus);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectLatestDispatchStatus { agent_name: None }
        );
        assert!(plan.read_contract.required_fields.contains(&"agent_name"));
        assert!(!plan.requires_visible_confirmation);
        let command = plan.command.expect("voice command");
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "dispatch_selector")
                .map(|(_, value)| value.as_str()),
            Some("latest")
        );

        let agent_plan = plan_dummy_transcript(input(
            "What is the latest dispatch status for codex-worker?",
        ));
        assert_eq!(
            agent_plan.read_contract.query_scope,
            VoiceReadScope::ProjectLatestDispatchStatus {
                agent_name: Some("codex-worker".to_string())
            }
        );
        let command = agent_plan.command.expect("voice command");
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "agent")
                .map(|(_, value)| value.as_str()),
            Some("codex-worker")
        );
    }

    #[test]
    fn latest_connectivity_status_question_reads_latest_exposure_without_mutation() {
        let plan = plan_dummy_transcript(input("What is the latest connectivity exposure status?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::ConnectivityStatus);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectLatestConnectivityExposure {
                owner_kind: None,
                owner_id: None,
                channel_kind: None,
            }
        );
        assert!(plan.read_contract.required_fields.contains(&"status"));
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"permission_scope")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "connectivity_selector")
                .map(|(_, value)| value.as_str()),
            Some("latest")
        );

        let owner_plan = plan_dummy_transcript(input(
            "What is the latest connectivity exposure status for runtime target remote-target-1?",
        ));
        assert_eq!(
            owner_plan.read_contract.query_scope,
            VoiceReadScope::ProjectLatestConnectivityExposure {
                owner_kind: Some("runtime_target".to_string()),
                owner_id: Some("remote-target-1".to_string()),
                channel_kind: None,
            }
        );
        let command = owner_plan.command.expect("voice command");
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "owner_kind")
                .map(|(_, value)| value.as_str()),
            Some("runtime_target")
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "owner_id")
                .map(|(_, value)| value.as_str()),
            Some("remote-target-1")
        );

        let channel_plan =
            plan_dummy_transcript(input("Latest remote control exposure for dashboard"));
        assert_eq!(
            channel_plan.read_contract.query_scope,
            VoiceReadScope::ProjectLatestConnectivityExposure {
                owner_kind: None,
                owner_id: None,
                channel_kind: Some("dashboard".to_string()),
            }
        );
    }

    #[test]
    fn runtime_target_status_question_reads_target_without_mutation() {
        let plan = plan_dummy_transcript(input(
            "What is the runtime target status for remote target 1?",
        ));

        assert_eq!(plan.intent_kind, VoiceIntentKind::RuntimeTargetStatus);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectRuntimeTargetStatus {
                runtime_target_id: "remote-target-1".to_string(),
            }
        );
        assert!(plan.read_contract.required_fields.contains(&"status"));
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"capability_profile")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("runtime_target_status")
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "runtime_target")
                .map(|(_, value)| value.as_str()),
            Some("remote-target-1")
        );
    }

    #[test]
    fn next_work_question_reads_project_workpad_queue_without_mutation() {
        let plan = plan_dummy_transcript(input("What should we do next?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::NextWork);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectNextWork
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"next_workpad_task")
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"candidate_count")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("next_work")
        );
    }

    #[test]
    fn start_next_work_requires_confirmation_and_uses_agent_target() {
        let plan = plan_dummy_transcript(input("Start next task with fake-codex."));

        assert_eq!(plan.intent_kind, VoiceIntentKind::StartNextWork);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectNextWork
        );
        assert!(plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::SendTask);
        assert_eq!(
            command.target,
            CommandTarget::Agent(AgentId::new("agent-fake-codex"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "agent")
                .map(|(_, value)| value.as_str()),
            Some("fake-codex")
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "action")
                .map(|(_, value)| value.as_str()),
            Some("start_next_workpad")
        );
        assert_eq!(
            command.text.as_deref(),
            Some("voice start next work requested")
        );
    }

    #[test]
    fn project_recent_work_question_reads_project_work_without_mutation() {
        let plan = plan_dummy_transcript(input("What have my agents done?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::RecentWork);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectRecentWork
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"latest_summary")
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"project_evidence")
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"tool_observations")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("recent_work")
        );
    }

    #[test]
    fn agent_recent_work_question_reads_agent_work_without_mutation() {
        let plan = plan_dummy_transcript(input("What has fake-codex done?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::RecentWork);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::Agent {
                agent_name: "fake-codex".to_string()
            }
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"latest_summary")
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"tool_observations")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Agent(AgentId::new("agent-fake-codex"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("recent_work")
        );
    }

    #[test]
    fn tool_activity_questions_read_tool_activity_without_mutation() {
        let project_plan = plan_dummy_transcript(input("What tools have my agents used?"));

        assert_eq!(project_plan.intent_kind, VoiceIntentKind::ToolActivity);
        assert_eq!(
            project_plan.read_contract.query_scope,
            VoiceReadScope::ProjectToolActivity
        );
        assert!(
            project_plan
                .read_contract
                .required_fields
                .contains(&"tool_calls")
        );
        assert!(
            project_plan
                .read_contract
                .required_fields
                .contains(&"tool_observations")
        );
        assert!(!project_plan.requires_visible_confirmation);
        assert!(!project_plan.transcript_policy.retain_raw_transcript);
        let command = project_plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("tool_activity")
        );

        let agent_plan = plan_dummy_transcript(input("What tools has fake-codex used?"));
        assert_eq!(agent_plan.intent_kind, VoiceIntentKind::ToolActivity);
        assert_eq!(
            agent_plan.read_contract.query_scope,
            VoiceReadScope::AgentToolActivity {
                agent_name: "fake-codex".to_string()
            }
        );
        assert!(!agent_plan.requires_visible_confirmation);
        let command = agent_plan.command.expect("voice command");
        assert_eq!(
            command.target,
            CommandTarget::Agent(AgentId::new("agent-fake-codex"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("tool_activity")
        );
    }

    #[test]
    fn review_needs_question_reads_review_and_outcome_state_without_mutation() {
        let plan = plan_dummy_transcript(input("What needs review?"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::ReviewNeeds);
        assert_eq!(
            plan.read_contract.query_scope,
            VoiceReadScope::ProjectReviewNeeds
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"review_blockers")
        );
        assert!(
            plan.read_contract
                .required_fields
                .contains(&"task_outcome_reports")
        );
        assert!(!plan.requires_visible_confirmation);
        assert!(!plan.transcript_policy.retain_raw_transcript);
        let command = plan.command.expect("voice command");
        assert_eq!(command.intent, CommandIntent::QueryStatus);
        assert_eq!(
            command.target,
            CommandTarget::Project(ProjectId::new("project-capo"))
        );
        assert_eq!(
            command
                .structured_args
                .iter()
                .find(|(key, _)| key == "view")
                .map(|(_, value)| value.as_str()),
            Some("review_needs")
        );
    }

    #[test]
    fn unknown_transcript_does_not_mutate_state_or_enter_memory() {
        let plan = plan_dummy_transcript(input("Maybe later, never mind"));

        assert_eq!(plan.intent_kind, VoiceIntentKind::Unknown);
        assert_eq!(plan.command, None);
        assert_eq!(plan.read_contract.query_scope, VoiceReadScope::None);
        assert_eq!(
            plan.transcript_policy.memory_ingestion,
            MemoryIngestionPolicy::None
        );
    }

    fn input(transcript_text: &str) -> VoiceTranscriptInput {
        VoiceTranscriptInput {
            voice_session_id: "voice-session-1".to_string(),
            actor_id: "local-user".to_string(),
            project_id: ProjectId::new("project-capo"),
            transcript_text: transcript_text.to_string(),
            asr_confidence: Some(91),
            retention_policy: VOICE_TRANSCRIPT_RETENTION_DEFAULT,
        }
    }
}
