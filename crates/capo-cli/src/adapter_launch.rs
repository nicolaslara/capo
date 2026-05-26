use std::path::PathBuf;

use capo_adapters::{ClaudeCodeAdapter, CodexExecAdapter, LocalAdapterSmokePlan};
use capo_state::{
    AdapterDispatchPlanProjection, AdapterDispatchPromptSourceProjection,
    AdapterReadinessProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
    WorkpadTaskProjection,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{controller, debug_error, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn adapter_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let record = args.iter().any(|arg| arg == "--record");
    if let Some(unknown) = args.iter().find(|arg| arg.as_str() != "--record") {
        return Err(format!("unknown adapter readiness option: {unknown}"));
    }
    let state_root = parsed.state_root.clone();
    let workspace_root = state_root.join("adapter-readiness").join("workspace");
    let artifact_root = state_root.join("adapter-readiness").join("artifacts");
    let plans = [
        CodexExecAdapter::local_smoke_plan(
            workspace_root.join("codex"),
            artifact_root.join("codex"),
        ),
        ClaudeCodeAdapter::local_smoke_plan(
            workspace_root.join("claude"),
            artifact_root.join("claude"),
        ),
    ];
    let mut output = format!(
        "adapter_readiness=true\nadapters={}\ncredential_policy=not_inspected\nreal_smoke_required_for_dogfood=true\n",
        plans.len()
    );
    let mut all_opted_in = true;
    let mut records = Vec::new();
    for plan in &plans {
        let opted_in = plan.is_opted_in();
        all_opted_in &= opted_in;
        output.push_str(&render_adapter_readiness(plan, opted_in));
        records.push(adapter_readiness_projection(plan, opted_in));
    }
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-readiness-{}",
                stable_cli_hash(&format!(
                    "{}:{}",
                    records
                        .iter()
                        .map(|record| format!(
                            "{}:{}:{}",
                            record.adapter_kind, record.opted_in, record.smoke_status
                        ))
                        .collect::<Vec<_>>()
                        .join("|"),
                    records.len()
                ))
            ),
            kind: EventKind::AdapterReadinessChecked,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: None,
            payload_json: format!(
                "{{\"adapter_count\":{},\"credential_policy\":\"not_inspected\",\"real_smoke_required_for_dogfood\":true}}",
                records.len()
            ),
            idempotency_key: Some("adapter-readiness-check:v1".to_string()),
            redaction_state: RedactionState::Safe,
        };
        let records = records
            .into_iter()
            .map(ProjectionRecord::AdapterReadiness)
            .collect::<Vec<_>>();
        Some(
            state(parsed)?
                .append_event(event, &records)
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    output.push_str(&format!(
        "ready_to_run_all_real_smokes={}\nready_for_real_agent_dogfood=false\nblocked_reason=real_subscription_smoke_not_recorded\nrecorded={}\nrecorded_sequence={}\n",
        all_opted_in,
        record,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    Ok(output)
}

pub(crate) fn plan_adapter_launch(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let adapter = required_arg(args, "--adapter")?;
    validate_local_launch_adapter(&adapter)?;
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let record = has_flag(args, "--record");
    let state_root = parsed.state_root.clone();
    let workspace = optional_arg(args, "--workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_root.join("adapter-launch").join("workspace"));
    let artifacts = optional_arg(args, "--artifacts")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_root.join("adapter-launch").join("artifacts"));
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--adapter" | "--agent" | "--goal" | "--workspace" | "--artifacts" | "--record"
            )
    }) {
        return Err(format!("unknown adapter plan-launch option: {unknown}"));
    }
    let plan = recordable_adapter_dispatch_plan(
        parsed,
        DispatchPlanRecordRequest {
            adapter: &adapter,
            agent: &agent,
            goal: &goal,
            workspace,
            artifacts,
            prompt_source: DispatchPromptSourceInput::inline_cli_prompt(),
            record,
        },
    )?;
    Ok(format!(
        "adapter_launch_planned=true\n{}\n",
        render_adapter_dispatch_plan(&plan)
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedAdapterDispatchPlan {
    pub(crate) projection: AdapterDispatchPlanProjection,
    prompt_source: AdapterDispatchPromptSourceProjection,
    runtime_safe_arg_count: usize,
    subscription_safe: bool,
    recorded: bool,
    recorded_sequence: Option<i64>,
}

pub(crate) struct DispatchPlanRecordRequest<'a> {
    pub(crate) adapter: &'a str,
    pub(crate) agent: &'a str,
    pub(crate) goal: &'a str,
    pub(crate) workspace: PathBuf,
    pub(crate) artifacts: PathBuf,
    pub(crate) prompt_source: DispatchPromptSourceInput,
    pub(crate) record: bool,
}

pub(crate) fn recordable_adapter_dispatch_plan(
    parsed: &ParsedArgs,
    request: DispatchPlanRecordRequest<'_>,
) -> Result<RecordedAdapterDispatchPlan, String> {
    let controller = controller(parsed)?;
    controller
        .registration_for_agent_name(request.agent)
        .or_else(|_| controller.register_agent(request.agent))
        .map_err(debug_error)?;
    let plan = controller
        .plan_local_adapter_dispatch(
            request.adapter,
            request.agent,
            request.goal,
            request.workspace,
            request.artifacts,
        )
        .map_err(|error| format!("adapter launch planning failed: {error}"))?;
    let safe_prompt_arg_count = plan.runtime_arg_count.saturating_sub(1);
    let goal_hash = stable_cli_hash(request.goal);
    let projection = adapter_dispatch_plan_projection(&plan, &goal_hash);
    let prompt_source_projection =
        adapter_dispatch_prompt_source_projection(&projection, &goal_hash, request.prompt_source);
    let recorded_sequence = if request.record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-plan-{}",
                stable_cli_hash(&projection.dispatch_plan_id)
            ),
            kind: EventKind::AdapterDispatchPlanned,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: Some(projection.agent_id.clone()),
            session_id: Some(projection.session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            turn_id: None,
            item_id: None,
            payload_json: format!(
                "{{\"adapter\":\"{}\",\"agent\":\"{}\",\"runtime_prompt_policy\":\"not_rendered\",\"provider_cli_executed\":false}}",
                projection.adapter_kind,
                escape_json(&projection.agent_name)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-plan:{}:{}:{}:{}:{}",
                projection.project_id,
                projection.adapter_kind,
                projection.agent_id,
                goal_hash,
                stable_cli_hash(&format!(
                    "{}:{}:{}",
                    projection.runtime_cwd, projection.artifact_root, projection.runtime_arg_count
                ))
            )),
            redaction_state: RedactionState::Safe,
        };
        let prompt_source_event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-prompt-source-{}",
                stable_cli_hash(&prompt_source_projection.prompt_source_id)
            ),
            kind: EventKind::AdapterDispatchPromptSourceRecorded,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: Some(projection.agent_id.clone()),
            session_id: Some(projection.session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            turn_id: None,
            item_id: Some(prompt_source_projection.prompt_source_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"prompt_hash\":\"{}\",\"source_kind\":\"{}\",\"raw_prompt_policy\":\"not_rendered\"}}",
                escape_json(&projection.dispatch_plan_id),
                prompt_source_projection.prompt_hash,
                escape_json(&prompt_source_projection.source_kind)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-prompt-source:{}:{}:{}:{}",
                prompt_source_projection.project_id,
                prompt_source_projection.dispatch_plan_id,
                prompt_source_projection.prompt_hash,
                prompt_source_projection.source_kind
            )),
            redaction_state: RedactionState::Safe,
        };
        let state = state(parsed)?;
        state
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchPlan(projection.clone())],
            )
            .map_err(debug_error)?;
        Some(
            state
                .append_event(
                    prompt_source_event,
                    &[ProjectionRecord::AdapterDispatchPromptSource(
                        prompt_source_projection.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(RecordedAdapterDispatchPlan {
        projection,
        prompt_source: prompt_source_projection,
        runtime_safe_arg_count: safe_prompt_arg_count,
        subscription_safe: true,
        recorded: request.record,
        recorded_sequence,
    })
}

pub(crate) fn render_adapter_dispatch_plan(plan: &RecordedAdapterDispatchPlan) -> String {
    format!(
        "adapter={}\nprovider_kind={}\ncredential_scope={}\nagent={}\nagent_id={}\nsession_id={}\nrun_id={}\nruntime_program={}\nruntime_arg_count={}\nruntime_prompt_policy={}\nruntime_prompt_source={}\nruntime_prompt_source_kind={}\nruntime_prompt_materialization={}\nruntime_safe_arg_count={}\nruntime_cwd={}\nartifact_root={}\nrequest_env_count={}\nenv_allowlist={}\nredaction_rules={}\nstdout_format={}\nstderr_policy={}\nsubscription_safe={}\nprovider_cli_executed={}\nrecorded={}\nrecorded_sequence={}",
        plan.projection.adapter_kind,
        plan.projection.provider_kind,
        plan.projection.credential_scope,
        plan.projection.agent_name,
        plan.projection.agent_id,
        plan.projection.session_id,
        plan.projection.run_id,
        plan.projection.runtime_program,
        plan.projection.runtime_arg_count,
        plan.projection.runtime_prompt_policy,
        plan.prompt_source.prompt_source_id,
        plan.prompt_source.source_kind,
        plan.prompt_source.materialization_status,
        plan.runtime_safe_arg_count,
        plan.projection.runtime_cwd,
        plan.projection.artifact_root,
        plan.projection.request_env_count,
        plan.projection.env_allowlist_count,
        plan.projection.redaction_rule_count,
        plan.projection.stdout_format,
        plan.projection.stderr_policy,
        plan.subscription_safe,
        plan.projection.provider_cli_executed,
        plan.recorded,
        plan.recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn adapter_dispatch_plan_projection(
    plan: &capo_controller::LocalAdapterDispatchPlan,
    goal_hash: &str,
) -> AdapterDispatchPlanProjection {
    AdapterDispatchPlanProjection {
        dispatch_plan_id: format!(
            "adapter-dispatch-plan-{}-{}-{}",
            plan.launch_plan.adapter_kind.as_str(),
            goal_hash,
            stable_cli_hash(&format!(
                "{}:{}:{}:{}",
                plan.agent_id,
                plan.session_id,
                plan.runtime_cwd.display(),
                plan.launch_plan.artifact_root.display()
            ))
        ),
        project_id: plan.project_id.clone(),
        adapter_kind: plan.launch_plan.adapter_kind.as_str().to_string(),
        provider_kind: plan.launch_plan.provider_kind.clone(),
        credential_scope: plan.launch_plan.credential_scope.clone(),
        agent_id: plan.agent_id.clone(),
        agent_name: plan.agent_name.clone(),
        session_id: plan.session_id.clone(),
        run_id: plan.run_id.clone(),
        runtime_program: plan.runtime_program.clone(),
        runtime_arg_count: plan.runtime_arg_count as i64,
        runtime_prompt_policy: "not_rendered".to_string(),
        runtime_cwd: plan.runtime_cwd.display().to_string(),
        artifact_root: plan.launch_plan.artifact_root.display().to_string(),
        request_env_count: plan.request_env_count as i64,
        env_allowlist_count: plan.launch_plan.env_allowlist.len() as i64,
        redaction_rule_count: plan.launch_plan.redaction_rules.len() as i64,
        stdout_format: plan.launch_plan.stdout_format.clone(),
        stderr_policy: plan.launch_plan.stderr_policy.clone(),
        provider_cli_executed: false,
        status: "planned".to_string(),
        updated_sequence: 0,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DispatchPromptSourceInput {
    source_kind: String,
    source_ref: Option<String>,
    source_hash: Option<String>,
    materialization_status: String,
}

impl DispatchPromptSourceInput {
    pub(crate) fn inline_cli_prompt() -> Self {
        Self {
            source_kind: "inline_cli_prompt".to_string(),
            source_ref: None,
            source_hash: None,
            materialization_status: "manual_prompt_not_replayable".to_string(),
        }
    }

    pub(crate) fn workpad_task(task: &WorkpadTaskProjection, source_hash: String) -> Self {
        Self {
            source_kind: "workpad_task".to_string(),
            source_ref: Some(format!("{}#{}", task.path, task.source_anchor)),
            source_hash: Some(source_hash),
            materialization_status: "replayable_if_source_hash_matches".to_string(),
        }
    }
}

fn adapter_dispatch_prompt_source_projection(
    plan: &AdapterDispatchPlanProjection,
    prompt_hash: &str,
    input: DispatchPromptSourceInput,
) -> AdapterDispatchPromptSourceProjection {
    AdapterDispatchPromptSourceProjection {
        prompt_source_id: format!(
            "adapter-dispatch-prompt-source-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!(
                "{}:{}:{}",
                prompt_hash,
                input.source_kind,
                input.source_ref.as_deref().unwrap_or("none")
            ))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        prompt_hash: prompt_hash.to_string(),
        source_kind: input.source_kind,
        source_ref: input.source_ref,
        source_hash: input.source_hash,
        materialization_status: input.materialization_status,
        raw_prompt_policy: "not_rendered".to_string(),
        updated_sequence: 0,
    }
}

pub(crate) fn validate_local_launch_adapter(adapter: &str) -> Result<(), String> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" | "claude" | "claude-code" | "claude_code" => Ok(()),
        other => Err(format!(
            "unsupported local adapter dispatch plan: {other}; expected codex or claude"
        )),
    }
}

fn render_adapter_readiness(plan: &LocalAdapterSmokePlan, opted_in: bool) -> String {
    format!(
        "adapter={} program={} opt_in_env={} opted_in={} smoke_status={} expected_marker={} env_allowlist={} redaction_rules={} output_limit_bytes={} workspace={} artifacts={}\n",
        plan.adapter_kind.as_str(),
        plan.program,
        plan.opt_in_env,
        opted_in,
        if opted_in {
            "ready_to_run"
        } else {
            "waiting_on_opt_in"
        },
        plan.expected_output_marker,
        plan.env_allowlist.len(),
        plan.redaction_rules.len(),
        plan.output_limit_bytes,
        plan.workspace_root.display(),
        plan.artifact_root.display()
    )
}

fn adapter_readiness_projection(
    plan: &LocalAdapterSmokePlan,
    opted_in: bool,
) -> AdapterReadinessProjection {
    AdapterReadinessProjection {
        adapter_kind: plan.adapter_kind.as_str().to_string(),
        project_id: project_id(),
        program: plan.program.clone(),
        opt_in_env: plan.opt_in_env.to_string(),
        opted_in,
        smoke_status: if opted_in {
            "ready_to_run".to_string()
        } else {
            "waiting_on_opt_in".to_string()
        },
        credential_policy: "not_inspected".to_string(),
        expected_marker: plan.expected_output_marker.to_string(),
        env_allowlist_count: plan.env_allowlist.len() as i64,
        redaction_rule_count: plan.redaction_rules.len() as i64,
        output_limit_bytes: plan.output_limit_bytes as i64,
        dogfood_blocker: Some("real_subscription_smoke_not_recorded".to_string()),
        updated_sequence: 0,
    }
}
