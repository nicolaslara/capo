use std::env;

use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{
    AdapterDispatchExecutionRequestProjection, AdapterDispatchGateProjection,
    AdapterDispatchPlanProjection, AdapterDispatchPromptMaterializationProjection,
    AdapterDispatchPromptSourceProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
    SqliteStateStore,
};

use crate::cli_surface::{ParsedArgs, has_flag, required_arg};
use crate::{debug_error, escape_json, project_id, stable_cli_hash, state, workpad_task_goal};

pub(crate) fn adapter_dispatch_execution_request(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!(
            "unknown adapter execution-request option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
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
    let request = adapter_dispatch_execution_request_projection(plan, latest_gate);
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-execution-request-{}",
                stable_cli_hash(&request.execution_request_id)
            ),
            kind: EventKind::AdapterDispatchExecutionRequested,
            actor: "local-cli".to_string(),
            project_id: Some(request.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(request.execution_request_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"dispatch_gate_id\":\"{}\",\"provider_cli_execution_allowed\":{},\"provider_cli_executed\":false,\"status\":\"{}\"}}",
                escape_json(&request.dispatch_plan_id),
                escape_json(&request.dispatch_gate_id),
                request.provider_cli_execution_allowed,
                escape_json(&request.status)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-execution-request:{}:{}:{}",
                request.project_id, request.dispatch_plan_id, request.status
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchExecutionRequest(
                        request.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_execution_request(
        &request,
        record,
        recorded_sequence,
    ))
}

pub(crate) fn adapter_dispatch_materialize_prompt(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!(
            "unknown adapter materialize-prompt option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let prompt_source = dashboard
        .adapter_dispatch_prompt_sources
        .iter()
        .rev()
        .find(|source| source.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| {
            format!("dispatch plan has no recorded prompt source: {dispatch_plan_id}")
        })?;
    let materialization =
        adapter_dispatch_prompt_materialization_projection(&state, prompt_source)?;
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-prompt-materialization-{}",
                stable_cli_hash(&materialization.materialization_id)
            ),
            kind: EventKind::AdapterDispatchPromptMaterialized,
            actor: "local-cli".to_string(),
            project_id: Some(materialization.project_id.clone()),
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: Some(materialization.materialization_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"prompt_source_id\":\"{}\",\"status\":\"{}\",\"raw_prompt_policy\":\"not_rendered\"}}",
                escape_json(&materialization.dispatch_plan_id),
                escape_json(&materialization.prompt_source_id),
                escape_json(&materialization.status)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-prompt-materialization:{}:{}:{}:{}",
                materialization.project_id,
                materialization.prompt_source_id,
                materialization.status,
                materialization.materialization_id
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                        materialization.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_prompt_materialization(
        &materialization,
        record,
        recorded_sequence,
    ))
}

pub(crate) fn adapter_dispatch_run_preflight(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan"))
    {
        return Err(format!("unknown adapter run-preflight option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let execution_request = dashboard
        .adapter_dispatch_execution_requests
        .iter()
        .rev()
        .find(|request| request.dispatch_plan_id == plan.dispatch_plan_id);
    let materialization = dashboard
        .adapter_dispatch_prompt_materializations
        .iter()
        .rev()
        .find(|row| row.dispatch_plan_id == plan.dispatch_plan_id);
    Ok(render_adapter_dispatch_run_preflight(
        &dispatch_run_preflight(plan, execution_request, materialization),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdapterDispatchRunPreflight {
    pub(crate) dispatch_plan_id: String,
    pub(crate) adapter_kind: String,
    pub(crate) execution_request_id: String,
    pub(crate) materialization_id: String,
    pub(crate) provider_cli_execution_allowed: bool,
    pub(crate) opt_in_env: String,
    pub(crate) opt_in_set: bool,
    pub(crate) status: String,
    pub(crate) runtime_prompt_policy: String,
    pub(crate) raw_prompt_policy: String,
    pub(crate) reasons: Vec<String>,
    pub(crate) next_action: String,
}

pub(crate) fn dispatch_run_preflight(
    plan: &AdapterDispatchPlanProjection,
    execution_request: Option<&AdapterDispatchExecutionRequestProjection>,
    materialization: Option<&AdapterDispatchPromptMaterializationProjection>,
) -> AdapterDispatchRunPreflight {
    let opt_in_env = adapter_dispatch_opt_in_env(&plan.adapter_kind);
    let opt_in_set = env::var(&opt_in_env).as_deref() == Ok("1");
    let mut reasons = Vec::new();
    let mut status = "ready_to_execute_provider_cli".to_string();

    match execution_request {
        Some(request)
            if request.provider_cli_execution_allowed
                && request.status == "waiting_on_explicit_provider_opt_in"
                && !request.provider_cli_executed => {}
        Some(request) => {
            status = "blocked_execution_request_not_ready".to_string();
            reasons.push(request.status.clone());
        }
        None => {
            status = "blocked_missing_execution_request".to_string();
            reasons.push("recorded_execution_request_missing".to_string());
        }
    }

    match materialization {
        Some(row) if row.status == "ready_without_rendering_prompt" => {}
        Some(row) => {
            if status == "ready_to_execute_provider_cli" {
                status = "blocked_prompt_materialization_not_ready".to_string();
            }
            reasons.push(row.status.clone());
        }
        None => {
            if status == "ready_to_execute_provider_cli" {
                status = "blocked_missing_prompt_materialization".to_string();
            }
            reasons.push("recorded_prompt_materialization_missing".to_string());
        }
    }

    if !opt_in_set {
        if status == "ready_to_execute_provider_cli" {
            status = "blocked_missing_explicit_provider_opt_in".to_string();
        }
        reasons.push(format!("{opt_in_env}=1_required"));
    }
    if reasons.is_empty() {
        reasons.push("all_preflight_checks_passed".to_string());
    }
    let provider_cli_execution_allowed = status == "ready_to_execute_provider_cli";
    let next_action = if provider_cli_execution_allowed {
        "execute_provider_cli_through_runtime_runner"
    } else {
        "resolve_preflight_blockers_before_provider_execution"
    };

    AdapterDispatchRunPreflight {
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        execution_request_id: execution_request
            .map(|request| request.execution_request_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        materialization_id: materialization
            .map(|row| row.materialization_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        provider_cli_execution_allowed,
        opt_in_env,
        opt_in_set,
        status,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        raw_prompt_policy: materialization
            .map(|row| row.raw_prompt_policy.clone())
            .unwrap_or_else(|| "none".to_string()),
        reasons,
        next_action: next_action.to_string(),
    }
}

fn render_adapter_dispatch_run_preflight(preflight: &AdapterDispatchRunPreflight) -> String {
    format!(
        "adapter_dispatch_run_preflight=true\ndispatch_plan={}\nadapter={}\nexecution_request={}\nprompt_materialization={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed=false\nopt_in_env={}\nopt_in_set={}\nstatus={}\nruntime_prompt_policy={}\nraw_prompt_policy={}\nreasons={}\nnext_action={}\n",
        preflight.dispatch_plan_id,
        preflight.adapter_kind,
        preflight.execution_request_id,
        preflight.materialization_id,
        preflight.provider_cli_execution_allowed,
        preflight.opt_in_env,
        preflight.opt_in_set,
        preflight.status,
        preflight.runtime_prompt_policy,
        preflight.raw_prompt_policy,
        preflight.reasons.join(","),
        preflight.next_action
    )
}

fn adapter_dispatch_prompt_materialization_projection(
    state: &SqliteStateStore,
    source: &AdapterDispatchPromptSourceProjection,
) -> Result<AdapterDispatchPromptMaterializationProjection, String> {
    let mut observed_source_hash = None;
    let mut materialized_prompt_hash = None;
    let mut status = "blocked_non_replayable_prompt".to_string();
    let mut reasons = vec!["manual_prompt_not_replayable".to_string()];

    if source.source_kind == "workpad_task" {
        reasons.clear();
        let source_ref = source
            .source_ref
            .as_deref()
            .ok_or_else(|| "workpad prompt source missing source_ref".to_string())?;
        let (path, anchor) = split_source_ref(source_ref)?;
        let workpad_file = state
            .workpad_file(&source.project_id, path)
            .map_err(debug_error)?;
        let workpad_file = match workpad_file {
            Some(file) => file,
            None => {
                status = "blocked_missing_source".to_string();
                reasons.push("workpad_file_missing".to_string());
                return Ok(prompt_materialization_row(
                    source,
                    observed_source_hash,
                    materialized_prompt_hash,
                    status,
                    reasons,
                ));
            }
        };
        observed_source_hash = Some(workpad_file.content_hash.clone());
        if source.source_hash.as_deref() != Some(workpad_file.content_hash.as_str()) {
            status = "blocked_source_hash_mismatch".to_string();
            reasons.push("workpad_source_hash_mismatch".to_string());
            return Ok(prompt_materialization_row(
                source,
                observed_source_hash,
                materialized_prompt_hash,
                status,
                reasons,
            ));
        }
        let task = state
            .workpad_tasks(&source.project_id)
            .map_err(debug_error)?
            .into_iter()
            .find(|task| task.path == path && task.source_anchor == anchor);
        let Some(task) = task else {
            status = "blocked_missing_source".to_string();
            reasons.push("workpad_task_missing".to_string());
            return Ok(prompt_materialization_row(
                source,
                observed_source_hash,
                materialized_prompt_hash,
                status,
                reasons,
            ));
        };
        let prompt_hash = stable_cli_hash(&workpad_task_goal(&task));
        materialized_prompt_hash = Some(prompt_hash.clone());
        if prompt_hash == source.prompt_hash {
            status = "ready_without_rendering_prompt".to_string();
            reasons.push("prompt_hash_matches_source".to_string());
        } else {
            status = "blocked_prompt_hash_mismatch".to_string();
            reasons.push("prompt_hash_mismatch".to_string());
        }
    }

    Ok(prompt_materialization_row(
        source,
        observed_source_hash,
        materialized_prompt_hash,
        status,
        reasons,
    ))
}

fn prompt_materialization_row(
    source: &AdapterDispatchPromptSourceProjection,
    observed_source_hash: Option<String>,
    materialized_prompt_hash: Option<String>,
    status: String,
    reasons: Vec<String>,
) -> AdapterDispatchPromptMaterializationProjection {
    AdapterDispatchPromptMaterializationProjection {
        materialization_id: format!(
            "adapter-dispatch-prompt-materialization-{}-{}",
            stable_cli_hash(&source.prompt_source_id),
            stable_cli_hash(&format!(
                "{}:{}",
                status,
                observed_source_hash.as_deref().unwrap_or("none")
            ))
        ),
        project_id: source.project_id.clone(),
        dispatch_plan_id: source.dispatch_plan_id.clone(),
        prompt_source_id: source.prompt_source_id.clone(),
        source_kind: source.source_kind.clone(),
        source_ref: source.source_ref.clone(),
        expected_source_hash: source.source_hash.clone(),
        observed_source_hash,
        expected_prompt_hash: source.prompt_hash.clone(),
        materialized_prompt_hash,
        status,
        raw_prompt_policy: "not_rendered".to_string(),
        reason_codes: reasons.join(","),
        updated_sequence: 0,
    }
}

pub(crate) fn split_source_ref(source_ref: &str) -> Result<(&str, &str), String> {
    source_ref
        .split_once('#')
        .ok_or_else(|| format!("invalid prompt source ref: {source_ref}"))
}

fn adapter_dispatch_execution_request_projection(
    plan: &AdapterDispatchPlanProjection,
    latest_gate: Option<&AdapterDispatchGateProjection>,
) -> AdapterDispatchExecutionRequestProjection {
    let ready_gate = latest_gate.filter(|gate| {
        gate.provider_cli_execution_allowed
            && gate.status == "ready_for_execution"
            && !gate.provider_cli_executed
            && gate.runtime_prompt_policy == "not_rendered"
    });
    let opt_in_env = adapter_dispatch_opt_in_env(&plan.adapter_kind);
    let (dispatch_gate_id, allowed, status, reason_codes) = if let Some(gate) = ready_gate {
        (
            gate.dispatch_gate_id.clone(),
            true,
            "waiting_on_explicit_provider_opt_in".to_string(),
            "explicit_provider_execution_opt_in_required".to_string(),
        )
    } else if let Some(gate) = latest_gate {
        (
            gate.dispatch_gate_id.clone(),
            false,
            "blocked_by_dispatch_gate".to_string(),
            gate.reason_codes.clone(),
        )
    } else {
        (
            "none".to_string(),
            false,
            "blocked_missing_ready_gate".to_string(),
            "recorded_ready_dispatch_gate_missing".to_string(),
        )
    };

    AdapterDispatchExecutionRequestProjection {
        execution_request_id: format!(
            "adapter-dispatch-execution-request-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!("{status}:{dispatch_gate_id}"))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        dispatch_gate_id,
        adapter_kind: plan.adapter_kind.clone(),
        provider_cli_execution_allowed: allowed,
        provider_cli_executed: false,
        status,
        opt_in_env,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        reason_codes,
        updated_sequence: 0,
    }
}

fn adapter_dispatch_opt_in_env(adapter_kind: &str) -> String {
    match adapter_kind {
        "codex_exec" => "CAPO_RUN_CODEX_LOCAL_DISPATCH",
        "claude_code" => "CAPO_RUN_CLAUDE_LOCAL_DISPATCH",
        _ => "CAPO_RUN_LOCAL_ADAPTER_DISPATCH",
    }
    .to_string()
}

fn render_adapter_dispatch_execution_request(
    request: &AdapterDispatchExecutionRequestProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_execution_request=true\nexecution_request={}\ndispatch_plan={}\ndispatch_gate={}\nadapter={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nstatus={}\nopt_in_env={}\nruntime_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        request.execution_request_id,
        request.dispatch_plan_id,
        request.dispatch_gate_id,
        request.adapter_kind,
        request.provider_cli_execution_allowed,
        request.provider_cli_executed,
        request.status,
        request.opt_in_env,
        request.runtime_prompt_policy,
        request.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_adapter_dispatch_prompt_materialization(
    materialization: &AdapterDispatchPromptMaterializationProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_prompt_materialization=true\nmaterialization={}\ndispatch_plan={}\nprompt_source={}\nsource_kind={}\nsource_ref={}\nexpected_source_hash={}\nobserved_source_hash={}\nexpected_prompt_hash={}\nmaterialized_prompt_hash={}\nstatus={}\nraw_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        materialization.materialization_id,
        materialization.dispatch_plan_id,
        materialization.prompt_source_id,
        materialization.source_kind,
        materialization.source_ref.as_deref().unwrap_or("none"),
        materialization
            .expected_source_hash
            .as_deref()
            .unwrap_or("none"),
        materialization
            .observed_source_hash
            .as_deref()
            .unwrap_or("none"),
        materialization.expected_prompt_hash,
        materialization
            .materialized_prompt_hash
            .as_deref()
            .unwrap_or("none"),
        materialization.status,
        materialization.raw_prompt_policy,
        materialization.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}
