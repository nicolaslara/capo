//! Conversational voice control contract for Capo.
//!
//! P14 deliberately avoids audio capture or ASR integration. This crate takes
//! already-produced dummy transcript text and lowers it into the same command
//! envelope/read-model contract used by CLI and dashboard surfaces.

use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, RiskLevel,
};

mod contract;
mod planning_support;

pub use contract::*;
use planning_support::*;
pub use planning_support::{ConnectivityExposureVoiceFilter, RuntimeTargetVoiceFilter};

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
                    "runtime_target_ready",
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

    if let Some(smoke_report_id) = adapter_smoke_report_status_plan(&normalized) {
        let mut command = voice_command(
            "voice-adapter-smoke-status",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "adapter_smoke_status".to_string()));
        command
            .structured_args
            .push(("smoke_report".to_string(), smoke_report_id.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::AdapterSmokeStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectAdapterSmokeReportStatus { smoke_report_id },
                required_fields: vec![
                    "smoke_report_id",
                    "adapter_kind",
                    "smoke_status",
                    "credential_scan_status",
                    "marker_found",
                    "dogfood_readiness_effect",
                    "provider_cli_executed",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer adapter smoke-report status from the shared project dashboard query."
                    .to_string(),
        };
    }

    if let Some(adapter_kind) = latest_adapter_smoke_report_filter(&normalized) {
        let mut command = voice_command(
            "voice-latest-adapter-smoke-status",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "adapter_smoke_status".to_string()));
        command
            .structured_args
            .push(("smoke_selector".to_string(), "latest".to_string()));
        if let Some(adapter_kind) = &adapter_kind {
            command
                .structured_args
                .push(("adapter".to_string(), adapter_kind.clone()));
        }
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::AdapterSmokeStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectLatestAdapterSmokeReport { adapter_kind },
                required_fields: vec![
                    "smoke_report_id",
                    "adapter_kind",
                    "smoke_status",
                    "credential_scan_status",
                    "marker_found",
                    "dogfood_readiness_effect",
                    "provider_cli_executed",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer latest adapter smoke-report status from the shared project dashboard query."
                    .to_string(),
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

    if let Some(filter) = latest_runtime_target_status_filter(&normalized) {
        let mut command = voice_command(
            "voice-latest-runtime-target-status",
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
            .push(("runtime_target_selector".to_string(), "latest".to_string()));
        if let Some(runner_kind) = &filter.runner_kind {
            command
                .structured_args
                .push(("runner".to_string(), runner_kind.clone()));
        }
        if let Some(status) = &filter.status {
            command
                .structured_args
                .push(("status".to_string(), status.clone()));
        }
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RuntimeTargetStatus,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectLatestRuntimeTargetStatus {
                    runner_kind: filter.runner_kind,
                    status: filter.status,
                },
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
            assistant_reply_hint:
                "Answer latest runtime target status from shared dashboard read models.".to_string(),
        };
    }

    if let Some(runtime_target_id) = runtime_target_readiness_id(&normalized) {
        let mut command = voice_command(
            "voice-runtime-target-readiness",
            &input,
            CommandTarget::Project(input.project_id.clone()),
            CommandIntent::QueryStatus,
            None,
        );
        command
            .structured_args
            .push(("view".to_string(), "runtime_target_readiness".to_string()));
        command
            .structured_args
            .push(("runtime_target".to_string(), runtime_target_id.clone()));
        return VoiceCommandPlan {
            intent_kind: VoiceIntentKind::RuntimeTargetReadiness,
            command: Some(command),
            read_contract: VoiceReadContract {
                query_scope: VoiceReadScope::ProjectRuntimeTargetControlReadiness {
                    runtime_target_id,
                },
                required_fields: vec![
                    "runtime_target_id",
                    "runner",
                    "target_status",
                    "target_ready",
                    "control_exposure_ready",
                    "control_exposure_status",
                    "control_exposure_reachable",
                    "ready",
                    "blockers",
                    "next_action",
                ],
            },
            transcript_policy: policy,
            requires_visible_confirmation: false,
            assistant_reply_hint:
                "Answer runtime target control readiness from shared dashboard read models."
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

#[cfg(test)]
mod tests;
