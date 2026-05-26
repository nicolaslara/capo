use capo_core::{CommandEnvelope, CommandIntent, CommandTarget, ProjectId, SessionId};
use capo_query::{ProjectDashboard, ProjectDashboardQuery, project_dashboard};

use crate::adapter_dogfood::render_adapter_dogfood_gate;
use crate::cli_surface::ParsedArgs;
use crate::runtime_target::{render_runtime_target_control_readiness, render_runtime_target_row};
use crate::workpad::default_workpad_task_id;
use crate::{comma_or_none, debug_error, envelope, project_id, state};

pub(crate) fn dashboard(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let query = dashboard_query(args)?;
    let command = envelope(
        "dashboard",
        CommandTarget::Project(query.project_id.clone()),
        CommandIntent::QueryStatus,
        None,
    );
    let command = CommandEnvelope {
        project_id: query.project_id.clone(),
        ..command
    };
    let state = state(parsed)?;
    let dashboard = project_dashboard(&state, query).map_err(debug_error)?;
    Ok(render_dashboard(&command, &dashboard))
}

fn dashboard_query(args: &[String]) -> Result<ProjectDashboardQuery, String> {
    let mut project_id = project_id();
    let mut session_id = None;
    let mut status = None;
    let mut workpad_path = None;
    let mut workpad_status = None;
    let mut index = 0;
    while index < args.len() {
        let key = args[index].as_str();
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{key} requires a value"))?;
        match key {
            "--project" => project_id = ProjectId::new(value),
            "--session" => session_id = Some(SessionId::new(value)),
            "--status" => status = Some(value.clone()),
            "--workpad-path" => workpad_path = Some(value.clone()),
            "--workpad-status" => workpad_status = Some(value.clone()),
            other => return Err(format!("unknown dashboard filter: {other}")),
        }
        index += 2;
    }
    let mut query = ProjectDashboardQuery::new(project_id);
    if let Some(session_id) = session_id {
        query = query.with_session_id(session_id);
    }
    if let Some(status) = status {
        query = query.with_status(status);
    }
    if let Some(workpad_path) = workpad_path {
        query = query.with_workpad_path(workpad_path);
    }
    if let Some(workpad_status) = workpad_status {
        query = query.with_workpad_status(workpad_status);
    }
    Ok(query)
}

fn render_dashboard(command: &CommandEnvelope, dashboard: &ProjectDashboard) -> String {
    let dogfood_readiness = dashboard.dogfood_readiness();
    let tool_activity = dashboard.tool_activity_summary(None);
    let mut output = format!(
        "command_id={}\nview=dashboard\nagents={}\ntool_activity_agents={}\ntool_activity_active_sessions={}\ntool_calls={}\ntool_observations={}\n",
        command.command_id,
        dashboard.agents.len(),
        tool_activity.agent_count,
        tool_activity.active_session_count,
        tool_activity.tool_call_count,
        tool_activity.tool_observation_count
    );

    for row in &dashboard.agents {
        let agent = &row.agent;
        output.push_str(&format!(
            "agent={} agent_status={} current_session={}\n",
            agent.name,
            agent.status,
            agent
                .current_session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string())
        ));

        let Some(session_row) = &row.session else {
            continue;
        };
        let session = &session_row.session;

        output.push_str(&format!(
            "session={} session_status={} run={} run_status={} goal={} blocker={} confidence={} evidence_refs={} tool_calls={} memory_packet_refs={} recent_events={}\n",
            session.session_id,
            session.status,
            session_row
                .run
                .as_ref()
                .map(|item| item.run_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            session_row
                .run
                .as_ref()
                .map(|item| item.status.clone())
                .unwrap_or_else(|| "none".to_string()),
            session.current_goal,
            session.latest_blocker.as_deref().unwrap_or("none"),
            session
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
            session_row.memory_packets.len(),
            session_row.recent_events.len()
        ));
        output.push_str(&format!(
            "session_review_findings={}\n",
            session_row.review_findings.len()
        ));
        for finding in &session_row.review_findings {
            output.push_str(&format!(
                "review_finding={} session={} kind={} severity={} status={} reviewer={} evidence_artifact={} follow_up={} summary={}\n",
                finding.review_finding_id,
                finding.session_id,
                finding.finding_kind,
                finding.severity,
                finding.status,
                finding.reviewer,
                finding.evidence_artifact_id.as_deref().unwrap_or("none"),
                finding.follow_up.as_deref().unwrap_or("none"),
                finding.summary
            ));
        }
        output.push_str(&format!(
            "session_task_outcome_reports={}\n",
            session_row.task_outcome_reports.len()
        ));
        for report in &session_row.task_outcome_reports {
            output.push_str(&format!(
                "task_outcome_report={} session={} task={} run={} outcome_status={} review_outcome={} actions={} tool_calls={} evidence={} memory_packets={} confidence={} blocker={} artifact={}\n",
                report.task_outcome_report_id,
                report.session_id,
                report.task_id,
                report.run_id,
                report.outcome_status,
                report.review_outcome,
                report.action_count,
                report.tool_call_count,
                report.evidence_count,
                report.memory_packet_count,
                report
                    .confidence
                    .map(|confidence| confidence.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                report.blocker.as_deref().unwrap_or("none"),
                report.report_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for tool_call in &session_row.tool_calls {
            output.push_str(&format!(
                "tool_call={} tool={} tool_origin={} tool_status={} input_artifact={} output_artifact={}\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id.as_deref().unwrap_or("none"),
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        output.push_str(&format!(
            "tool_observations={}\n",
            session_row.tool_observations.len()
        ));
        for observation in &session_row.tool_observations {
            output.push_str(&format!(
                "tool_observation={} tool={} source={} observed_status={} instrumentation={} confidence={} external_ref={} artifact={} raw_event_hash={}\n",
                observation.tool_observation_id,
                observation.tool_name,
                observation.source,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.external_tool_ref.as_deref().unwrap_or("none"),
                observation.artifact_id.as_deref().unwrap_or("none"),
                observation.raw_event_hash
            ));
        }
        for packet in &session_row.memory_packets {
            output.push_str(&format!(
                "memory_packet={} purpose={} artifact={}\n",
                packet.memory_packet_id,
                packet.purpose,
                packet.packet_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for event in &session_row.recent_events {
            output.push_str(&format!("event={} kind={}\n", event.sequence, event.kind));
        }
    }

    output.push_str(&format!(
        "project_evidence={}\n",
        dashboard.project_evidence.len()
    ));
    for evidence in &dashboard.project_evidence {
        output.push_str(&format!(
            "project_evidence_ref={} kind={} artifact={} confidence={}\n",
            evidence.evidence_id,
            evidence.kind,
            evidence.artifact_id.as_deref().unwrap_or("none"),
            evidence.confidence
        ));
    }

    output.push_str(&format!(
        "review_findings={}\n",
        dashboard.review_findings.len()
    ));
    for finding in &dashboard.review_findings {
        output.push_str(&format!(
            "project_review_finding={} session={} kind={} severity={} status={} reviewer={} evidence_artifact={} follow_up={} summary={}\n",
            finding.review_finding_id,
            finding.session_id,
            finding.finding_kind,
            finding.severity,
            finding.status,
            finding.reviewer,
            finding.evidence_artifact_id.as_deref().unwrap_or("none"),
            finding.follow_up.as_deref().unwrap_or("none"),
            finding.summary
        ));
    }

    output.push_str(&format!(
        "task_outcome_reports={}\n",
        dashboard.task_outcome_reports.len()
    ));
    for report in &dashboard.task_outcome_reports {
        output.push_str(&format!(
            "project_task_outcome_report={} session={} task={} run={} outcome_status={} review_outcome={} actions={} tool_calls={} evidence={} memory_packets={} confidence={} blocker={} artifact={}\n",
            report.task_outcome_report_id,
            report.session_id,
            report.task_id,
            report.run_id,
            report.outcome_status,
            report.review_outcome,
            report.action_count,
            report.tool_call_count,
            report.evidence_count,
            report.memory_packet_count,
            report
                .confidence
                .map(|confidence| confidence.to_string())
                .unwrap_or_else(|| "none".to_string()),
            report.blocker.as_deref().unwrap_or("none"),
            report.report_artifact_id.as_deref().unwrap_or("none")
        ));
    }

    output.push_str(&format!(
        "runtime_targets={}\n",
        dashboard.runtime_targets.len()
    ));
    for target in &dashboard.runtime_targets {
        output.push_str(&render_runtime_target_row("runtime_target", target));
        if let Some(readiness) =
            dashboard.runtime_target_control_readiness(&target.runtime_target_id)
        {
            output.push_str(&render_runtime_target_control_readiness(&readiness));
        }
    }

    output.push_str(&format!(
        "connectivity_exposures={}\n",
        dashboard.connectivity_exposures.len()
    ));
    for exposure in &dashboard.connectivity_exposures {
        output.push_str(&format!(
            "connectivity_exposure={} endpoint={} owner={}:{} channel={} exposure={} exposure_status={} health={} reachable={} permission_scope={} grant={} revoked_at={}\n",
            exposure.exposure_id,
            exposure.connectivity_endpoint_id,
            exposure.owner_kind,
            exposure.owner_id,
            exposure.channel_kind,
            exposure.exposure,
            exposure.status,
            exposure.health_status,
            exposure.reachable,
            exposure.permission_scope,
            exposure.capability_grant_id.as_deref().unwrap_or("none"),
            exposure.revoked_at.as_deref().unwrap_or("none")
        ));
    }
    output.push_str(&format!(
        "adapter_readiness={}\n",
        dashboard.adapter_readiness.len()
    ));
    for readiness in &dashboard.adapter_readiness {
        output.push_str(&format!(
            "adapter_readiness_row={} program={} opt_in_env={} opted_in={} smoke_status={} credential_policy={} expected_marker={} env_allowlist={} redaction_rules={} output_limit_bytes={} dogfood_blocker={}\n",
            readiness.adapter_kind,
            readiness.program,
            readiness.opt_in_env,
            readiness.opted_in,
            readiness.smoke_status,
            readiness.credential_policy,
            readiness.expected_marker,
            readiness.env_allowlist_count,
            readiness.redaction_rule_count,
            readiness.output_limit_bytes,
            readiness.dogfood_blocker.as_deref().unwrap_or("none")
        ));
    }
    output.push_str(&format!(
        "adapter_smoke_reports={}\n",
        dashboard.adapter_smoke_reports.len()
    ));
    append_dashboard_latest_adapter_smoke_report(&mut output, dashboard, "any", None);
    append_dashboard_latest_adapter_smoke_report(
        &mut output,
        dashboard,
        "codex",
        Some("codex_exec"),
    );
    append_dashboard_latest_adapter_smoke_report(
        &mut output,
        dashboard,
        "claude",
        Some("claude_code"),
    );
    for report in &dashboard.adapter_smoke_reports {
        output.push_str(&format!(
            "adapter_smoke_report={} adapter={} smoke_status={} credential_scan_status={} marker_found={} dogfood_readiness_effect={} artifact_root={} reason={}\n",
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
    output.push_str(&format!(
        "adapter_dispatch_plans={}\n",
        dashboard.adapter_dispatch_plans.len()
    ));
    for plan in &dashboard.adapter_dispatch_plans {
        output.push_str(&format!(
            "adapter_dispatch_plan={} adapter={} provider_kind={} credential_scope={} agent={} session={} run={} runtime_program={} runtime_arg_count={} runtime_prompt_policy={} runtime_cwd={} artifact_root={} provider_cli_executed={} status={}\n",
            plan.dispatch_plan_id,
            plan.adapter_kind,
            plan.provider_kind,
            plan.credential_scope,
            plan.agent_name,
            plan.session_id,
            plan.run_id,
            plan.runtime_program,
            plan.runtime_arg_count,
            plan.runtime_prompt_policy,
            plan.runtime_cwd,
            plan.artifact_root,
            plan.provider_cli_executed,
            plan.status
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_gates={}\n",
        dashboard.adapter_dispatch_gates.len()
    ));
    for gate in &dashboard.adapter_dispatch_gates {
        output.push_str(&format!(
            "adapter_dispatch_gate={} dispatch_plan={} adapter={} provider_cli_execution_allowed={} gate_status={} required_dogfood_gate={} provider_cli_executed={} runtime_prompt_policy={} reasons={}\n",
            gate.dispatch_gate_id,
            gate.dispatch_plan_id,
            gate.adapter_kind,
            gate.provider_cli_execution_allowed,
            gate.status,
            gate.required_dogfood_gate,
            gate.provider_cli_executed,
            gate.runtime_prompt_policy,
            gate.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_replays={}\n",
        dashboard.adapter_dispatch_replays.len()
    ));
    for replay in &dashboard.adapter_dispatch_replays {
        output.push_str(&format!(
            "adapter_dispatch_replay={} dispatch_plan={} dispatch_gate={} adapter={} session={} run={} fixture_hash={} input_events={} appended_events={} tool_events={} summary_events={} completed_turns={} provider_cli_executed={} raw_content_policy={}\n",
            replay.dispatch_replay_id,
            replay.dispatch_plan_id,
            replay.dispatch_gate_id,
            replay.adapter_kind,
            replay.session_id,
            replay.run_id,
            replay.fixture_hash,
            replay.input_event_count,
            replay.appended_event_count,
            replay.tool_event_count,
            replay.summary_event_count,
            replay.completed_turn_count,
            replay.provider_cli_executed,
            replay.raw_content_policy
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_execution_requests={}\n",
        dashboard.adapter_dispatch_execution_requests.len()
    ));
    for request in &dashboard.adapter_dispatch_execution_requests {
        output.push_str(&format!(
            "adapter_dispatch_execution_request={} dispatch_plan={} dispatch_gate={} adapter={} execution_status={} provider_cli_execution_allowed={} provider_cli_executed={} opt_in_env={} runtime_prompt_policy={} reasons={}\n",
            request.execution_request_id,
            request.dispatch_plan_id,
            request.dispatch_gate_id,
            request.adapter_kind,
            request.status,
            request.provider_cli_execution_allowed,
            request.provider_cli_executed,
            request.opt_in_env,
            request.runtime_prompt_policy,
            request.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_executions={}\n",
        dashboard.adapter_dispatch_executions.len()
    ));
    for execution in &dashboard.adapter_dispatch_executions {
        output.push_str(&format!(
            "adapter_dispatch_execution={} dispatch_plan={} execution_request={} adapter={} session={} run={} execution_status={} provider_cli_execution_allowed={} provider_cli_executed={} exit_code={} runtime_process_ref={} stdout_artifact={} stderr_artifact={} artifact_root={} credential_scan_status={} raw_prompt_policy={} raw_output_policy={} reasons={}\n",
            execution.dispatch_execution_id,
            execution.dispatch_plan_id,
            execution.execution_request_id,
            execution.adapter_kind,
            execution.session_id,
            execution.run_id,
            execution.status,
            execution.provider_cli_execution_allowed,
            execution.provider_cli_executed,
            execution
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string()),
            execution.runtime_process_ref.as_deref().unwrap_or("none"),
            execution.stdout_artifact_id.as_deref().unwrap_or("none"),
            execution.stderr_artifact_id.as_deref().unwrap_or("none"),
            execution.artifact_root,
            execution.credential_scan_status,
            execution.raw_prompt_policy,
            execution.raw_output_policy,
            execution.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_prompt_sources={}\n",
        dashboard.adapter_dispatch_prompt_sources.len()
    ));
    for source in &dashboard.adapter_dispatch_prompt_sources {
        output.push_str(&format!(
            "adapter_dispatch_prompt_source={} dispatch_plan={} source_kind={} source_ref={} source_hash={} materialization_status={} raw_prompt_policy={}\n",
            source.prompt_source_id,
            source.dispatch_plan_id,
            source.source_kind,
            source.source_ref.as_deref().unwrap_or("none"),
            source.source_hash.as_deref().unwrap_or("none"),
            source.materialization_status,
            source.raw_prompt_policy
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_prompt_materializations={}\n",
        dashboard.adapter_dispatch_prompt_materializations.len()
    ));
    for materialization in &dashboard.adapter_dispatch_prompt_materializations {
        output.push_str(&format!(
            "adapter_dispatch_prompt_materialization={} dispatch_plan={} prompt_source={} source_kind={} status={} raw_prompt_policy={} reasons={}\n",
            materialization.materialization_id,
            materialization.dispatch_plan_id,
            materialization.prompt_source_id,
            materialization.source_kind,
            materialization.status,
            materialization.raw_prompt_policy,
            materialization.reason_codes
        ));
    }
    output.push_str(&render_adapter_dogfood_gate(
        &dashboard.adapter_dogfood_gate,
    ));
    output.push_str(&format!(
        "project_dogfood_readiness={} status={} real_agent_connector_ready={} runtime_target_ready={} workpad_bridge_ready={} dispatch_chain_ready={} connector_evidence_refs={} runtime_target_refs={} workpad_task_refs={} dispatch_chain_refs={} project_evidence_refs={} blockers={} next_actions={}\n",
        dogfood_readiness.ready,
        dogfood_readiness.status,
        dogfood_readiness.real_agent_connector_ready,
        dogfood_readiness.runtime_target_ready,
        dogfood_readiness.workpad_bridge_ready,
        dogfood_readiness.dispatch_chain_ready,
        comma_or_none(&dogfood_readiness.connector_evidence_refs),
        comma_or_none(&dogfood_readiness.runtime_target_refs),
        comma_or_none(&dogfood_readiness.workpad_task_refs),
        comma_or_none(&dogfood_readiness.dispatch_chain_refs),
        comma_or_none(&dogfood_readiness.project_evidence_refs),
        comma_or_none(&dogfood_readiness.blockers),
        comma_or_none(&dogfood_readiness.next_actions)
    ));
    output.push_str(&format!(
        "workpad_tasks={}\n",
        dashboard.workpad_tasks.len()
    ));
    for task in &dashboard.workpad_tasks {
        output.push_str(&format!(
            "workpad_task={} path={} source_anchor={} observed_status={} capo_execution_status={} default_task_id={}\n",
            task.workpad_task_id,
            task.path,
            task.source_anchor,
            task.observed_status,
            task.capo_execution_status,
            default_workpad_task_id(&task.workpad_task_id)
        ));
    }

    output.push_str(&format!(
        "active_sessions={}\n",
        dashboard.active_session_count()
    ));
    output
}

fn append_dashboard_latest_adapter_smoke_report(
    output: &mut String,
    dashboard: &ProjectDashboard,
    label: &str,
    adapter_kind: Option<&str>,
) {
    if let Some(report) = dashboard.latest_adapter_smoke_report(adapter_kind) {
        output.push_str(&format!(
            "latest_adapter_smoke_report_{label}={} adapter={} smoke_status={} credential_scan_status={} marker_found={} dogfood_readiness_effect={} updated_sequence={}\n",
            report.smoke_report_id,
            report.adapter_kind,
            report.smoke_status,
            report.credential_scan_status,
            report.marker_found,
            report.dogfood_readiness_effect,
            report.updated_sequence
        ));
    } else {
        output.push_str(&format!("latest_adapter_smoke_report_{label}=none\n"));
    }
}
