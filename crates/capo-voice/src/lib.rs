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
    DashboardSummary,
    DogfoodReadiness,
    RedirectSession,
    InterruptSession,
    StopSession,
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
    ProjectDogfoodReadiness,
    Agent { agent_name: String },
    SessionForAgent { agent_name: String },
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
