use capo_core::{CommandEnvelope, SessionId};
use capo_query::{ProjectDashboard, ProjectDashboardQuery, project_dashboard};
use capo_state::{
    CapabilityGrantProjection, EventKind, MemoryRecordProjection, MemorySourceProjection, NewEvent,
    PermissionApprovalProjection, ProjectionRecord, RedactionState,
};
use capo_voice::{
    MemoryIngestionPolicy, TranscriptRetentionPolicy, VOICE_TRANSCRIPT_RETENTION_DEFAULT,
    VoiceCommandPlan, VoiceIntentKind, VoiceReadScope, VoiceTranscriptInput, plan_dummy_transcript,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::permission::{approval_decision_effect, approval_subject_json};
use crate::voice_render::{
    render_voice_approval, render_voice_header, render_voice_memory_retention,
    render_voice_read_contract, voice_intent_label,
};
use crate::workpad::start_next_workpad_task;
use crate::{controller, debug_error, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn submit_voice(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let transcript = required_arg(args, "--transcript")?;
    let voice_session_id =
        optional_arg(args, "--voice-session").unwrap_or_else(|| "voice-session-cli".to_string());
    let actor_id = optional_arg(args, "--actor").unwrap_or_else(|| "local-user".to_string());
    let confirmed = has_flag(args, "--confirm");
    let redacted_summary = optional_arg(args, "--redacted-summary");
    let retention_policy = if redacted_summary.is_some() {
        TranscriptRetentionPolicy::RetainRedactedSummary
    } else {
        VOICE_TRANSCRIPT_RETENTION_DEFAULT
    };
    if redacted_summary.is_some() && !has_flag(args, "--reviewed-summary") {
        return Err(
            "--redacted-summary requires --reviewed-summary before memory ingestion".to_string(),
        );
    }
    let plan = plan_dummy_transcript(VoiceTranscriptInput {
        voice_session_id,
        actor_id,
        project_id: project_id(),
        transcript_text: transcript,
        asr_confidence: None,
        retention_policy,
    });

    let retained_memory = if let Some(summary) = redacted_summary {
        Some(ingest_reviewed_voice_summary(parsed, &plan, &summary)?)
    } else {
        None
    };

    if plan.intent_kind == VoiceIntentKind::Unknown {
        let mut output = render_voice_header(&plan, None, false, false);
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }
    if plan.requires_visible_confirmation && !confirmed {
        let approval = queue_voice_permission_approval(parsed, &plan)?;
        let mut output = render_voice_header(&plan, plan.command.as_ref(), true, false);
        output.push_str(&render_voice_approval(&approval, None));
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }

    let mut output = render_voice_header(&plan, plan.command.as_ref(), false, false);
    match plan.intent_kind {
        VoiceIntentKind::AdapterSmokeStatus
        | VoiceIntentKind::ConnectivityStatus
        | VoiceIntentKind::DashboardSummary
        | VoiceIntentKind::DispatchStatus
        | VoiceIntentKind::DogfoodReadiness
        | VoiceIntentKind::NextWork
        | VoiceIntentKind::RecentWork
        | VoiceIntentKind::ReviewNeeds
        | VoiceIntentKind::RuntimeTargetReadiness
        | VoiceIntentKind::RuntimeTargetStatus
        | VoiceIntentKind::ToolActivity
        | VoiceIntentKind::AgentStatus => {
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::RedirectSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice redirect plan missing command".to_string())?;
            controller(parsed)?
                .redirect_command(command)
                .map_err(debug_error)?;
            output = render_voice_header(&plan, Some(command), false, true);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::StartNextWork => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice start-next plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let agent = structured_arg_value(command, "agent")
                .ok_or_else(|| "voice start-next plan missing agent".to_string())?
                .to_string();
            let started = start_next_workpad_task(parsed, &["--agent".to_string(), agent.clone()])?;
            output = render_voice_header(&plan, Some(command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!("controlled_agent={agent}\n"));
            output.push_str(&started);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::InterruptSession | VoiceIntentKind::StopSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice session-control plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let durable_reason = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => "voice interrupt confirmed",
                VoiceIntentKind::StopSession => "voice stop confirmed",
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            let command = CommandEnvelope {
                text: Some(durable_reason.to_string()),
                ..command.clone()
            };
            let controller = controller(parsed)?;
            let observation = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => controller
                    .interrupt_command(&command)
                    .map_err(debug_error)?,
                VoiceIntentKind::StopSession => {
                    controller.stop_command(&command).map_err(debug_error)?
                }
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            output = render_voice_header(&plan, Some(&command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!(
                "controlled_session={} session_status={} run_status={}\n",
                observation.session.session_id, observation.session.status, observation.run.status
            ));
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::Unknown => {}
    }
    if let Some(record) = &retained_memory {
        output.push_str(&render_voice_memory_retention(record));
    }
    Ok(output)
}

fn ingest_reviewed_voice_summary(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    redacted_summary: &str,
) -> Result<MemoryRecordProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "reviewed voice summary requires a planned command".to_string())?;
    if redacted_summary.trim().is_empty() {
        return Err("--redacted-summary cannot be empty".to_string());
    }
    if !plan.transcript_policy.redaction_required
        || plan.transcript_policy.memory_ingestion
            != MemoryIngestionPolicy::ReviewedRedactedSummaryOnly
    {
        return Err(
            "voice summary memory ingestion requires reviewed redacted summary policy".to_string(),
        );
    }
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let summary_hash = stable_cli_hash(redacted_summary);
    let record_id = format!(
        "memory-voice-summary-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}",
            command.project_id, voice_session_id, summary_hash
        ))
    );
    let source_id = format!("source-{record_id}");
    let event_id = format!("event-memory-voice-summary-{summary_hash}");
    let record = MemoryRecordProjection {
        memory_record_id: record_id.clone(),
        project_id: command.project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: command.project_id.to_string(),
        subject_ref: Some(voice_session_id.clone()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "summary".to_string(),
        subject: "voice_conversation".to_string(),
        predicate: "retained_reviewed_summary".to_string(),
        object: voice_intent_label(plan.intent_kind).to_string(),
        body: redacted_summary.trim().to_string(),
        confidence: "medium".to_string(),
        review_state: "reviewed".to_string(),
        source_count: 1,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Redacted.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{record_id}")),
        updated_sequence: 0,
    };
    let source = MemorySourceProjection {
        memory_source_id: source_id,
        memory_record_id: record_id.clone(),
        source_kind: "event".to_string(),
        source_event_id: Some(event_id.clone()),
        source_artifact_id: None,
        source_path: None,
        source_anchor: Some("voice:redacted-summary".to_string()),
        source_content_hash: Some(summary_hash.to_string()),
        source_sequence: None,
        quote_artifact_id: None,
        observed_at: Some("cli".to_string()),
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(event_id, EventKind::MemoryRecordIngested, "capo-voice");
    event.project_id = Some(command.project_id.clone());
    event.payload_json = format!(
        "{{\"memory_record_id\":\"{}\",\"origin\":\"voice\",\"review_state\":\"reviewed\",\"redaction_state\":\"redacted\",\"voice_session_id\":\"{}\",\"intent\":\"{}\",\"summary_hash\":{}}}",
        escape_json(&record.memory_record_id),
        escape_json(&voice_session_id),
        voice_intent_label(plan.intent_kind),
        summary_hash
    );
    event.idempotency_key = Some(format!("voice-summary-memory:{record_id}"));
    event.redaction_state = RedactionState::Redacted;
    state(parsed)?
        .append_event(
            event,
            &[
                ProjectionRecord::MemoryRecord(Box::new(record.clone())),
                ProjectionRecord::MemorySource(source),
            ],
        )
        .map_err(debug_error)?;
    Ok(record)
}

fn queue_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let approval = voice_permission_approval(parsed, plan)?;
    let state = state(parsed)?;
    if let Some(existing) = state
        .permission_approval(&approval.project_id, &approval.approval_id)
        .map_err(debug_error)?
    {
        return Ok(existing);
    }
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-approval-queued-{}",
            stable_cli_hash(&approval.approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!(
        "voice-permission-approval:{}",
        approval.approval_id
    ));
    event.redaction_state = RedactionState::Safe;
    state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(approval)
}

fn decide_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    decision: &str,
) -> Result<PermissionApprovalProjection, String> {
    let approval = queue_voice_permission_approval(parsed, plan)?;
    if approval.status == "decided" {
        return Ok(approval);
    }
    let (effect, persistence) = approval_decision_effect(decision)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-voice-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json,
        decision_source: "user_visible_voice_confirmation".to_string(),
        persistence: persistence.to_string(),
        explanation: format!(
            "visible voice confirmation {decision} for {}",
            approval.approval_id
        ),
        updated_sequence: 0,
    };
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.to_string()),
        capability_grant_id: Some(grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-decided-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::PermissionDecided,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(decision),
        escape_json(&grant.capability_grant_id),
        effect,
        persistence
    );
    event.redaction_state = RedactionState::Safe;
    let mut grant_event = NewEvent::new(
        format!(
            "event-voice-capability-grant-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::CapabilityGrantCreated,
        "capo-voice",
    );
    grant_event.project_id = Some(approval.project_id.clone());
    grant_event.session_id = approval.session_id.clone();
    grant_event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&grant.capability_grant_id),
        escape_json(&grant.effect),
        escape_json(&grant.decision_source),
        escape_json(&grant.persistence)
    );
    grant_event.redaction_state = RedactionState::Safe;
    state(parsed)?
        .decide_permission_approval(
            &approval.approval_id,
            event,
            Some(grant_event),
            decided_approval.clone(),
            Some(grant),
        )
        .map_err(debug_error)?;
    Ok(decided_approval)
}

fn voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice privileged plan missing command".to_string())?;
    let session_id = voice_session_id(parsed, plan)?;
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let approval_id = format!(
        "approval-voice-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}",
            command.command_id,
            command.actor_id,
            session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            voice_intent_label(plan.intent_kind)
        ))
    );
    Ok(PermissionApprovalProjection {
        approval_id,
        project_id: command.project_id.clone(),
        session_id,
        tool_call_id: None,
        capability_profile_id: "voice-control".to_string(),
        scope_json: "[\"voice:approve:privileged\"]".to_string(),
        subject_json: format!(
            "{{\"actor\":\"{}\",\"origin\":\"voice\",\"voice_session_id\":\"{}\",\"command_id\":\"{}\",\"intent\":\"{}\"}}",
            escape_json(&command.actor_id),
            escape_json(&voice_session_id),
            escape_json(command.command_id.as_str()),
            voice_intent_label(plan.intent_kind)
        ),
        status: "pending".to_string(),
        requested_by: format!("voice:{}", command.actor_id),
        reason: format!(
            "visible confirmation required for {}",
            voice_intent_label(plan.intent_kind)
        ),
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    })
}

fn voice_session_id(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<Option<SessionId>, String> {
    match &plan.read_contract.query_scope {
        VoiceReadScope::SessionForAgent { agent_name } | VoiceReadScope::Agent { agent_name } => {
            let dashboard = voice_dashboard(parsed, plan)?;
            Ok(dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
                .and_then(|row| row.session.as_ref())
                .map(|row| row.session.session_id.clone()))
        }
        VoiceReadScope::ProjectDashboard
        | VoiceReadScope::ProjectLatestConnectivityExposure { .. }
        | VoiceReadScope::ProjectRuntimeTargetStatus { .. }
        | VoiceReadScope::ProjectRuntimeTargetControlReadiness { .. }
        | VoiceReadScope::ProjectLatestRuntimeTargetStatus { .. }
        | VoiceReadScope::ProjectAdapterSmokeReportStatus { .. }
        | VoiceReadScope::ProjectLatestAdapterSmokeReport { .. }
        | VoiceReadScope::ProjectDispatchStatus { .. }
        | VoiceReadScope::ProjectLatestDispatchStatus { .. }
        | VoiceReadScope::ProjectDogfoodReadiness
        | VoiceReadScope::ProjectNextWork
        | VoiceReadScope::ProjectRecentWork
        | VoiceReadScope::ProjectReviewNeeds
        | VoiceReadScope::ProjectToolActivity
        | VoiceReadScope::AgentToolActivity { .. }
        | VoiceReadScope::None => Ok(None),
    }
}

fn voice_dashboard(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<ProjectDashboard, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice plan missing query command".to_string())?;
    project_dashboard(
        &state(parsed)?,
        ProjectDashboardQuery::new(command.project_id.clone()),
    )
    .map_err(debug_error)
}

fn structured_arg_value<'a>(command: &'a CommandEnvelope, key: &str) -> Option<&'a str> {
    command
        .structured_args
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
}
