use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget};
use capo_query::{
    AdapterDispatchStatus, AdapterDogfoodGate, ProjectDashboardQuery, project_dashboard,
};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchGateProjection,
    AdapterDispatchPlanProjection, AdapterDispatchReplayProjection, ArtifactRecord, EventKind,
    EvidenceProjection, NewEvent, ProjectionRecord, RedactionState, ToolObservationProjection,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{debug_error, envelope, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn adapter_dispatch_gate(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!("unknown adapter dispatch-gate option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let gate = adapter_dispatch_gate_projection(plan, &dashboard.adapter_dogfood_gate);
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-gate-{}",
                stable_cli_hash(&gate.dispatch_gate_id)
            ),
            kind: EventKind::AdapterDispatchGateChecked,
            actor: "local-cli".to_string(),
            project_id: Some(gate.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(gate.dispatch_gate_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"provider_cli_execution_allowed\":{},\"provider_cli_executed\":false}}",
                escape_json(&gate.dispatch_plan_id),
                gate.provider_cli_execution_allowed
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-gate:{}:{}:{}",
                gate.project_id, gate.dispatch_plan_id, gate.reason_codes
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchGate(gate.clone())],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_gate(
        &gate,
        record,
        recorded_sequence,
    ))
}

pub(crate) fn adapter_dispatch_status(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let dispatch_plan_id = optional_arg(args, "--dispatch-plan");
    let agent_name = optional_arg(args, "--agent");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--latest" | "--agent")
    }) {
        return Err(format!("unknown adapter dispatch-status option: {unknown}"));
    }
    if latest && dispatch_plan_id.is_some() {
        return Err(
            "adapter dispatch-status accepts either --dispatch-plan or --latest".to_string(),
        );
    }
    if !latest && agent_name.is_some() {
        return Err("adapter dispatch-status --agent requires --latest".to_string());
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let status = if latest {
        dashboard
            .latest_adapter_dispatch_status(agent_name.as_deref())
            .ok_or_else(|| {
                agent_name.map_or_else(
                    || "no recorded adapter dispatch plans".to_string(),
                    |agent| format!("no recorded adapter dispatch plans for agent: {agent}"),
                )
            })?
    } else {
        let dispatch_plan_id = dispatch_plan_id.ok_or_else(|| {
            "adapter dispatch-status requires --dispatch-plan or --latest".to_string()
        })?;
        dashboard
            .adapter_dispatch_status(&dispatch_plan_id)
            .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?
    };
    Ok(render_adapter_dispatch_status(&status))
}

pub(crate) fn adapter_dispatch_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let dispatch_plan_id = optional_arg(args, "--dispatch-plan");
    let agent_name = optional_arg(args, "--agent");
    let out = PathBuf::from(required_arg(args, "--out")?);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--dispatch-plan" | "--latest" | "--agent" | "--out"
            )
    }) {
        return Err(format!(
            "unknown adapter dispatch-evidence option: {unknown}"
        ));
    }
    if latest && dispatch_plan_id.is_some() {
        return Err(
            "adapter dispatch-evidence accepts either --dispatch-plan or --latest".to_string(),
        );
    }
    if !latest && agent_name.is_some() {
        return Err("adapter dispatch-evidence --agent requires --latest".to_string());
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let dispatch_plan_id = if latest {
        dashboard
            .latest_adapter_dispatch_status(agent_name.as_deref())
            .ok_or_else(|| {
                agent_name.map_or_else(
                    || "no recorded adapter dispatch plans".to_string(),
                    |agent| format!("no recorded adapter dispatch plans for agent: {agent}"),
                )
            })?
            .dispatch_plan_id
    } else {
        dispatch_plan_id.ok_or_else(|| {
            "adapter dispatch-evidence requires --dispatch-plan or --latest".to_string()
        })?
    };
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let latest_gate = dashboard
        .adapter_dispatch_gates
        .iter()
        .rev()
        .find(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id);
    let latest_replay = dashboard
        .adapter_dispatch_replays
        .iter()
        .rev()
        .find(|replay| replay.dispatch_plan_id == plan.dispatch_plan_id);
    let latest_execution = dashboard
        .adapter_dispatch_executions
        .iter()
        .rev()
        .find(|execution| execution.dispatch_plan_id == plan.dispatch_plan_id);
    let tool_observations = state
        .tool_observations_for_session(&plan.session_id)
        .map_err(debug_error)?;
    let command = envelope(
        "adapter-dispatch-evidence",
        CommandTarget::Session(plan.session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(dispatch_plan_id.clone()),
    );
    let markdown = render_adapter_dispatch_evidence(
        plan,
        latest_gate,
        latest_replay,
        latest_execution,
        &dashboard.adapter_dogfood_gate,
        &tool_observations,
    );
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!(
        "artifact-adapter-dispatch-evidence-{}-{}",
        stable_cli_hash(&plan.dispatch_plan_id),
        content_hash
    );
    let path = out.join(format!("{artifact_id}.md"));
    write_dispatch_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(plan.project_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            kind: "adapter_dispatch_evidence".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_id = format!("evidence-{artifact_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!("event-{evidence_id}"),
                kind: EventKind::EvidenceRecorded,
                actor: "cli".to_string(),
                project_id: Some(plan.project_id.clone()),
                task_id: None,
                agent_id: Some(plan.agent_id.clone()),
                session_id: Some(plan.session_id.clone()),
                run_id: Some(plan.run_id.clone()),
                turn_id: None,
                item_id: Some(evidence_id.clone()),
                payload_json: format!(
                    "{{\"dispatch_plan_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\"}}",
                    escape_json(&plan.dispatch_plan_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash)
                ),
                idempotency_key: Some(format!(
                    "adapter-dispatch-evidence:{}:{}",
                    plan.dispatch_plan_id, content_hash
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: plan.project_id.clone(),
                task_id: None,
                session_id: Some(plan.session_id.clone()),
                run_id: Some(plan.run_id.clone()),
                kind: "adapter_dispatch_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: dispatch_evidence_confidence(latest_execution, latest_replay),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_dispatch_evidence_exported=true\ndispatch_plan={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        plan.dispatch_plan_id,
        path.display(),
        command.command_id
    ))
}

fn adapter_dispatch_gate_projection(
    plan: &AdapterDispatchPlanProjection,
    dogfood_gate: &AdapterDogfoodGate,
) -> AdapterDispatchGateProjection {
    let adapter_proven = dogfood_gate
        .proven_adapters
        .iter()
        .any(|adapter| adapter == &plan.adapter_kind);
    let execution_allowed = dogfood_gate.ready
        && adapter_proven
        && plan.status == "planned"
        && plan.runtime_prompt_policy == "not_rendered"
        && !plan.provider_cli_executed;
    let mut reasons = Vec::new();
    if !dogfood_gate.ready {
        reasons.extend(dogfood_gate.reasons.iter().cloned());
    }
    if dogfood_gate.ready && !adapter_proven {
        reasons.push(format!(
            "{}:real_subscription_smoke_not_recorded",
            plan.adapter_kind
        ));
    }
    if plan.status != "planned" {
        reasons.push(format!("dispatch_plan_status:{}", plan.status));
    }
    if plan.runtime_prompt_policy != "not_rendered" {
        reasons.push("runtime_prompt_policy_not_redacted".to_string());
    }
    if plan.provider_cli_executed {
        reasons.push("provider_cli_already_executed".to_string());
    }
    if reasons.is_empty() {
        reasons.push("required_real_smoke_evidence_recorded".to_string());
    }
    let status = if execution_allowed {
        "ready_for_execution"
    } else {
        "blocked"
    }
    .to_string();
    AdapterDispatchGateProjection {
        dispatch_gate_id: format!(
            "adapter-dispatch-gate-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&reasons.join(","))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        provider_cli_execution_allowed: execution_allowed,
        status,
        required_dogfood_gate: dogfood_gate.status.clone(),
        reason_codes: reasons.join(","),
        provider_cli_executed: plan.provider_cli_executed,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        updated_sequence: 0,
    }
}

fn render_adapter_dispatch_gate(
    gate: &AdapterDispatchGateProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_gate=true\ndispatch_gate={}\ndispatch_plan={}\nadapter={}\nprovider_cli_execution_allowed={}\nstatus={}\nrequired_dogfood_gate={}\nprovider_cli_executed={}\nruntime_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        gate.dispatch_gate_id,
        gate.dispatch_plan_id,
        gate.adapter_kind,
        gate.provider_cli_execution_allowed,
        gate.status,
        gate.required_dogfood_gate,
        gate.provider_cli_executed,
        gate.runtime_prompt_policy,
        gate.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_adapter_dispatch_status(status: &AdapterDispatchStatus) -> String {
    format!(
        "adapter_dispatch_status=true\ndispatch_plan={}\nadapter={}\nagent={}\nsession_id={}\nrun_id={}\nplan_status={}\nprovider_kind={}\ncredential_scope={}\nruntime_program={}\nruntime_arg_count={}\nruntime_prompt_policy={}\nprovider_cli_executed={}\ndogfood_gate={}\nlatest_dispatch_gate={}\nlatest_gate_status={}\nlatest_gate_provider_cli_execution_allowed={}\nlatest_gate_reasons={}\nlatest_dispatch_replay={}\nlatest_replay_appended_events={}\nlatest_replay_raw_content_policy={}\nlatest_dispatch_execution={}\nlatest_execution_status={}\nlatest_execution_provider_cli_execution_allowed={}\nlatest_execution_provider_cli_executed={}\nlatest_execution_credential_scan_status={}\nlatest_execution_stdout_artifact={}\nlatest_execution_stderr_artifact={}\nlatest_execution_reasons={}\nnext_action={}\n",
        status.dispatch_plan_id,
        status.adapter_kind,
        status.agent_name,
        status.session_id,
        status.run_id,
        status.plan_status,
        status.provider_kind,
        status.credential_scope,
        status.runtime_program,
        status.runtime_arg_count,
        status.runtime_prompt_policy,
        status.provider_cli_executed,
        status.dogfood_gate_status,
        status.latest_dispatch_gate_id,
        status.latest_gate_status,
        status.latest_gate_provider_cli_execution_allowed,
        status.latest_gate_reasons,
        status.latest_dispatch_replay_id,
        status.latest_replay_appended_events,
        status.latest_replay_raw_content_policy,
        status.latest_dispatch_execution_id,
        status.latest_execution_status,
        status.latest_execution_provider_cli_execution_allowed,
        status.latest_execution_provider_cli_executed,
        status.latest_execution_credential_scan_status,
        status.latest_execution_stdout_artifact_id,
        status.latest_execution_stderr_artifact_id,
        status.latest_execution_reasons,
        status.next_action
    )
}

fn dispatch_evidence_confidence(
    latest_execution: Option<&AdapterDispatchExecutionProjection>,
    latest_replay: Option<&AdapterDispatchReplayProjection>,
) -> i64 {
    if latest_execution
        .map(|execution| execution.provider_cli_executed)
        .unwrap_or(false)
    {
        85
    } else if latest_replay.is_some() {
        75
    } else if latest_execution.is_some() {
        65
    } else {
        55
    }
}

fn render_adapter_dispatch_evidence(
    plan: &AdapterDispatchPlanProjection,
    latest_gate: Option<&AdapterDispatchGateProjection>,
    latest_replay: Option<&AdapterDispatchReplayProjection>,
    latest_execution: Option<&AdapterDispatchExecutionProjection>,
    dogfood_gate: &AdapterDogfoodGate,
    tool_observations: &[ToolObservationProjection],
) -> String {
    let mut markdown = format!(
        "<!-- capo:adapter-dispatch-evidence -->\n# Capo Adapter Dispatch Evidence - {}\n\n## Objective\n\nReview a prompt-redacted dispatch chain before treating provider execution as dogfood evidence.\n\n## Dispatch Plan\n\n- Project: `{}`\n- Dispatch plan: `{}`\n- Adapter: `{}`\n- Provider: `{}`\n- Credential scope: `{}`\n- Agent: `{}` `{}`\n- Session: `{}`\n- Run: `{}`\n- Plan status: `{}`\n- Runtime program: `{}`\n- Runtime arg count: `{}`\n- Runtime cwd: `{}`\n- Artifact root: `{}`\n- Runtime prompt policy: `{}`\n- Provider CLI executed in plan: `{}`\n\n## Dogfood Gate\n\n- Status: `{}`\n- Ready: `{}`\n- Reasons: `{}`\n\n",
        plan.dispatch_plan_id,
        plan.project_id,
        plan.dispatch_plan_id,
        plan.adapter_kind,
        plan.provider_kind,
        plan.credential_scope,
        plan.agent_id,
        plan.agent_name,
        plan.session_id,
        plan.run_id,
        plan.status,
        plan.runtime_program,
        plan.runtime_arg_count,
        plan.runtime_cwd,
        plan.artifact_root,
        plan.runtime_prompt_policy,
        plan.provider_cli_executed,
        dogfood_gate.status,
        dogfood_gate.ready,
        dogfood_gate.reasons.join(",")
    );
    markdown.push_str("## Latest Dispatch Gate\n\n");
    if let Some(gate) = latest_gate {
        markdown.push_str(&format!(
            "- Gate: `{}`\n- Status: `{}`\n- Provider execution allowed: `{}`\n- Required dogfood gate: `{}`\n- Runtime prompt policy: `{}`\n- Provider CLI executed: `{}`\n- Reasons: `{}`\n\n",
            gate.dispatch_gate_id,
            gate.status,
            gate.provider_cli_execution_allowed,
            gate.required_dogfood_gate,
            gate.runtime_prompt_policy,
            gate.provider_cli_executed,
            gate.reason_codes
        ));
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Latest Fixture Replay\n\n");
    if let Some(replay) = latest_replay {
        markdown.push_str(&format!(
            "- Replay: `{}`\n- Gate: `{}`\n- Fixture path: `{}`\n- Fixture hash: `{}`\n- Input events: `{}`\n- Appended events: `{}`\n- Tool events: `{}`\n- Summary events: `{}`\n- Completed turns: `{}`\n- Provider CLI executed: `{}`\n- Raw content policy: `{}`\n\n",
            replay.dispatch_replay_id,
            replay.dispatch_gate_id,
            replay.fixture_path,
            replay.fixture_hash,
            replay.input_event_count,
            replay.appended_event_count,
            replay.tool_event_count,
            replay.summary_event_count,
            replay.completed_turn_count,
            replay.provider_cli_executed,
            replay.raw_content_policy
        ));
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Latest Local Execution\n\n");
    if let Some(execution) = latest_execution {
        markdown.push_str(&format!(
            "- Execution: `{}`\n- Request: `{}`\n- Status: `{}`\n- Provider execution allowed: `{}`\n- Provider CLI executed: `{}`\n- Exit code: `{}`\n- Runtime process ref: `{}`\n- Stdout artifact: `{}`\n- Stderr artifact: `{}`\n- Artifact root: `{}`\n- Credential scan: `{}`\n- Raw prompt policy: `{}`\n- Raw output policy: `{}`\n- Reasons: `{}`\n\n",
            execution.dispatch_execution_id,
            execution.execution_request_id,
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
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Observed Tool Activity\n\n");
    if tool_observations.is_empty() {
        markdown.push_str("- none\n\n");
    } else {
        for observation in tool_observations {
            markdown.push_str(&format!(
                "- Observation: `{}` name=`{}` source=`{}` observed_status=`{}` instrumentation=`{}` confidence=`{}` external_ref=`{}` artifact=`{}` raw_event_hash=`{}`\n",
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
        markdown.push('\n');
    }
    markdown.push_str("## Redaction Policy\n\n- Raw dispatch prompts are not rendered.\n- Raw provider output is not rendered.\n- Runtime stdout/stderr are referenced by artifact IDs only.\n");
    markdown
}

fn write_dispatch_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:adapter-dispatch-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo adapter dispatch evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo adapter dispatch evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
