use std::env;

use capo_controller::FakeBoundaryController;
use capo_core::{CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId};
use capo_state::SqliteStateStore;

mod adapter_dispatch;
mod adapter_dispatch_prepare;
mod adapter_dispatch_run;
mod adapter_dogfood;
mod adapter_launch;
mod adapter_replay;
mod adapter_smoke;
mod agent_session;
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
use agent_session::{
    init, interrupt_session, list_agents, recover, redirect_session, register_agent, send_task,
    session_status,
};
use cli_surface::{HELP, ParsedArgs};
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

#[cfg(test)]
mod tests;
