use super::*;

pub(crate) fn server_dispatch_plan(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let adapter = required_arg(args, "--adapter")?;
    require_adapter_arg(&adapter)?;
    let goal = required_arg(args, "--goal")?;
    let workspace = optional_value(args, "--workspace")?.unwrap_or_else(|| ".".to_string());
    let artifacts =
        optional_value(args, "--artifacts")?.unwrap_or_else(|| ".capo-artifacts".to_string());
    let session_id = required_arg(args, "--session")?;
    let run_id = required_arg(args, "--run")?;
    let turn_id = required_arg(args, "--turn")?;
    let deterministic_opt_in =
        std::env::var("CAPO_SERVER_DETERMINISTIC_DISPATCH").as_deref() == Ok("1");
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dispatch-plan",
            ServerCommand::PlanDispatch {
                agent_name,
                adapter,
                goal,
                workspace,
                artifacts,
                session_id,
                run_id,
                turn_id,
                deterministic_opt_in,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::DispatchPlanned(plan) = response.payload else {
        return Err("server returned unexpected response for dispatch plan".to_string());
    };
    Ok(format!(
        "{}server_dispatch_planned=true\ndispatch_plan_id={}\nprompt_source_id={}\nadapter={}\nagent={}\nsession_id={}\nrun_id={}\nruntime_program={}\nruntime_prompt_policy={}\nraw_prompt_policy={}\nprovider_cli_executed={}\ndeterministic_opt_in={}\nstatus={}\n",
        header,
        plan.dispatch_plan_id,
        plan.prompt_source_id,
        plan.adapter,
        plan.agent_name,
        plan.session_id,
        plan.run_id,
        plan.runtime_program,
        plan.runtime_prompt_policy,
        plan.raw_prompt_policy,
        plan.provider_cli_executed,
        deterministic_opt_in,
        plan.status
    ))
}

pub(crate) fn server_dispatch_gate(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dispatch-gate",
            ServerCommand::GateDispatch { dispatch_plan_id },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::DispatchGated(gate) = response.payload else {
        return Err("server returned unexpected response for dispatch gate".to_string());
    };
    Ok(format!(
        "{}server_dispatch_gated=true\ndispatch_plan_id={}\ndispatch_gate_id={}\nexecution_request_id={}\nmaterialization_id={}\nadapter={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nstatus={}\nreasons={}\nraw_prompt_policy={}\n",
        header,
        gate.dispatch_plan_id,
        gate.dispatch_gate_id,
        gate.execution_request_id,
        gate.materialization_id,
        gate.adapter,
        gate.provider_cli_execution_allowed,
        gate.provider_cli_executed,
        gate.status,
        gate.reasons,
        gate.raw_prompt_policy
    ))
}

pub(crate) fn server_dispatch_live_preflight(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let adapter = required_arg(args, "--adapter")?;
    require_live_provider_adapter_arg(&adapter)?;
    let goal = required_arg(args, "--goal")?;
    let workspace = optional_value(args, "--workspace")?.unwrap_or_else(|| ".".to_string());
    let artifacts =
        optional_value(args, "--artifacts")?.unwrap_or_else(|| ".capo-artifacts".to_string());
    let session_id = required_arg(args, "--session")?;
    let run_id = required_arg(args, "--run")?;
    let turn_id = required_arg(args, "--turn")?;
    let capability_profile = optional_value(args, "--capability-profile")?
        .unwrap_or_else(|| "trusted-local".to_string());
    let runtime_scope = optional_value(args, "--runtime-scope")?
        .unwrap_or_else(|| "local_process_loopback".to_string());
    let credential_scan_policy = optional_value(args, "--credential-scan-policy")?
        .unwrap_or_else(|| "metadata_only_no_secret_read".to_string());
    let raw_prompt_policy =
        optional_value(args, "--raw-prompt-policy")?.unwrap_or_else(|| "not_rendered".to_string());
    let raw_output_policy = optional_value(args, "--raw-output-policy")?
        .unwrap_or_else(|| "artifacts_scanned_redacted".to_string());
    let tool_wrapper_policy = optional_value(args, "--tool-wrapper-policy")?
        .unwrap_or_else(|| "capo_wrapped_required".to_string());
    let live_provider_opt_in =
        std::env::var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT").as_deref() == Ok("1");
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dispatch-live-preflight",
            ServerCommand::PreflightLiveProvider {
                agent_name,
                adapter,
                goal,
                workspace,
                artifacts,
                session_id,
                run_id,
                turn_id,
                capability_profile,
                runtime_scope,
                credential_scan_policy,
                raw_prompt_policy,
                raw_output_policy,
                tool_wrapper_policy,
                live_provider_opt_in,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = response.payload else {
        return Err("server returned unexpected response for live provider preflight".to_string());
    };
    Ok(format!(
        "{}server_dispatch_live_preflight=true\ndispatch_plan_id={}\ndispatch_gate_id={}\nexecution_request_id={}\nadapter={}\nprovider_kind={}\nagent={}\nsession_id={}\nrun_id={}\ncapability_profile={}\nruntime_scope={}\ncredential_scan_policy={}\nraw_prompt_policy={}\nraw_output_policy={}\ntool_wrapper_policy={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nstatus={}\nreasons={}\nnext_action={}\n",
        header,
        preflight.dispatch_plan_id,
        preflight.dispatch_gate_id,
        preflight.execution_request_id,
        preflight.adapter,
        preflight.provider_kind,
        preflight.agent_name,
        preflight.session_id,
        preflight.run_id,
        preflight.capability_profile,
        preflight.runtime_scope,
        preflight.credential_scan_policy,
        preflight.raw_prompt_policy,
        preflight.raw_output_policy,
        preflight.tool_wrapper_policy,
        preflight.provider_cli_execution_allowed,
        preflight.provider_cli_executed,
        preflight.status,
        preflight.reasons,
        preflight.next_action
    ))
}

pub(crate) fn server_dispatch_run_local(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    if let Ok(metadata) = fs::metadata(&fixture_path)
        && metadata.len() > MAX_ADAPTER_FIXTURE_BYTES
    {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            metadata.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    let fixture_jsonl = fs::read_to_string(&fixture_path)
        .map_err(|error| format!("failed to read adapter fixture: {error}"))?;
    if fixture_jsonl.len() as u64 > MAX_ADAPTER_FIXTURE_BYTES {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            fixture_jsonl.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dispatch-run-local",
            ServerCommand::RunDispatchLocal {
                dispatch_plan_id,
                fixture_name: fixture_path.display().to_string(),
                fixture_jsonl,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::DispatchRun(run) = response.payload else {
        return Err("server returned unexpected response for dispatch run-local".to_string());
    };
    Ok(format!(
        "{}server_dispatch_run_local=true\ndispatch_plan_id={}\ndispatch_execution_id={}\nadapter={}\nsession_id={}\nrun_id={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nstatus={}\nruntime_process_ref={}\ncredential_scan_status={}\nraw_prompt_policy={}\nraw_output_policy={}\nreason_codes={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\n",
        header,
        run.dispatch_plan_id,
        run.dispatch_execution_id,
        run.adapter,
        run.session_id,
        run.run_id,
        run.provider_cli_execution_allowed,
        run.provider_cli_executed,
        run.status,
        run.runtime_process_ref
            .unwrap_or_else(|| "none".to_string()),
        run.credential_scan_status,
        run.raw_prompt_policy,
        run.raw_output_policy,
        run.reason_codes,
        run.input_event_count,
        run.appended_event_count,
        run.tool_event_count,
        run.summary_event_count,
        run.completed_turn_count
    ))
}

pub(crate) fn server_dispatch_live_run_local(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let goal = required_arg(args, "--goal")?;
    let timeout_seconds = optional_value(args, "--timeout-seconds")?
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| "--timeout-seconds must be a non-negative integer".to_string())
        })
        .transpose()?
        .unwrap_or(300);
    let mock_fixture_path = optional_value(args, "--mock-fixture")?.map(PathBuf::from);
    let mock_provider_output_jsonl = mock_fixture_path
        .as_ref()
        .map(read_bounded_fixture)
        .transpose()?;
    let mock_provider_output_name = mock_fixture_path.map(|path| path.display().to_string());
    let live_execution_opt_in = std::env::var("CAPO_SERVER_RUN_CODEX_LIVE").as_deref() == Ok("1");
    let mock_runtime_opt_in =
        std::env::var("CAPO_SERVER_MOCK_LIVE_PROVIDER_RUNTIME").as_deref() == Ok("1");
    // RTL9: a live workspace write must be attended. The CLI defaults to
    // unattended (read-only/dry-run); `--attended` opts in to a live write, which
    // still also needs `CAPO_SERVER_RUN_CODEX_LIVE` and the caller opt-in.
    let unattended = !args.iter().any(|arg| arg == "--attended");
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dispatch-live-run-local",
            ServerCommand::RunLiveProviderLocal {
                dispatch_plan_id,
                goal,
                live_execution_opt_in,
                mock_runtime_opt_in,
                mock_provider_output_name,
                mock_provider_output_jsonl,
                timeout_seconds,
                // The spawn-path codex binary is resolved server-side from
                // `CAPO_CODEX_BIN`; the CLI passes no explicit override.
                codex_program_override: None,
                unattended,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::DispatchRun(run) = response.payload else {
        return Err("server returned unexpected response for live run-local".to_string());
    };
    Ok(format!(
        "{}server_dispatch_live_run_local=true\ndispatch_plan_id={}\ndispatch_execution_id={}\nadapter={}\nsession_id={}\nrun_id={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nlive_execution_opt_in={}\nmock_runtime_opt_in={}\nstatus={}\nruntime_process_ref={}\ncredential_scan_status={}\nraw_prompt_policy={}\nraw_output_policy={}\nreason_codes={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\n",
        header,
        run.dispatch_plan_id,
        run.dispatch_execution_id,
        run.adapter,
        run.session_id,
        run.run_id,
        run.provider_cli_execution_allowed,
        run.provider_cli_executed,
        live_execution_opt_in,
        mock_runtime_opt_in,
        run.status,
        run.runtime_process_ref
            .unwrap_or_else(|| "none".to_string()),
        run.credential_scan_status,
        run.raw_prompt_policy,
        run.raw_output_policy,
        run.reason_codes,
        run.input_event_count,
        run.appended_event_count,
        run.tool_event_count,
        run.summary_event_count,
        run.completed_turn_count
    ))
}
