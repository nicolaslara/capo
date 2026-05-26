use std::env;
use std::path::Path;

use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
};
use capo_state::{
    EvidenceProjection, SqliteStateStore, ToolCallProjection, ToolObservationProjection,
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
mod dogfood;
mod evidence;
mod permission;
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
use adapter_launch::{adapter_readiness, plan_adapter_launch, plan_adapter_proof};
use adapter_replay::{replay_adapter_dispatch_fixture, replay_adapter_fixture};
use adapter_smoke::{
    adapter_smoke_report_evidence, adapter_smoke_report_status, record_adapter_smoke_report,
    scan_adapter_smoke_artifacts,
};
use cli_surface::{HELP, ParsedArgs, optional_arg, required_arg};
use connectivity::{
    activate_connectivity_exposure, connectivity_exposure_status, expose_connectivity_stub,
    request_connectivity_exposure_approval, revoke_connectivity_exposure,
};
use connectivity_evidence::connectivity_exposure_evidence;
use dashboard::dashboard;
use dogfood::dogfood_readiness;
use evidence::{export_evidence, export_task_outcome_report, record_review_finding};
use permission::{
    decide_permission_approval, list_permission_approvals, request_permission_approval,
};
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
        [area, command, rest @ ..] if area == "adapter" && command == "plan-proof" => {
            plan_adapter_proof(&parsed, rest)
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

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests;
