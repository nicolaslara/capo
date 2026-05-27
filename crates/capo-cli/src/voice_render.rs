use capo_core::CommandEnvelope;
use capo_query::{AdapterDispatchStatus, ProjectDashboard, RuntimeTargetControlReadiness};
use capo_state::{
    AdapterSmokeReportProjection, ConnectivityExposureProjection, MemoryRecordProjection,
    PermissionApprovalProjection, RuntimeTargetProjection,
};
use capo_voice::{MemoryIngestionPolicy, VoiceCommandPlan, VoiceIntentKind, VoiceReadScope};

use crate::comma_or_none;
use crate::project_memory_flow::default_source_task_task_id;
use crate::workpad::default_workpad_task_id;

pub(crate) fn render_voice_approval(
    approval: &PermissionApprovalProjection,
    decision: Option<&str>,
) -> String {
    format!(
        "permission_approval={}\npermission_status={}\npermission_decision={}\npermission_scope={}\npermission_requested_by={}\npermission_reason={}\n",
        approval.approval_id,
        approval.status,
        decision.or(approval.decision.as_deref()).unwrap_or("none"),
        approval.scope_json,
        approval.requested_by,
        approval.reason
    )
}

pub(crate) fn render_voice_memory_retention(record: &MemoryRecordProjection) -> String {
    format!(
        "memory_record={}\nmemory_review_state={}\nmemory_redaction_state={}\nmemory_ingestion=reviewed_redacted_summary_only\n",
        record.memory_record_id, record.review_state, record.redaction_state
    )
}

pub(crate) fn render_voice_header(
    plan: &VoiceCommandPlan,
    command: Option<&CommandEnvelope>,
    confirmation_required: bool,
    mutation_applied: bool,
) -> String {
    format!(
        "voice_plan={}\norigin=voice\ncommand_id={}\nconfirmation_required={}\nmutation_applied={}\nraw_transcript_retained={}\nredaction_required={}\nmemory_ingestion={}\nassistant_reply_hint={}\n",
        voice_intent_label(plan.intent_kind),
        command
            .map(|command| command.command_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        confirmation_required,
        mutation_applied,
        plan.transcript_policy.retain_raw_transcript,
        plan.transcript_policy.redaction_required,
        memory_ingestion_label(plan.transcript_policy.memory_ingestion),
        plan.assistant_reply_hint
    )
}

pub(crate) fn render_voice_read_contract(
    plan: &VoiceCommandPlan,
    dashboard: &ProjectDashboard,
) -> String {
    let mut output = format!(
        "read_scope={}\nrequired_fields={}\n",
        voice_scope_label(&plan.read_contract.query_scope),
        plan.read_contract.required_fields.join(",")
    );
    match &plan.read_contract.query_scope {
        VoiceReadScope::ProjectDashboard => {
            output.push_str(&format!(
                "spoken_agents={}\nspoken_active_sessions={}\n",
                dashboard.agents.len(),
                dashboard.active_session_count()
            ));
            for row in &dashboard.agents {
                append_voice_agent_row(&mut output, row);
            }
        }
        VoiceReadScope::ProjectDispatchStatus { dispatch_plan_id } => {
            if let Some(status) = dashboard.adapter_dispatch_status(dispatch_plan_id) {
                append_voice_dispatch_status(&mut output, &status);
            } else {
                output.push_str(&format!(
                    "spoken_dispatch_plan_missing={dispatch_plan_id}\n"
                ));
            }
        }
        VoiceReadScope::ProjectLatestDispatchStatus { agent_name } => {
            if let Some(status) = dashboard.latest_adapter_dispatch_status(agent_name.as_deref()) {
                append_voice_dispatch_status(&mut output, &status);
            } else if let Some(agent_name) = agent_name {
                output.push_str(&format!(
                    "spoken_latest_dispatch_missing_for_agent={agent_name}\n"
                ));
            } else {
                output.push_str("spoken_latest_dispatch_missing=true\n");
            }
        }
        VoiceReadScope::ProjectAdapterSmokeReportStatus { smoke_report_id } => {
            if let Some(report) = dashboard.adapter_smoke_report_status(smoke_report_id) {
                append_voice_adapter_smoke_report_status(&mut output, report);
            } else {
                output.push_str(&format!("spoken_smoke_report_missing={smoke_report_id}\n"));
            }
        }
        VoiceReadScope::ProjectLatestAdapterSmokeReport { adapter_kind } => {
            if let Some(report) = dashboard.latest_adapter_smoke_report(adapter_kind.as_deref()) {
                append_voice_adapter_smoke_report_status(&mut output, report);
            } else if let Some(adapter_kind) = adapter_kind {
                output.push_str(&format!(
                    "spoken_latest_smoke_report_missing_for_adapter={adapter_kind}\n"
                ));
            } else {
                output.push_str("spoken_latest_smoke_report_missing=true\n");
            }
        }
        VoiceReadScope::ProjectLatestConnectivityExposure {
            owner_kind,
            owner_id,
            channel_kind,
        } => {
            if let Some(exposure) = dashboard.latest_connectivity_exposure(
                owner_kind.as_deref(),
                owner_id.as_deref(),
                channel_kind.as_deref(),
            ) {
                append_voice_connectivity_exposure_status(&mut output, exposure);
            } else {
                output.push_str("spoken_latest_connectivity_exposure_missing=true\n");
            }
        }
        VoiceReadScope::ProjectRuntimeTargetStatus { runtime_target_id } => {
            if let Some(target) = dashboard.runtime_target_status(runtime_target_id) {
                append_voice_runtime_target_status(&mut output, target);
            } else {
                output.push_str(&format!(
                    "spoken_runtime_target_missing={runtime_target_id}\n"
                ));
            }
        }
        VoiceReadScope::ProjectRuntimeTargetControlReadiness { runtime_target_id } => {
            if let Some(readiness) = dashboard.runtime_target_control_readiness(runtime_target_id) {
                append_voice_runtime_target_control_readiness(&mut output, &readiness);
            } else {
                output.push_str(&format!(
                    "spoken_runtime_target_missing={runtime_target_id}\n"
                ));
            }
        }
        VoiceReadScope::ProjectLatestRuntimeTargetStatus {
            runner_kind,
            status,
        } => {
            if let Some(target) =
                dashboard.latest_runtime_target(runner_kind.as_deref(), status.as_deref())
            {
                append_voice_runtime_target_status(&mut output, target);
            } else {
                output.push_str(&format!(
                    "spoken_latest_runtime_target_missing=true\nspoken_latest_runtime_target_filter_runner={} spoken_latest_runtime_target_filter_status={}\n",
                    runner_kind.as_deref().unwrap_or("any"),
                    status.as_deref().unwrap_or("any")
                ));
            }
        }
        VoiceReadScope::ProjectDogfoodReadiness => {
            let readiness = dashboard.dogfood_readiness();
            output.push_str(&format!(
                "spoken_dogfood_ready={}\nspoken_dogfood_status={}\nspoken_real_agent_connector_ready={}\nspoken_runtime_target_ready={}\nspoken_project_memory_ready={}\nspoken_workpad_bridge_ready={}\nspoken_dispatch_chain_ready={}\nspoken_blockers={}\nspoken_next_actions={}\nspoken_compatibility_blockers={}\nspoken_compatibility_next_actions={}\n",
                readiness.ready,
                readiness.status,
                readiness.real_agent_connector_ready,
                readiness.runtime_target_ready,
                readiness.project_memory_ready,
                readiness.workpad_bridge_ready,
                readiness.dispatch_chain_ready,
                comma_or_none(&readiness.blockers),
                comma_or_none(&readiness.next_actions),
                comma_or_none(&readiness.compatibility_blockers),
                comma_or_none(&readiness.compatibility_next_actions)
            ));
            output.push_str(&format!(
                "spoken_connector_evidence_refs={}\nspoken_runtime_target_refs={}\nspoken_source_task_refs={}\nspoken_workpad_task_refs={}\nspoken_dispatch_chain_refs={}\nspoken_project_evidence_refs={}\n",
                comma_or_none(&readiness.connector_evidence_refs),
                comma_or_none(&readiness.runtime_target_refs),
                comma_or_none(&readiness.source_task_refs),
                comma_or_none(&readiness.workpad_task_refs),
                comma_or_none(&readiness.dispatch_chain_refs),
                comma_or_none(&readiness.project_evidence_refs)
            ));
        }
        VoiceReadScope::ProjectNextWork => {
            let source_tasks = dashboard.source_tasks();
            output.push_str(&format!(
                "spoken_source_tasks={}\nspoken_next_source_task_candidates={}\nspoken_workpad_tasks={}\nspoken_next_work_candidates={}\n",
                source_tasks.len(),
                dashboard.next_source_task_candidate_count(),
                dashboard.workpad_tasks.len(),
                dashboard.next_workpad_candidate_count()
            ));
            if let Some(next) = dashboard.next_source_task() {
                output.push_str(&format!(
                    "spoken_next_source_task={} default_task_id={} source_path={} source_anchor={} source={}#{} title={} observed_source_status={} capo_binding_status={} compatibility_workpad_task_id={}\n",
                    next.source_task_id,
                    default_source_task_task_id(&next.source_task_id),
                    next.source_path,
                    next.source_anchor,
                    next.source_path,
                    next.source_anchor,
                    next.title,
                    next.observed_source_status,
                    next.capo_binding_status,
                    next.compatibility_workpad_task_id
                ));
            } else {
                output.push_str("spoken_next_source_task=none\n");
            }
            if let Some(next) = dashboard.next_workpad_task() {
                output.push_str(&format!(
                    "spoken_next_workpad_task={} default_task_id={} path={} source_anchor={} source={}#{} title={} observed_status={} capo_execution_status={}\n",
                    next.workpad_task_id,
                    default_workpad_task_id(&next.workpad_task_id),
                    next.path,
                    next.source_anchor,
                    next.path,
                    next.source_anchor,
                    next.title,
                    next.observed_status,
                    next.capo_execution_status
                ));
            } else {
                output.push_str("spoken_next_workpad_task=none\n");
            }
        }
        VoiceReadScope::ProjectRecentWork => {
            output.push_str(&format!(
                "spoken_agents={}\nspoken_active_sessions={}\nspoken_project_evidence={}\n",
                dashboard.agents.len(),
                dashboard.active_session_count(),
                dashboard.project_evidence.len()
            ));
            for row in &dashboard.agents {
                append_voice_agent_row(&mut output, row);
            }
        }
        VoiceReadScope::ProjectToolActivity => {
            append_voice_tool_activity_summary(&mut output, &dashboard.tool_activity_summary(None));
            for row in &dashboard.agents {
                append_voice_agent_tool_activity(&mut output, row);
            }
        }
        VoiceReadScope::ProjectReviewNeeds => {
            let open_review_findings = dashboard
                .review_findings
                .iter()
                .filter(|finding| finding.status == "open")
                .count();
            let review_blockers = dashboard
                .review_findings
                .iter()
                .filter(|finding| finding.status == "open" && finding.finding_kind == "blocker")
                .count();
            let reports_with_findings = dashboard
                .task_outcome_reports
                .iter()
                .filter(|report| report.review_outcome == "reviewed_with_findings")
                .count();
            let latest_review_outcome = dashboard
                .task_outcome_reports
                .iter()
                .max_by_key(|report| report.updated_sequence)
                .map(|report| report.review_outcome.as_str())
                .unwrap_or("none");
            output.push_str(&format!(
                "spoken_review_findings={}\nspoken_open_review_findings={}\nspoken_review_blockers={}\nspoken_task_outcome_reports={}\nspoken_reports_with_findings={}\nspoken_latest_review_outcome={}\n",
                dashboard.review_findings.len(),
                open_review_findings,
                review_blockers,
                dashboard.task_outcome_reports.len(),
                reports_with_findings,
                latest_review_outcome
            ));
            for finding in &dashboard.review_findings {
                output.push_str(&format!(
                    "spoken_review_finding={} kind={} severity={} status={} summary={}\n",
                    finding.review_finding_id,
                    finding.finding_kind,
                    finding.severity,
                    finding.status,
                    finding.summary
                ));
            }
            for report in &dashboard.task_outcome_reports {
                output.push_str(&format!(
                    "spoken_task_outcome_report={} outcome_status={} review_outcome={} artifact={}\n",
                    report.task_outcome_report_id,
                    report.outcome_status,
                    report.review_outcome,
                    report.report_artifact_id.as_deref().unwrap_or("none")
                ));
            }
        }
        VoiceReadScope::Agent { agent_name } | VoiceReadScope::SessionForAgent { agent_name } => {
            if let Some(row) = dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
            {
                append_voice_agent_row(&mut output, row);
            } else {
                output.push_str(&format!("spoken_agent_missing={agent_name}\n"));
            }
        }
        VoiceReadScope::AgentToolActivity { agent_name } => {
            if let Some(row) = dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
            {
                append_voice_tool_activity_summary(
                    &mut output,
                    &dashboard.tool_activity_summary(Some(agent_name)),
                );
                append_voice_agent_tool_activity(&mut output, row);
            } else {
                output.push_str(&format!("spoken_agent_missing={agent_name}\n"));
            }
        }
        VoiceReadScope::None => {}
    }
    output
}

fn append_voice_dispatch_status(output: &mut String, status: &AdapterDispatchStatus) {
    output.push_str(&format!(
        "spoken_dispatch_plan={} spoken_adapter={} spoken_agent={} spoken_plan_status={} spoken_provider_kind={} spoken_credential_scope={} spoken_provider_cli_executed={} spoken_dogfood_gate={} spoken_latest_gate_status={} spoken_latest_gate_provider_cli_execution_allowed={} spoken_latest_gate_reasons={} spoken_latest_dispatch_replay={} spoken_latest_replay_appended_events={} spoken_latest_execution_status={} spoken_latest_execution_provider_cli_executed={} spoken_latest_execution_credential_scan_status={} spoken_next_action={}\n",
        status.dispatch_plan_id,
        status.adapter_kind,
        status.agent_name,
        status.plan_status,
        status.provider_kind,
        status.credential_scope,
        status.provider_cli_executed,
        status.dogfood_gate_status,
        status.latest_gate_status,
        status.latest_gate_provider_cli_execution_allowed,
        status.latest_gate_reasons,
        status.latest_dispatch_replay_id,
        status.latest_replay_appended_events,
        status.latest_execution_status,
        status.latest_execution_provider_cli_executed,
        status.latest_execution_credential_scan_status,
        status.next_action
    ));
}

fn append_voice_adapter_smoke_report_status(
    output: &mut String,
    report: &AdapterSmokeReportProjection,
) {
    output.push_str(&format!(
        "spoken_smoke_report={} spoken_adapter={} spoken_smoke_status={} spoken_credential_scan_status={} spoken_marker_found={} spoken_dogfood_readiness_effect={} spoken_artifact_root={} spoken_reason={} spoken_provider_cli_executed=false spoken_credential_material_rendered=false spoken_state_mutated=false\n",
        report.smoke_report_id,
        report.adapter_kind,
        report.smoke_status,
        report.credential_scan_status,
        report.marker_found,
        report.dogfood_readiness_effect,
        report.artifact_root.as_deref().unwrap_or("none"),
        report.reason
    ));
}

fn append_voice_connectivity_exposure_status(
    output: &mut String,
    exposure: &ConnectivityExposureProjection,
) {
    output.push_str(&format!(
        "spoken_connectivity_exposure={} spoken_endpoint={} spoken_owner={}:{} spoken_channel={} spoken_exposure_scope={} spoken_permission_scope={} spoken_exposure_status={} spoken_health={} spoken_reachable={} spoken_grant={} spoken_revoked_at={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.health_status,
        exposure.reachable,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.revoked_at.as_deref().unwrap_or("none")
    ));
}

fn append_voice_runtime_target_status(output: &mut String, target: &RuntimeTargetProjection) {
    output.push_str(&format!(
        "spoken_runtime_target={} spoken_runtime_target_name={} spoken_runner={} spoken_workspace={} spoken_artifacts={} spoken_default_cwd={} spoken_capability_profile={} spoken_endpoint={} spoken_runtime_status={} spoken_updated_sequence={}\n",
        target.runtime_target_id,
        target.name,
        target.runner_kind,
        target.workspace_root,
        target.artifact_root,
        target.default_cwd,
        target.capability_profile_id,
        target.connectivity_endpoint_id.as_deref().unwrap_or("none"),
        target.status,
        target.updated_sequence
    ));
}

fn append_voice_runtime_target_control_readiness(
    output: &mut String,
    readiness: &RuntimeTargetControlReadiness,
) {
    output.push_str(&format!(
        "spoken_runtime_target={} spoken_runner={} spoken_target_status={} spoken_target_ready={} spoken_control_exposure_ready={} spoken_control_exposure={} spoken_control_exposure_status={} spoken_control_exposure_scope={} spoken_control_exposure_permission_scope={} spoken_control_exposure_reachable={} spoken_runtime_target_ready_for_control={} spoken_blockers={} spoken_next_action={}\n",
        readiness.runtime_target_id,
        readiness.runner_kind,
        readiness.target_status,
        readiness.target_ready,
        readiness.control_exposure_ready,
        readiness.control_exposure_id,
        readiness.control_exposure_status,
        readiness.control_exposure_scope,
        readiness.control_exposure_permission_scope,
        readiness.control_exposure_reachable,
        readiness.ready,
        readiness.blockers,
        readiness.next_action
    ));
}

fn append_voice_agent_row(output: &mut String, row: &capo_query::AgentDashboardRow) {
    output.push_str(&format!(
        "spoken_agent={} agent_status={}\n",
        row.agent.name, row.agent.status
    ));
    if let Some(session_row) = &row.session {
        output.push_str(&format!(
            "spoken_session={} session_status={} run_status={} current_goal={} latest_summary={} blocker={} confidence={} evidence_refs={} tool_calls={} tool_observations={} recent_events={}\n",
            session_row.session.session_id,
            session_row.session.status,
            session_row
                .run
                .as_ref()
                .map(|run| run.status.clone())
                .unwrap_or_else(|| "none".to_string()),
            session_row.session.current_goal,
            session_row
                .session
                .latest_summary
                .as_deref()
                .unwrap_or("none"),
            session_row.session.latest_blocker.as_deref().unwrap_or("none"),
            session_row
                .session
                .latest_confidence
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            session_row
                .evidence
                .iter()
                .map(|item| item.evidence_id.to_string())
                .collect::<Vec<_>>()
                .join(","),
            session_row.tool_calls.len(),
            session_row.tool_observations.len(),
            session_row.recent_events.len()
        ));
        for tool_call in &session_row.tool_calls {
            output.push_str(&format!(
                "spoken_tool_call={} tool={} origin={} status={} output_artifact={}\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for observation in &session_row.tool_observations {
            output.push_str(&format!(
                "spoken_tool_observation={} tool={} source={} observed_status={} instrumentation={} confidence={} external_ref={} artifact={}\n",
                observation.tool_observation_id,
                observation.tool_name,
                observation.source,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.external_tool_ref.as_deref().unwrap_or("none"),
                observation.artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
}

fn append_voice_tool_activity_summary(
    output: &mut String,
    summary: &capo_query::ToolActivitySummary,
) {
    output.push_str(&format!(
        "spoken_tool_activity_agents={}\nspoken_tool_activity_active_sessions={}\nspoken_tool_calls={}\nspoken_tool_observations={}\n",
        summary.agent_count,
        summary.active_session_count,
        summary.tool_call_count,
        summary.tool_observation_count
    ));
}

fn append_voice_agent_tool_activity(output: &mut String, row: &capo_query::AgentDashboardRow) {
    output.push_str(&format!(
        "spoken_tool_activity_agent={} agent_status={}\n",
        row.agent.name, row.agent.status
    ));
    if let Some(session_row) = &row.session {
        output.push_str(&format!(
            "spoken_tool_activity_session={} tool_calls={} tool_observations={}\n",
            session_row.session.session_id,
            session_row.tool_calls.len(),
            session_row.tool_observations.len()
        ));
        for tool_call in &session_row.tool_calls {
            output.push_str(&format!(
                "spoken_tool_call={} tool={} origin={} status={} output_artifact={}\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for observation in &session_row.tool_observations {
            output.push_str(&format!(
                "spoken_tool_observation={} tool={} source={} observed_status={} instrumentation={} confidence={} external_ref={} artifact={}\n",
                observation.tool_observation_id,
                observation.tool_name,
                observation.source,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.external_tool_ref.as_deref().unwrap_or("none"),
                observation.artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
}

pub(crate) fn voice_intent_label(intent: VoiceIntentKind) -> &'static str {
    match intent {
        VoiceIntentKind::AgentStatus => "agent_status",
        VoiceIntentKind::AdapterSmokeStatus => "adapter_smoke_status",
        VoiceIntentKind::ConnectivityStatus => "connectivity_status",
        VoiceIntentKind::DashboardSummary => "dashboard_summary",
        VoiceIntentKind::DispatchStatus => "dispatch_status",
        VoiceIntentKind::DogfoodReadiness => "dogfood_readiness",
        VoiceIntentKind::NextWork => "next_work",
        VoiceIntentKind::RecentWork => "recent_work",
        VoiceIntentKind::ReviewNeeds => "review_needs",
        VoiceIntentKind::RedirectSession => "redirect_session",
        VoiceIntentKind::RuntimeTargetReadiness => "runtime_target_readiness",
        VoiceIntentKind::RuntimeTargetStatus => "runtime_target_status",
        VoiceIntentKind::StartNextWork => "start_next_work",
        VoiceIntentKind::InterruptSession => "interrupt_session",
        VoiceIntentKind::StopSession => "stop_session",
        VoiceIntentKind::ToolActivity => "tool_activity",
        VoiceIntentKind::Unknown => "unknown",
    }
}

fn voice_scope_label(scope: &VoiceReadScope) -> &'static str {
    match scope {
        VoiceReadScope::ProjectDashboard => "project_dashboard",
        VoiceReadScope::ProjectLatestConnectivityExposure { .. } => {
            "project_latest_connectivity_exposure"
        }
        VoiceReadScope::ProjectRuntimeTargetStatus { .. } => "project_runtime_target_status",
        VoiceReadScope::ProjectRuntimeTargetControlReadiness { .. } => {
            "project_runtime_target_control_readiness"
        }
        VoiceReadScope::ProjectLatestRuntimeTargetStatus { .. } => {
            "project_latest_runtime_target_status"
        }
        VoiceReadScope::ProjectAdapterSmokeReportStatus { .. } => {
            "project_adapter_smoke_report_status"
        }
        VoiceReadScope::ProjectLatestAdapterSmokeReport { .. } => {
            "project_latest_adapter_smoke_report"
        }
        VoiceReadScope::ProjectDispatchStatus { .. } => "project_dispatch_status",
        VoiceReadScope::ProjectLatestDispatchStatus { .. } => "project_latest_dispatch_status",
        VoiceReadScope::ProjectDogfoodReadiness => "project_dogfood_readiness",
        VoiceReadScope::ProjectNextWork => "project_next_work",
        VoiceReadScope::ProjectRecentWork => "project_recent_work",
        VoiceReadScope::ProjectReviewNeeds => "project_review_needs",
        VoiceReadScope::ProjectToolActivity => "project_tool_activity",
        VoiceReadScope::AgentToolActivity { .. } => "agent_tool_activity",
        VoiceReadScope::Agent { .. } => "agent",
        VoiceReadScope::SessionForAgent { .. } => "session_for_agent",
        VoiceReadScope::None => "none",
    }
}

fn memory_ingestion_label(policy: MemoryIngestionPolicy) -> &'static str {
    match policy {
        MemoryIngestionPolicy::None => "none",
        MemoryIngestionPolicy::ReviewedRedactedSummaryOnly => "reviewed_redacted_summary_only",
    }
}
