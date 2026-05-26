use super::*;
use capo_core::ProjectId;

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
    assert!(
        plan.read_contract
            .required_fields
            .contains(&"runtime_target_ready")
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
fn adapter_smoke_status_questions_read_smoke_reports_without_mutation() {
    let plan = plan_dummy_transcript(input(
        "What is smoke report status for adapter-smoke-codex?",
    ));

    assert_eq!(plan.intent_kind, VoiceIntentKind::AdapterSmokeStatus);
    assert_eq!(
        plan.read_contract.query_scope,
        VoiceReadScope::ProjectAdapterSmokeReportStatus {
            smoke_report_id: "adapter-smoke-codex".to_string()
        }
    );
    assert!(
        plan.read_contract
            .required_fields
            .contains(&"credential_scan_status")
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
        Some("adapter_smoke_status")
    );
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "smoke_report")
            .map(|(_, value)| value.as_str()),
        Some("adapter-smoke-codex")
    );

    let latest = plan_dummy_transcript(input("What is the latest smoke report status?"));
    assert_eq!(latest.intent_kind, VoiceIntentKind::AdapterSmokeStatus);
    assert_eq!(
        latest.read_contract.query_scope,
        VoiceReadScope::ProjectLatestAdapterSmokeReport { adapter_kind: None }
    );
    let command = latest.command.expect("voice command");
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "smoke_selector")
            .map(|(_, value)| value.as_str()),
        Some("latest")
    );

    let latest_codex =
        plan_dummy_transcript(input("What is the latest smoke report status for Codex?"));
    assert_eq!(
        latest_codex.read_contract.query_scope,
        VoiceReadScope::ProjectLatestAdapterSmokeReport {
            adapter_kind: Some("codex_exec".to_string())
        }
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

    let channel_plan = plan_dummy_transcript(input("Latest remote control exposure for dashboard"));
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
    let latest = plan_dummy_transcript(input("What is the latest runtime target status?"));

    assert_eq!(latest.intent_kind, VoiceIntentKind::RuntimeTargetStatus);
    assert_eq!(
        latest.read_contract.query_scope,
        VoiceReadScope::ProjectLatestRuntimeTargetStatus {
            runner_kind: None,
            status: None,
        }
    );
    assert!(!latest.requires_visible_confirmation);
    assert!(!latest.transcript_policy.retain_raw_transcript);
    let command = latest.command.expect("voice command");
    assert_eq!(command.intent, CommandIntent::QueryStatus);
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "runtime_target_selector")
            .map(|(_, value)| value.as_str()),
        Some("latest")
    );

    let filtered = plan_dummy_transcript(input(
        "What is the latest runtime target status for available local process?",
    ));
    assert_eq!(
        filtered.read_contract.query_scope,
        VoiceReadScope::ProjectLatestRuntimeTargetStatus {
            runner_kind: Some("local-process".to_string()),
            status: Some("available".to_string()),
        }
    );
    let command = filtered.command.expect("voice command");
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "runner")
            .map(|(_, value)| value.as_str()),
        Some("local-process")
    );
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "status")
            .map(|(_, value)| value.as_str()),
        Some("available")
    );

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
fn runtime_target_readiness_question_reads_control_readiness_without_mutation() {
    let plan = plan_dummy_transcript(input(
        "Is runtime target remote target 1 ready for remote control?",
    ));

    assert_eq!(plan.intent_kind, VoiceIntentKind::RuntimeTargetReadiness);
    assert_eq!(
        plan.read_contract.query_scope,
        VoiceReadScope::ProjectRuntimeTargetControlReadiness {
            runtime_target_id: "remote-target-1".to_string(),
        }
    );
    assert!(
        plan.read_contract
            .required_fields
            .contains(&"control_exposure_ready")
    );
    assert!(plan.read_contract.required_fields.contains(&"next_action"));
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
        Some("runtime_target_readiness")
    );
    assert_eq!(
        command
            .structured_args
            .iter()
            .find(|(key, _)| key == "runtime_target")
            .map(|(_, value)| value.as_str()),
        Some("remote-target-1")
    );

    let alternate = plan_dummy_transcript(input(
        "What is runtime target readiness for remote target 1?",
    ));
    assert_eq!(
        alternate.read_contract.query_scope,
        VoiceReadScope::ProjectRuntimeTargetControlReadiness {
            runtime_target_id: "remote-target-1".to_string(),
        }
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
