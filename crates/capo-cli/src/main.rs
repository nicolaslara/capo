use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    SessionId, ToolCallId,
};
use capo_query::{
    ProjectDashboardQuery, ProjectDogfoodReadiness, project_dashboard, project_dogfood_readiness,
};
use capo_state::{
    ArtifactRecord, CapabilityGrantProjection, EventKind, EvidenceProjection, NewEvent,
    PermissionApprovalProjection, ProjectionRecord, RedactionState, SqliteStateStore,
    ToolCallProjection, ToolObservationProjection,
};

mod adapter_dispatch;
mod adapter_dispatch_prepare;
mod adapter_dispatch_run;
mod adapter_dogfood;
mod adapter_launch;
mod adapter_replay;
mod adapter_smoke;
mod cli_surface;
mod connectivity;
mod connectivity_evidence;
mod dashboard;
mod evidence;
mod runtime_target;
mod runtime_target_evidence;
mod tool_wrapper;
mod voice;
mod voice_render;
mod workpad;

use adapter_dispatch::{adapter_dispatch_evidence, adapter_dispatch_gate, adapter_dispatch_status};
use adapter_dispatch_prepare::{
    adapter_dispatch_execution_request, adapter_dispatch_materialize_prompt,
    adapter_dispatch_run_preflight,
};
use adapter_dispatch_run::adapter_dispatch_run_local;
use adapter_dogfood::{adapter_dogfood_gate, adapter_dogfood_gate_evidence};
use adapter_launch::{adapter_readiness, plan_adapter_launch};
use adapter_replay::{replay_adapter_dispatch_fixture, replay_adapter_fixture};
use adapter_smoke::{
    adapter_smoke_report_evidence, adapter_smoke_report_status, record_adapter_smoke_report,
    scan_adapter_smoke_artifacts,
};
use cli_surface::{HELP, ParsedArgs, has_flag, optional_arg, required_arg};
use connectivity::{
    activate_connectivity_exposure, connectivity_exposure_status, expose_connectivity_stub,
    request_connectivity_exposure_approval, revoke_connectivity_exposure,
};
use connectivity_evidence::connectivity_exposure_evidence;
use dashboard::dashboard;
use evidence::{export_evidence, export_task_outcome_report, record_review_finding};
use runtime_target::{
    list_runtime_targets, register_runtime_target, runtime_target_readiness, runtime_target_status,
    set_runtime_target_status,
};
use runtime_target_evidence::{runtime_target_evidence, runtime_target_readiness_evidence};
use tool_wrapper::run_wrapper_tool;
use voice::submit_voice;
use workpad::{
    apply_workpad_proposal, import_workpad_task, index_workpads, next_workpad_task,
    plan_next_workpad_task, propose_workpad_update, start_next_workpad_task,
};

const DEFAULT_PROJECT_ID: &str = "project-capo";

fn main() {
    match run_cli(env::args().skip(1).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

fn run_cli(raw_args: Vec<String>) -> Result<String, String> {
    let parsed = ParsedArgs::new(raw_args)?;
    let args = parsed.args.as_slice();

    match args {
        [] => Ok(HELP.to_string()),
        [flag] if flag == "--help" || flag == "-h" || flag == "help" => Ok(HELP.to_string()),
        [flag] if flag == "version" || flag == "--version" || flag == "-V" => {
            Ok(format!("capo {}\n", env!("CARGO_PKG_VERSION")))
        }
        [command] if command == "--help" || command == "-h" || command == "help" => {
            Ok(HELP.to_string())
        }
        [command] if command == "init" => init(&parsed),
        [command, rest @ ..] if command == "dashboard" => dashboard(&parsed, rest),
        [area, command, rest @ ..] if area == "agent" && command == "register" => {
            register_agent(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "agent" && command == "spawn" => {
            register_agent(&parsed, rest)
        }
        [area, command] if area == "agent" && command == "list" => list_agents(&parsed),
        [area, command, rest @ ..] if area == "adapter" && command == "replay-fixture" => {
            replay_adapter_fixture(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "replay-dispatch" => {
            replay_adapter_dispatch_fixture(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "dogfood" && command == "readiness" => {
            dogfood_readiness(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "readiness" => {
            adapter_readiness(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "plan-launch" => {
            plan_adapter_launch(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-gate" => {
            adapter_dispatch_gate(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-status" => {
            adapter_dispatch_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-evidence" => {
            adapter_dispatch_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "execution-request" => {
            adapter_dispatch_execution_request(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "materialize-prompt" => {
            adapter_dispatch_materialize_prompt(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "run-preflight" => {
            adapter_dispatch_run_preflight(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "run-local" => {
            adapter_dispatch_run_local(&parsed, rest)
        }
        [area, command] if area == "adapter" && command == "dogfood-gate" => {
            adapter_dogfood_gate(&parsed)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "dogfood-gate" && action == "evidence" =>
        {
            adapter_dogfood_gate_evidence(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "scan" =>
        {
            scan_adapter_smoke_artifacts(rest)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "record" =>
        {
            record_adapter_smoke_report(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "status" =>
        {
            adapter_smoke_report_status(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "evidence" =>
        {
            adapter_smoke_report_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "task" && command == "send" => {
            send_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "status" => {
            session_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "redirect" => {
            redirect_session(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "interrupt" => {
            interrupt_session(&parsed, rest, "interrupt")
        }
        [area, command, rest @ ..] if area == "session" && command == "stop" => {
            interrupt_session(&parsed, rest, "stop")
        }
        [area, command, rest @ ..] if area == "voice" && command == "submit" => {
            submit_voice(&parsed, rest)
        }
        [command] if command == "recover" => recover(&parsed),
        [area, command, rest @ ..] if area == "permission" && command == "request" => {
            request_permission_approval(&parsed, rest)
        }
        [area, command] if area == "permission" && command == "list" => {
            list_permission_approvals(&parsed)
        }
        [area, command, rest @ ..] if area == "permission" && command == "decide" => {
            decide_permission_approval(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "register" =>
        {
            register_runtime_target(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "set-status" =>
        {
            set_runtime_target_status(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "status" =>
        {
            runtime_target_status(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "readiness" =>
        {
            runtime_target_readiness(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "readiness-evidence" =>
        {
            runtime_target_readiness_evidence(&parsed, rest)
        }
        [area, command, action, rest @ ..]
            if area == "runtime" && command == "target" && action == "evidence" =>
        {
            runtime_target_evidence(&parsed, rest)
        }
        [area, command, action] if area == "runtime" && command == "target" && action == "list" => {
            list_runtime_targets(&parsed)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "expose-stub" => {
            expose_connectivity_stub(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "request-approval" => {
            request_connectivity_exposure_approval(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "activate-exposure" => {
            activate_connectivity_exposure(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "revoke-exposure" => {
            revoke_connectivity_exposure(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "exposure-status" => {
            connectivity_exposure_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "exposure-evidence" => {
            connectivity_exposure_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "index" => {
            index_workpads(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "next" => {
            next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "plan-next" => {
            plan_next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "start-next" => {
            start_next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "import" => {
            import_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "propose" => {
            propose_workpad_update(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "apply" => {
            apply_workpad_proposal(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "evidence" && command == "export" => {
            export_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "eval" && command == "task-outcome" => {
            export_task_outcome_report(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "review" && command == "record" => {
            record_review_finding(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "tool" && command == "run-wrapper" => {
            run_wrapper_tool(&parsed, rest)
        }
        [unknown, ..] => Err(format!("unknown command: {unknown}\nrun `capo --help`")),
    }
}

fn init(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "init",
        CommandTarget::Project(project_id()),
        CommandIntent::InitializeProject,
        None,
    );
    let initialized = controller(parsed)?
        .initialize(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "initialized=true\nstate={}\ncommand_id={}\n",
        initialized.state_db_path, initialized.command_id
    ))
}

fn register_agent(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    require_fake_arg(args, "--adapter")?;
    require_fake_arg(args, "--runtime")?;
    let name = required_arg(args, "--name")?;
    let spawn = parsed.args.get(1).is_some_and(|command| command == "spawn");
    let command = envelope(
        if spawn {
            "agent-spawn"
        } else {
            "agent-register"
        },
        CommandTarget::Project(project_id()),
        CommandIntent::RegisterAgent,
        Some(name.clone()),
    );
    let controller = controller(parsed)?;
    let registration = if spawn {
        controller
            .spawn_agent_command(&command)
            .map_err(debug_error)?
    } else {
        controller
            .register_agent_command(&command)
            .map_err(debug_error)?
    };
    Ok(format!(
        "{}=true\nagent={}\nagent_id={}\nspawn_semantics=registered_fake_agent_runtime_starts_on_task_send\ncommand_id={}\n",
        if spawn {
            "agent_spawned"
        } else {
            "agent_registered"
        },
        registration.agent_name,
        registration.agent_id,
        command.command_id
    ))
}

fn list_agents(parsed: &ParsedArgs) -> Result<String, String> {
    let state = state(parsed)?;
    let command = envelope(
        "agent-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let agents = state.agents().map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\nactive_agents={}\n",
        command.command_id,
        agents.len()
    );
    for agent in agents {
        output.push_str(&format!(
            "agent={} status={} current_session={}\n",
            agent.name,
            agent.status,
            agent
                .current_session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string())
        ));
    }
    Ok(output)
}

fn dogfood_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let out = optional_arg(args, "--out").map(PathBuf::from);
    if has_flag(args, "--out") && out.is_none() {
        return Err("--out requires a value".to_string());
    }
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--out"))
    {
        return Err(format!("unknown dogfood readiness option: {unknown}"));
    }
    let state = state(parsed)?;
    let project_id = project_id();
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
    let readiness = project_dogfood_readiness(&dashboard);
    let mut output = render_dogfood_readiness(&readiness);
    if let Some(out) = out {
        let command = envelope(
            "dogfood-readiness",
            CommandTarget::Project(project_id.clone()),
            CommandIntent::ExportEvidence,
            Some(readiness.status.clone()),
        );
        let markdown = render_dogfood_readiness_evidence(&project_id, &readiness);
        fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        let content_hash = stable_cli_hash(&markdown);
        let artifact_id = format!("artifact-dogfood-readiness-{content_hash}");
        let path = out.join(format!("{artifact_id}.md"));
        write_dogfood_readiness_file(&path, &markdown)?;
        state
            .record_artifact(ArtifactRecord {
                artifact_id: artifact_id.clone(),
                project_id: Some(project_id.clone()),
                session_id: None,
                run_id: None,
                kind: "dogfood_readiness".to_string(),
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
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(evidence_id.clone()),
                    payload_json: format!(
                        "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"ready\":{},\"status\":\"{}\"}}",
                        escape_json(&artifact_id),
                        escape_json(&content_hash),
                        readiness.ready,
                        escape_json(&readiness.status)
                    ),
                    idempotency_key: Some(format!(
                        "dogfood-readiness:{}:{content_hash}",
                        project_id
                    )),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                    project_id: project_id.clone(),
                    task_id: None,
                    session_id: None,
                    run_id: None,
                    kind: "dogfood_readiness".to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: dogfood_readiness_confidence(&readiness),
                    updated_sequence: 0,
                })],
            )
            .map_err(debug_error)?;
        output.push_str(&format!(
            "dogfood_readiness_evidence_exported=true\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
            path.display(),
            command.command_id
        ));
    }
    Ok(output)
}

fn render_dogfood_readiness(readiness: &ProjectDogfoodReadiness) -> String {
    format!(
        "dogfood_readiness=true\nready={}\nstatus={}\nreal_agent_connector_ready={}\nruntime_target_ready={}\nworkpad_bridge_ready={}\ndispatch_chain_ready={}\nruntime_targets={}\nruntime_targets_available={}\nworkpad_tasks={}\nworkpad_tasks_observed_only={}\nworkpad_tasks_imported={}\ndispatch_plans={}\nready_dispatch_gates={}\ndispatch_replays={}\ndispatch_executions={}\nconnector_evidence_refs={}\nruntime_target_refs={}\nworkpad_task_refs={}\ndispatch_chain_refs={}\nproject_evidence_refs={}\nblockers={}\nnext_actions={}\n",
        readiness.ready,
        readiness.status,
        readiness.real_agent_connector_ready,
        readiness.runtime_target_ready,
        readiness.workpad_bridge_ready,
        readiness.dispatch_chain_ready,
        readiness.runtime_target_count,
        readiness.available_runtime_target_count,
        readiness.workpad_task_count,
        readiness.observed_workpad_task_count,
        readiness.imported_workpad_task_count,
        readiness.dispatch_plan_count,
        readiness.ready_dispatch_gate_count,
        readiness.dispatch_replay_count,
        readiness.dispatch_execution_count,
        comma_or_none(&readiness.connector_evidence_refs),
        comma_or_none(&readiness.runtime_target_refs),
        comma_or_none(&readiness.workpad_task_refs),
        comma_or_none(&readiness.dispatch_chain_refs),
        comma_or_none(&readiness.project_evidence_refs),
        readiness.blockers.join(","),
        readiness.next_actions.join(",")
    )
}

fn dogfood_readiness_confidence(readiness: &ProjectDogfoodReadiness) -> i64 {
    if readiness.ready { 90 } else { 65 }
}

fn render_dogfood_readiness_evidence(
    project_id: &ProjectId,
    readiness: &ProjectDogfoodReadiness,
) -> String {
    format!(
        "<!-- capo:dogfood-readiness -->\n# Capo Dogfood Readiness - {}\n\n## Objective\n\nReview whether Capo is ready to move its own project workpads into Capo-managed dogfood.\n\n## Summary\n\n- Project: `{}`\n- Ready: `{}`\n- Status: `{}`\n- Real-agent connector ready: `{}`\n- Runtime target ready: `{}`\n- Workpad bridge ready: `{}`\n- Dispatch chain ready: `{}`\n\n## Counts\n\n- Runtime targets: `{}`\n- Available runtime targets: `{}`\n- Workpad tasks: `{}`\n- Observed-only workpad tasks: `{}`\n- Imported workpad tasks: `{}`\n- Dispatch plans: `{}`\n- Ready dispatch gates: `{}`\n- Dispatch replays: `{}`\n- Dispatch executions: `{}`\n\n## Component Refs\n\n- Connector evidence refs: `{}`\n- Runtime target refs: `{}`\n- Workpad task refs: `{}`\n- Dispatch chain refs: `{}`\n- Project evidence refs: `{}`\n\n## Blockers\n\n{}\n\n## Next Actions\n\n{}\n\n## Evidence Policy\n\n- This report is derived from persisted Capo read models only.\n- It does not run provider CLIs, inspect credentials, materialize prompts, open tunnels, launch runtimes, or edit markdown.\n- Raw prompts, raw provider output, credentials, cookies, and subscription session material are not rendered.\n",
        readiness.status,
        project_id,
        readiness.ready,
        readiness.status,
        readiness.real_agent_connector_ready,
        readiness.runtime_target_ready,
        readiness.workpad_bridge_ready,
        readiness.dispatch_chain_ready,
        readiness.runtime_target_count,
        readiness.available_runtime_target_count,
        readiness.workpad_task_count,
        readiness.observed_workpad_task_count,
        readiness.imported_workpad_task_count,
        readiness.dispatch_plan_count,
        readiness.ready_dispatch_gate_count,
        readiness.dispatch_replay_count,
        readiness.dispatch_execution_count,
        comma_or_none(&readiness.connector_evidence_refs),
        comma_or_none(&readiness.runtime_target_refs),
        comma_or_none(&readiness.workpad_task_refs),
        comma_or_none(&readiness.dispatch_chain_refs),
        comma_or_none(&readiness.project_evidence_refs),
        markdown_list_or_none(&readiness.blockers),
        markdown_list_or_none(&readiness.next_actions)
    )
}

fn send_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let scenario = optional_arg(args, "--scenario").unwrap_or_else(|| "default".to_string());
    let mut command = envelope(
        "task-send",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::SendTask,
        Some(goal.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    command
        .structured_args
        .push(("scenario".to_string(), scenario.clone()));
    let refs = controller(parsed)?
        .send_task_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "task_sent=true\nagent={agent}\nscenario={scenario}\ntask_id={}\nsession_id={}\nrun_id={}\ncommand_id={}\n",
        refs.task_id, refs.session_id, refs.run_id, command.command_id
    ))
}

fn session_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let command = envelope(
        "session-status",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::QueryStatus,
        None,
    );
    let observation = controller(parsed)?
        .observe_agent_name(&agent)
        .map_err(debug_error)?;
    let state = state(parsed)?;
    let evidence = state
        .evidence_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    let tool_calls = state
        .tool_calls_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    let tool_observations = state
        .tool_observations_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    Ok(render_status(
        &command,
        &observation,
        &evidence,
        &tool_calls,
        &tool_observations,
    ))
}

fn redirect_session(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let mut command = envelope(
        "redirect",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::RedirectSession,
        Some(goal.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    let observation = controller(parsed)?
        .redirect_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "redirected=true\nagent={agent}\nsession_id={}\nstatus={}\ncurrent_goal={}\nlatest_summary={}\ncommand_id={}\n",
        observation.session.session_id,
        observation.session.status,
        observation.session.current_goal,
        observation
            .session
            .latest_summary
            .unwrap_or_else(|| "none".to_string()),
        command.command_id
    ))
}

fn interrupt_session(parsed: &ParsedArgs, args: &[String], action: &str) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let reason = required_arg(args, "--reason")?;
    let mut command = envelope(
        action,
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::InterruptSession,
        Some(reason.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    let controller = controller(parsed)?;
    let observation = if action == "stop" {
        controller.stop_command(&command).map_err(debug_error)?
    } else {
        controller
            .interrupt_command(&command)
            .map_err(debug_error)?
    };
    Ok(format!(
        "{action}=true\nagent={agent}\nsession_id={}\nstatus={}\nrun_status={}\nlatest_summary={}\ncommand_id={}\n",
        observation.session.session_id,
        observation.session.status,
        observation.run.status,
        observation
            .session
            .latest_summary
            .unwrap_or_else(|| "none".to_string()),
        command.command_id
    ))
}

fn recover(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "recover",
        CommandTarget::Project(project_id()),
        CommandIntent::Recover,
        None,
    );
    let report = controller(parsed)?
        .recover_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "recovered=true\nrecovery_attempt_id={}\nstarted_sequence={}\ncompleted_sequence={}\nwatermark={}\nrecovered_run_count={}\ncommand_id={}\n",
        report.recovery_attempt_id,
        report.started_sequence,
        report.completed_sequence,
        report
            .watermark
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        report.recovered_run_count,
        command.command_id
    ))
}

fn request_permission_approval(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let scope_json = required_arg(args, "--scope-json")?;
    validate_scope_json(&scope_json)?;
    let subject_json = optional_arg(args, "--subject-json")
        .unwrap_or_else(|| "{\"actor\":\"local-user\"}".to_string());
    validate_json_object("subject-json", &subject_json)?;
    let reason = required_arg(args, "--reason")?;
    let capability_profile_id =
        optional_arg(args, "--profile").unwrap_or_else(|| "trusted-local-dev".to_string());
    let requested_by =
        optional_arg(args, "--requested-by").unwrap_or_else(|| "local-user".to_string());
    let project_id = project_id();
    let session_id = optional_arg(args, "--session").map(SessionId::new);
    let tool_call_id = optional_arg(args, "--tool-call").map(ToolCallId::new);
    let command = envelope(
        "permission-request",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::QueuePermissionApproval,
        Some(reason.clone()),
    );
    let state = state(parsed)?;
    if state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .is_some()
    {
        return Err(format!("approval already exists: {approval_id}"));
    }
    let approval = PermissionApprovalProjection {
        approval_id: approval_id.clone(),
        project_id: project_id.clone(),
        session_id,
        tool_call_id,
        capability_profile_id,
        scope_json,
        subject_json,
        status: "pending".to_string(),
        requested_by,
        reason,
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-approval-queued-{}",
            stable_cli_hash(&approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!("permission-approval-request:{approval_id}"));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_queued=true\napproval_id={}\nstatus=pending\nprofile={}\nsequence={sequence}\ncommand_id={}\n",
        approval.approval_id, approval.capability_profile_id, command.command_id
    ))
}

fn list_permission_approvals(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "permission-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let approvals = state(parsed)?
        .permission_approvals(&project_id())
        .map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\npermission_approvals={}\n",
        command.command_id,
        approvals.len()
    );
    for approval in approvals {
        output.push_str(&format!(
            "approval={} status={} profile={} session={} tool_call={} decision={} grant={} requested_by={} reason={}\n",
            approval.approval_id,
            approval.status,
            approval.capability_profile_id,
            approval
                .session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval
                .tool_call_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval.decision.as_deref().unwrap_or("none"),
            approval.capability_grant_id.as_deref().unwrap_or("none"),
            approval.requested_by,
            approval.reason
        ));
    }
    Ok(output)
}

fn decide_permission_approval(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let decision = required_arg(args, "--decision")?;
    let (effect, persistence) = approval_decision_effect(&decision)?;
    let project_id = project_id();
    let command = envelope(
        "permission-decide",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::DecidePermissionApproval,
        Some(decision.clone()),
    );
    let state = state(parsed)?;
    let approval = state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing approval: {approval_id}"))?;
    if approval.status != "pending" {
        return Err(format!(
            "approval is not pending: {approval_id} status={}",
            approval.status
        ));
    }
    validate_decision_scope(&decision, &approval)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = (decision != "reject_once").then(|| CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json: subject_json.clone(),
        decision_source: "user".to_string(),
        persistence: persistence.to_string(),
        explanation: format!("user approval decision {decision} for {approval_id}"),
        updated_sequence: 0,
    });
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.clone()),
        capability_grant_id: grant
            .as_ref()
            .map(|grant| grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-decided-{}",
            stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
        ),
        EventKind::PermissionDecided,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\"}}",
        escape_json(&approval_id),
        escape_json(&decision),
        escape_json(&grant_id),
        effect,
        persistence
    );
    event.idempotency_key = None;
    event.redaction_state = RedactionState::Safe;
    let grant_event = grant.as_ref().map(|grant| {
        let mut event = NewEvent::new(
            format!(
                "event-capability-grant-{}",
                stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
            ),
            EventKind::CapabilityGrantCreated,
            "capo-cli",
        );
        event.project_id = Some(project_id.clone());
        event.session_id = approval.session_id.clone();
        event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
        event.payload_json = format!(
            "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\"}}",
            escape_json(&approval_id),
            escape_json(&grant.capability_grant_id),
            escape_json(&grant.effect),
            escape_json(&grant.decision_source),
            escape_json(&grant.persistence)
        );
        event.idempotency_key = None;
        event.redaction_state = RedactionState::Safe;
        event
    });
    let sequence = state
        .decide_permission_approval(&approval_id, event, grant_event, decided_approval, grant)
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_decided=true\napproval_id={approval_id}\ndecision={decision}\neffect={effect}\npersistence={persistence}\ncapability_grant_id={}\nsequence={sequence}\ncommand_id={}\n",
        if decision == "reject_once" {
            "none"
        } else {
            &grant_id
        },
        command.command_id
    ))
}

pub(crate) fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub(crate) fn stable_cli_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn render_status(
    command: &CommandEnvelope,
    observation: &capo_controller::FakeReadModelObservation,
    evidence: &[EvidenceProjection],
    tool_calls: &[ToolCallProjection],
    tool_observations: &[ToolObservationProjection],
) -> String {
    let mut output = format!(
        "command_id={}\nagent={} agent_status={}\nsession_id={} session_status={}\nrun_id={} run_status={}\ncurrent_goal={}\nlatest_summary={}\nconfidence={}\nblocker={}\nevidence_refs={}\ntool_calls={}\ntool_observations={}\nrecent_events={}\n",
        command.command_id,
        observation.agent.name,
        observation.agent.status,
        observation.session.session_id,
        observation.session.status,
        observation.run.run_id,
        observation.run.status,
        observation.session.current_goal,
        observation
            .session
            .latest_summary
            .as_deref()
            .unwrap_or("none"),
        observation
            .session
            .latest_confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        observation
            .session
            .latest_blocker
            .as_deref()
            .unwrap_or("none"),
        evidence
            .iter()
            .map(|item| item.evidence_id.to_string())
            .collect::<Vec<_>>()
            .join(","),
        tool_calls.len(),
        tool_observations.len(),
        observation.recent_events.len()
    );
    for tool_call in tool_calls {
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
    for observation in tool_observations {
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
    for event in &observation.recent_events {
        output.push_str(&format!("event={} kind={}\n", event.sequence, event.kind));
    }
    output
}

fn write_dogfood_readiness_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:dogfood-readiness -->") {
            return Err(format!(
                "refusing to overwrite non-Capo dogfood readiness file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo dogfood readiness file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

pub(crate) fn controller(parsed: &ParsedArgs) -> Result<FakeBoundaryController, String> {
    FakeBoundaryController::open(project_id(), &parsed.state_root).map_err(debug_error)
}

pub(crate) fn state(parsed: &ParsedArgs) -> Result<SqliteStateStore, String> {
    SqliteStateStore::open(&parsed.state_root).map_err(debug_error)
}

pub(crate) fn envelope(
    command_slug: &str,
    target: CommandTarget,
    intent: CommandIntent,
    text: Option<String>,
) -> CommandEnvelope {
    let mut command = CommandEnvelope::new(
        CommandId::new(format!("cmd-{command_slug}")),
        InputOrigin::Cli,
        "local-user",
        project_id(),
        target,
        intent,
    );
    if let Some(text) = text {
        command = command.with_text(text);
    }
    command
}

pub(crate) fn project_id() -> ProjectId {
    ProjectId::new(DEFAULT_PROJECT_ID)
}

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_arg(args, key).as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in P4, got `{other}`")),
    }
}

fn validate_scope_json(scope_json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values))
            if values.iter().all(|value| value.as_str().is_some()) =>
        {
            Ok(())
        }
        Ok(_) => Err("--scope-json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("--scope-json is not valid JSON: {error}")),
    }
}

fn validate_json_object(label: &str, json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(format!("--{label} must be a JSON object")),
        Err(error) => Err(format!("--{label} is not valid JSON: {error}")),
    }
}

pub(crate) fn approval_decision_effect(
    decision: &str,
) -> Result<(&'static str, &'static str), String> {
    match decision {
        "allow_once" => Ok(("allow", "once")),
        "allow_always" => Ok(("allow", "until_revoked")),
        "reject_once" => Ok(("deny", "once")),
        "reject_always" => Ok(("deny", "until_revoked")),
        other => Err(format!(
            "unknown approval decision: {other}; expected allow_once, allow_always, reject_once, or reject_always"
        )),
    }
}

fn validate_decision_scope(
    decision: &str,
    approval: &PermissionApprovalProjection,
) -> Result<(), String> {
    if decision != "allow_always" {
        return Ok(());
    }
    let scopes = scope_values(&approval.scope_json)?;
    let durable_allowed = scopes.iter().all(|scope| {
        matches!(
            scope.as_str(),
            "tool:invoke:capo.task_status"
                | "tool:invoke:capo.agent_status"
                | "tool:invoke:capo.session_summary"
                | "tool:invoke:capo.workpad_read"
        ) || scope.starts_with("state:read:")
    });
    if durable_allowed {
        Ok(())
    } else {
        Err(
            "allow_always is restricted to Capo-owned read/status scopes in the PT2 CLI path"
                .to_string(),
        )
    }
}

pub(crate) fn approval_subject_json(
    approval: &PermissionApprovalProjection,
) -> Result<String, String> {
    let mut subject = match serde_json::from_str::<serde_json::Value>(&approval.subject_json) {
        Ok(serde_json::Value::Object(subject)) => subject,
        Ok(_) => return Err("approval subject_json must be a JSON object".to_string()),
        Err(error) => return Err(format!("approval subject_json is not valid JSON: {error}")),
    };
    subject.insert(
        "approval_id".to_string(),
        serde_json::Value::String(approval.approval_id.clone()),
    );
    subject.insert(
        "persistence_scope".to_string(),
        serde_json::Value::String("permission_approval".to_string()),
    );
    if let Some(session_id) = &approval.session_id {
        subject.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
    }
    if let Some(tool_call_id) = &approval.tool_call_id {
        subject.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(tool_call_id.to_string()),
        );
    }
    Ok(serde_json::Value::Object(subject).to_string())
}

pub(crate) fn scope_values(scope_json: &str) -> Result<Vec<String>, String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values)) => values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| "scope_json must contain only strings".to_string())
            })
            .collect(),
        Ok(_) => Err("scope_json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("scope_json is not valid JSON: {error}")),
    }
}

pub(crate) fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

pub(crate) fn comma_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(",")
    }
}

fn markdown_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- none".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- `{item}`"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests;
