use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use capo_adapters::{
    ClaudeCodeAdapter, CodexExecAdapter, LocalAdapterLaunchPlan, LocalAdapterSmokeError,
    scan_artifacts_for_sensitive_markers,
};
use capo_controller::{AdapterReplayReport, LocalAdapterDispatchRunStart};
use capo_core::{RunId, TaskId};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_runtime::LocalProcessRunner;
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchPlanProjection, AdapterDispatchPromptMaterializationProjection,
    AdapterDispatchPromptSourceProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
    SqliteStateStore,
};

use crate::adapter_dispatch_prepare::{
    AdapterDispatchRunPreflight, dispatch_run_preflight, split_source_ref,
};
use crate::adapter_launch::dispatch_proof_prompt;
use crate::adapter_replay::parse_adapter_fixture;
use crate::adapter_smoke::format_smoke_scan_error;
use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::evidence::export_evidence;
use crate::workpad::workpad_task_goal;
use crate::{controller, debug_error, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn scan_dispatch_artifacts_or_delete<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> Result<(), LocalAdapterSmokeError> {
    let paths = paths.into_iter().cloned().collect::<Vec<_>>();
    match scan_artifacts_for_sensitive_markers(paths.iter()) {
        Ok(()) => Ok(()),
        Err(error) => {
            for path in &paths {
                let _ = fs::remove_file(path);
            }
            Err(error)
        }
    }
}

pub(crate) fn adapter_dispatch_run_local(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    let out = optional_arg(args, "--out");
    let timeout_seconds = optional_arg(args, "--timeout-seconds")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("invalid --timeout-seconds value: {value}"))
        })
        .transpose()?
        .unwrap_or(300);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--dispatch-plan" | "--record" | "--out" | "--timeout-seconds"
            )
    }) {
        return Err(format!("unknown adapter run-local option: {unknown}"));
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
    let preflight = dispatch_run_preflight(plan, execution_request, materialization);
    if !preflight.provider_cli_execution_allowed {
        let execution = adapter_dispatch_execution_projection(
            plan,
            execution_request,
            &preflight,
            AdapterDispatchExecutionRuntimeOutcome::blocked(),
        );
        let recorded_sequence = if record {
            Some(record_adapter_dispatch_execution(&state, plan, &execution)?)
        } else {
            None
        };
        return Ok(render_adapter_dispatch_run_local_blocked(
            &preflight,
            record,
            recorded_sequence,
        ));
    }

    let prompt_source = dashboard
        .adapter_dispatch_prompt_sources
        .iter()
        .rev()
        .find(|source| source.dispatch_plan_id == plan.dispatch_plan_id)
        .ok_or_else(|| {
            format!(
                "dispatch plan has no prompt source for local run: {}",
                plan.dispatch_plan_id
            )
        })?;
    let launch_plan = build_dispatch_run_launch_plan(&state, plan, prompt_source, materialization)?;
    launch_plan.assert_subscription_safe()?;
    fs::create_dir_all(&launch_plan.workspace_root)
        .map_err(|error| format!("failed to create dispatch workspace: {error}"))?;
    fs::create_dir_all(&launch_plan.artifact_root)
        .map_err(|error| format!("failed to create dispatch artifact root: {error}"))?;
    let runner = LocalProcessRunner::new(launch_plan.runtime_config());
    let mut process = runner
        .spawn_process(launch_plan.runtime_request(RunId::new(plan.run_id.to_string())))
        .map_err(LocalAdapterSmokeError::Runtime)
        .map_err(format_smoke_scan_error)?;
    let outcome = runner
        .wait_running_with_timeout(&mut process, Duration::from_secs(timeout_seconds))
        .map_err(LocalAdapterSmokeError::Runtime)
        .map_err(format_smoke_scan_error)?;
    scan_dispatch_artifacts_or_delete([&outcome.stdout.path, &outcome.stderr.path])
        .map_err(format_smoke_scan_error)?;
    if outcome.process.status != "exited" {
        let execution = adapter_dispatch_execution_projection(
            plan,
            execution_request,
            &preflight,
            AdapterDispatchExecutionRuntimeOutcome {
                provider_cli_executed: true,
                status: outcome.process.status.clone(),
                exit_code: outcome.exit_code.map(i64::from),
                runtime_process_ref: Some(outcome.process.runtime_process_ref.clone()),
                stdout_artifact_id: Some(outcome.stdout.artifact_id.clone()),
                stderr_artifact_id: Some(outcome.stderr.artifact_id.clone()),
                credential_scan_status: "clean".to_string(),
                raw_output_policy: "bounded_redacted_artifacts".to_string(),
                reason_codes: format!("provider_cli_{}", outcome.process.status),
            },
        );
        let recorded_sequence = record_adapter_dispatch_execution(&state, plan, &execution)?;
        return Ok(format!(
            "adapter_dispatch_run_local=true
dispatch_execution={}
dispatch_plan={}
adapter={}
provider_cli_execution_allowed=true
provider_cli_executed=true
status={}
runtime_process_ref={}
exit_code={}
stdout_artifact={}
stderr_artifact={}
artifact_root={}
raw_prompt_policy={}
raw_output_policy=bounded_redacted_artifacts
adapter_stream_ingested=false
recorded=true
recorded_sequence={}
",
            execution.dispatch_execution_id,
            plan.dispatch_plan_id,
            plan.adapter_kind,
            outcome.process.status,
            outcome.process.runtime_process_ref,
            outcome
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            outcome.stdout.artifact_id,
            outcome.stderr.artifact_id,
            launch_plan.artifact_root.display(),
            materialization
                .map(|row| row.raw_prompt_policy.as_str())
                .unwrap_or("none"),
            recorded_sequence
        ));
    }
    let adapter_stdout = fs::read_to_string(&outcome.stdout.path)
        .map_err(|error| format!("failed to read adapter stdout artifact: {error}"))?;
    let ingestion = apply_dispatch_adapter_output(
        parsed,
        plan,
        &adapter_stdout,
        outcome.process.runtime_process_ref.clone(),
        out,
    )?;
    let execution = adapter_dispatch_execution_projection(
        plan,
        execution_request,
        &preflight,
        AdapterDispatchExecutionRuntimeOutcome {
            provider_cli_executed: true,
            status: outcome.process.status.clone(),
            exit_code: outcome.exit_code.map(i64::from),
            runtime_process_ref: Some(outcome.process.runtime_process_ref.clone()),
            stdout_artifact_id: Some(outcome.stdout.artifact_id.clone()),
            stderr_artifact_id: Some(outcome.stderr.artifact_id.clone()),
            credential_scan_status: "clean".to_string(),
            raw_output_policy: "bounded_redacted_artifacts".to_string(),
            reason_codes: "provider_cli_executed_and_artifacts_scanned".to_string(),
        },
    );
    let recorded_sequence = record_adapter_dispatch_execution(&state, plan, &execution)?;
    Ok(format!(
        "adapter_dispatch_run_local=true
dispatch_execution={}
dispatch_plan={}
adapter={}
provider_cli_execution_allowed=true
provider_cli_executed=true
status={}
runtime_process_ref={}
exit_code={}
stdout_artifact={}
stderr_artifact={}
artifact_root={}
raw_prompt_policy={}
raw_output_policy=bounded_redacted_artifacts
adapter_stream_ingested=true
adapter_stream_input_events={}
adapter_stream_appended_events={}
adapter_stream_tool_events={}
adapter_stream_summary_events={}
adapter_stream_completed_turns={}
recorded=true
recorded_sequence={}
{}",
        execution.dispatch_execution_id,
        plan.dispatch_plan_id,
        plan.adapter_kind,
        outcome.process.status,
        outcome.process.runtime_process_ref,
        outcome
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        outcome.stdout.artifact_id,
        outcome.stderr.artifact_id,
        launch_plan.artifact_root.display(),
        materialization
            .map(|row| row.raw_prompt_policy.as_str())
            .unwrap_or("none"),
        ingestion.report.input_event_count,
        ingestion.report.appended_event_count,
        ingestion.report.tool_event_count,
        ingestion.report.summary_event_count,
        ingestion.report.completed_turn_count,
        recorded_sequence,
        ingestion.evidence_output
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DispatchAdapterOutputIngestion {
    pub(crate) report: AdapterReplayReport,
    pub(crate) evidence_output: String,
}

pub(crate) fn apply_dispatch_adapter_output(
    parsed: &ParsedArgs,
    plan: &AdapterDispatchPlanProjection,
    adapter_output: &str,
    runtime_process_ref: String,
    out: Option<String>,
) -> Result<DispatchAdapterOutputIngestion, String> {
    let adapter_events = parse_adapter_fixture(&plan.adapter_kind, adapter_output)?;
    if adapter_events.is_empty() {
        return Err(format!(
            "adapter output for dispatch plan {} produced no normalized adapter events",
            plan.dispatch_plan_id
        ));
    }
    let controller = controller(parsed)?;
    let refs = controller
        .prepare_local_adapter_dispatch_run(LocalAdapterDispatchRunStart {
            agent_name: plan.agent_name.clone(),
            task_id: TaskId::new(format!(
                "task-adapter-dispatch-{}",
                stable_cli_hash(&plan.dispatch_plan_id)
            )),
            session_id: plan.session_id.clone(),
            run_id: plan.run_id.clone(),
            goal: format!(
                "Ingest real adapter output for dispatch plan {}",
                plan.dispatch_plan_id
            ),
            runtime_process_ref,
            external_session_ref: format!("local-adapter-session-{}", plan.dispatch_plan_id),
        })
        .map_err(debug_error)?;
    if refs.session_id != plan.session_id || refs.run_id != plan.run_id {
        return Err(format!(
            "dispatch run output ref mismatch for {}: expected session={} run={}, got session={} run={}",
            plan.dispatch_plan_id, plan.session_id, plan.run_id, refs.session_id, refs.run_id
        ));
    }
    let report = controller
        .apply_normalized_adapter_events(&refs, &adapter_events)
        .map_err(debug_error)?;
    let evidence_output = if let Some(out) = out {
        export_evidence(
            parsed,
            &[
                "--session".to_string(),
                refs.session_id.to_string(),
                "--out".to_string(),
                out,
            ],
        )?
    } else {
        String::new()
    };
    Ok(DispatchAdapterOutputIngestion {
        report,
        evidence_output,
    })
}

fn render_adapter_dispatch_run_local_blocked(
    preflight: &AdapterDispatchRunPreflight,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_run_local=true
dispatch_plan={}
adapter={}
execution_request={}
prompt_materialization={}
provider_cli_execution_allowed=false
provider_cli_executed=false
opt_in_env={}
opt_in_set={}
status={}
runtime_prompt_policy={}
raw_prompt_policy={}
reasons={}
next_action={}
recorded={}
recorded_sequence={}
",
        preflight.dispatch_plan_id,
        preflight.adapter_kind,
        preflight.execution_request_id,
        preflight.materialization_id,
        preflight.opt_in_env,
        preflight.opt_in_set,
        preflight.status,
        preflight.runtime_prompt_policy,
        preflight.raw_prompt_policy,
        preflight.reasons.join(","),
        preflight.next_action,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdapterDispatchExecutionRuntimeOutcome {
    provider_cli_executed: bool,
    status: String,
    exit_code: Option<i64>,
    runtime_process_ref: Option<String>,
    stdout_artifact_id: Option<String>,
    stderr_artifact_id: Option<String>,
    credential_scan_status: String,
    raw_output_policy: String,
    reason_codes: String,
}

impl AdapterDispatchExecutionRuntimeOutcome {
    fn blocked() -> Self {
        Self {
            provider_cli_executed: false,
            status: "blocked_by_preflight".to_string(),
            exit_code: None,
            runtime_process_ref: None,
            stdout_artifact_id: None,
            stderr_artifact_id: None,
            credential_scan_status: "not_run".to_string(),
            raw_output_policy: "not_captured".to_string(),
            reason_codes: "preflight_blocked_provider_execution".to_string(),
        }
    }
}

fn adapter_dispatch_execution_projection(
    plan: &AdapterDispatchPlanProjection,
    execution_request: Option<&AdapterDispatchExecutionRequestProjection>,
    preflight: &AdapterDispatchRunPreflight,
    outcome: AdapterDispatchExecutionRuntimeOutcome,
) -> AdapterDispatchExecutionProjection {
    let reason_codes = if outcome.provider_cli_executed {
        outcome.reason_codes
    } else {
        preflight.reasons.join(",")
    };
    AdapterDispatchExecutionProjection {
        dispatch_execution_id: format!(
            "adapter-dispatch-execution-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!(
                "{}:{}:{}:{}:{}",
                preflight.status,
                outcome.provider_cli_executed,
                reason_codes,
                outcome.runtime_process_ref.as_deref().unwrap_or("none"),
                outcome.stdout_artifact_id.as_deref().unwrap_or("none")
            ))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        execution_request_id: execution_request
            .map(|request| request.execution_request_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        adapter_kind: plan.adapter_kind.clone(),
        session_id: plan.session_id.clone(),
        run_id: plan.run_id.clone(),
        provider_cli_execution_allowed: preflight.provider_cli_execution_allowed,
        provider_cli_executed: outcome.provider_cli_executed,
        status: if outcome.provider_cli_executed {
            outcome.status
        } else {
            preflight.status.clone()
        },
        exit_code: outcome.exit_code,
        runtime_process_ref: outcome.runtime_process_ref,
        stdout_artifact_id: outcome.stdout_artifact_id,
        stderr_artifact_id: outcome.stderr_artifact_id,
        artifact_root: plan.artifact_root.clone(),
        credential_scan_status: outcome.credential_scan_status,
        raw_prompt_policy: preflight.raw_prompt_policy.clone(),
        raw_output_policy: outcome.raw_output_policy,
        reason_codes,
        updated_sequence: 0,
    }
}

fn record_adapter_dispatch_execution(
    state: &SqliteStateStore,
    plan: &AdapterDispatchPlanProjection,
    execution: &AdapterDispatchExecutionProjection,
) -> Result<i64, String> {
    let event = NewEvent {
        event_id: format!(
            "event-adapter-dispatch-execution-{}",
            stable_cli_hash(&execution.dispatch_execution_id)
        ),
        kind: EventKind::AdapterDispatchExecuted,
        actor: "local-cli".to_string(),
        project_id: Some(execution.project_id.clone()),
        task_id: None,
        agent_id: Some(plan.agent_id.clone()),
        session_id: Some(execution.session_id.clone()),
        run_id: Some(execution.run_id.clone()),
        turn_id: None,
        item_id: Some(execution.dispatch_execution_id.clone()),
        payload_json: format!(
            "{{\"dispatch_plan_id\":\"{}\",\"execution_request_id\":\"{}\",\"provider_cli_executed\":{},\"status\":\"{}\",\"raw_prompt_policy\":\"{}\",\"raw_output_policy\":\"{}\"}}",
            escape_json(&execution.dispatch_plan_id),
            escape_json(&execution.execution_request_id),
            execution.provider_cli_executed,
            escape_json(&execution.status),
            escape_json(&execution.raw_prompt_policy),
            escape_json(&execution.raw_output_policy)
        ),
        idempotency_key: Some(format!(
            "adapter-dispatch-execution:{}:{}:{}:{}:{}",
            execution.project_id,
            execution.dispatch_plan_id,
            execution.execution_request_id,
            execution.status,
            execution.dispatch_execution_id
        )),
        redaction_state: RedactionState::Safe,
    };
    state
        .append_event(
            event,
            &[ProjectionRecord::AdapterDispatchExecution(
                execution.clone(),
            )],
        )
        .map_err(debug_error)
}

fn build_dispatch_run_launch_plan(
    state: &SqliteStateStore,
    plan: &AdapterDispatchPlanProjection,
    source: &AdapterDispatchPromptSourceProjection,
    materialization: Option<&AdapterDispatchPromptMaterializationProjection>,
) -> Result<LocalAdapterLaunchPlan, String> {
    let materialization = materialization
        .filter(|row| row.status == "ready_without_rendering_prompt")
        .ok_or_else(|| "dispatch prompt is not materialized for local run".to_string())?;
    if materialization.prompt_source_id != source.prompt_source_id {
        return Err(format!(
            "dispatch prompt materialization source mismatch for local run: expected {}, got {}",
            source.prompt_source_id, materialization.prompt_source_id
        ));
    }
    let prompt = match source.source_kind.as_str() {
        "dispatch_proof" => dispatch_proof_prompt().to_string(),
        "workpad_task" => {
            let source_ref = source
                .source_ref
                .as_deref()
                .ok_or_else(|| "workpad prompt source missing source_ref".to_string())?;
            let (path, anchor) = split_source_ref(source_ref)?;
            let task = state
                .workpad_tasks(&source.project_id)
                .map_err(debug_error)?
                .into_iter()
                .find(|task| task.path == path && task.source_anchor == anchor)
                .ok_or_else(|| format!("workpad prompt source is missing: {source_ref}"))?;
            workpad_task_goal(&task)
        }
        other => {
            return Err(format!(
                "dispatch prompt source is not replayable for local run: {other}"
            ));
        }
    };
    let prompt_hash = stable_cli_hash(&prompt);
    if materialization.materialized_prompt_hash.as_deref() != Some(prompt_hash.as_str()) {
        return Err("dispatch prompt materialization no longer matches source".to_string());
    }
    let workspace_root = PathBuf::from(&plan.runtime_cwd);
    let artifact_root = PathBuf::from(&plan.artifact_root);
    match plan.adapter_kind.as_str() {
        "codex_exec" => Ok(CodexExecAdapter::local_launch_plan(
            workspace_root,
            artifact_root,
            prompt,
        )),
        "claude_code" => Ok(ClaudeCodeAdapter::local_launch_plan(
            workspace_root,
            artifact_root,
            prompt,
        )),
        other => Err(format!("unsupported adapter for local run: {other}")),
    }
}
