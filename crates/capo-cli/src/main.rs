use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use capo_adapters::{
    AcpAdapter, AdapterFixtureParse, ClaudeCodeAdapter, CodexExecAdapter, LocalAdapterSmokePlan,
    NormalizedAdapterEvent,
};
use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    RunId, SessionId, TaskId, ToolCallId,
};
use capo_eval::TaskOutcomeReport;
use capo_query::{
    AdapterDispatchStatus, ProjectDashboard, ProjectDashboardQuery, ProjectDogfoodReadiness,
    RuntimeTargetControlReadiness, project_dashboard, project_dogfood_readiness,
};
use capo_state::{
    AdapterDispatchPlanProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    AgentProjection, ArtifactRecord, CapabilityGrantProjection, ConnectivityExposureProjection,
    EventKind, EventRecord, EvidenceProjection, MemoryPacketProjection, MemoryRecordProjection,
    MemorySourceProjection, NewEvent, PermissionApprovalProjection, ProjectionRecord,
    RedactionState, ReviewFindingProjection, RunProjection, RuntimeTargetProjection,
    SessionProjection, SqliteStateStore, ToolCallProjection, ToolObservationProjection,
    WorkpadFileProjection, WorkpadIndexResetProjection, WorkpadTaskProjection,
};
use capo_tools::{
    PermissionPolicy, RuntimeToolConfig, RuntimeToolWrappers, WrapperArtifact, WrapperToolRequest,
};
use capo_voice::{
    MemoryIngestionPolicy, TranscriptRetentionPolicy, VOICE_TRANSCRIPT_RETENTION_DEFAULT,
    VoiceCommandPlan, VoiceIntentKind, VoiceReadScope, VoiceTranscriptInput, plan_dummy_transcript,
};
use capo_workpads::{WorkpadIndex, index_project_workpads};

mod adapter_dispatch;
mod adapter_dispatch_prepare;
mod adapter_dispatch_run;
mod adapter_dogfood;
mod adapter_smoke;
mod cli_surface;
mod connectivity;
mod connectivity_evidence;
mod runtime_target;
mod runtime_target_evidence;

use adapter_dispatch::{adapter_dispatch_evidence, adapter_dispatch_gate, adapter_dispatch_status};
use adapter_dispatch_prepare::{
    adapter_dispatch_execution_request, adapter_dispatch_materialize_prompt,
    adapter_dispatch_run_preflight,
};
use adapter_dispatch_run::adapter_dispatch_run_local;
use adapter_dogfood::{
    adapter_dogfood_gate, adapter_dogfood_gate_evidence, render_adapter_dogfood_gate,
};
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
use runtime_target::{
    list_runtime_targets, register_runtime_target, render_runtime_target_control_readiness,
    render_runtime_target_row, runtime_target_readiness, runtime_target_status,
    set_runtime_target_status,
};
use runtime_target_evidence::{runtime_target_evidence, runtime_target_readiness_evidence};

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

fn run_wrapper_tool(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--tool"
                    | "--workspace"
                    | "--artifacts"
                    | "--policy"
                    | "--path"
                    | "--content"
                    | "--message"
                    | "--program"
                    | "--argv-json"
                    | "--cwd"
                    | "--record"
            )
    }) {
        return Err(format!("unknown tool run-wrapper option: {unknown}"));
    }
    let record = has_flag(args, "--record");
    let tool_id = normalize_wrapper_tool_id(&required_arg(args, "--tool")?)?;
    let workspace = PathBuf::from(required_arg(args, "--workspace")?);
    let artifacts = PathBuf::from(required_arg(args, "--artifacts")?);
    let policy = wrapper_tool_policy(optional_arg(args, "--policy").as_deref())?;
    let input = wrapper_tool_input(&tool_id, args)?;
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts.clone(),
    ));
    let request_hash = stable_cli_hash(&format!("{tool_id}:{input}:{workspace:?}:{artifacts:?}"));
    let request = WrapperToolRequest {
        tool_call_id: ToolCallId::new(format!("cli-wrapper-{request_hash}")),
        session_id: SessionId::new(format!("session-cli-wrapper-{request_hash}")),
        run_id: RunId::new(format!("cli-wrapper-run-{request_hash}")),
        tool_id,
        capability_profile_id: policy.default_profile_id().to_string(),
        input,
    };
    let session_id = request.session_id.clone();
    let run_id = request.run_id.clone();
    let result = wrappers.authorize_and_invoke(request, &policy);
    let recorded_sequence = if record {
        Some(record_wrapper_tool_result(
            parsed,
            &session_id,
            &run_id,
            &result,
        )?)
    } else {
        None
    };
    let mut output = format!(
        "wrapper_tool_run=true\ntool={}\ntool_call={}\nsession_id={}\nrun_id={}\npolicy={}\nstatus={}\npermission_effect={}\npermission_source={}\nrecorded={}\nrecorded_sequence={}\ninput_artifact={}\noutput_artifacts={}\n",
        result.tool_id,
        result.tool_call_id,
        session_id,
        run_id,
        result.permission_decision.capability_profile_id,
        result.status,
        result.permission_decision.effect,
        result.permission_decision.decision_source,
        recorded_sequence.is_some(),
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string()),
        result
            .input_artifact
            .as_ref()
            .map(|artifact| artifact.artifact_id.as_str())
            .unwrap_or("none"),
        result.output_artifacts.len()
    );
    if let Some(input_artifact) = &result.input_artifact {
        output.push_str(&render_wrapper_artifact("input", input_artifact));
    }
    for artifact in &result.output_artifacts {
        output.push_str(&render_wrapper_artifact("output", artifact));
    }
    output.push_str(&format!("audit_events={}\n", result.events.len()));
    for event in &result.events {
        output.push_str(&format!(
            "audit_event={} status={}\n",
            event.kind, event.status
        ));
    }
    output.push_str(&format!("summary={}\n", result.summary));
    Ok(output)
}

fn record_wrapper_tool_result(
    parsed: &ParsedArgs,
    session_id: &SessionId,
    run_id: &RunId,
    result: &capo_tools::WrapperToolResult,
) -> Result<i64, String> {
    let project_id = project_id();
    let agent_id = AgentId::new("agent-cli-wrapper");
    let state = state(parsed)?;
    for artifact in result
        .input_artifact
        .iter()
        .chain(result.output_artifacts.iter())
    {
        state
            .record_artifact(wrapper_artifact_record(
                artifact,
                &project_id,
                session_id,
                run_id,
            )?)
            .map_err(debug_error)?;
    }
    let output_artifact_id = result
        .output_artifacts
        .first()
        .map(|artifact| artifact.artifact_id.clone());
    let event_id = format!(
        "event-wrapper-tool-recorded-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}",
            result.tool_call_id, session_id, result.status
        ))
    );
    let mut event = NewEvent::new(event_id, EventKind::ToolCallCompleted, "capo-cli");
    event.project_id = Some(project_id.clone());
    event.agent_id = Some(agent_id.clone());
    event.session_id = Some(session_id.clone());
    event.run_id = Some(run_id.clone());
    event.item_id = Some(result.tool_call_id.to_string());
    event.payload_json = format!(
        "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"status\":\"{}\",\"permission_effect\":\"{}\",\"permission_source\":\"{}\",\"recorded_from\":\"tool.run_wrapper\"}}",
        escape_json(result.tool_call_id.as_str()),
        escape_json(&result.tool_id),
        escape_json(&result.status),
        escape_json(&result.permission_decision.effect),
        escape_json(&result.permission_decision.decision_source)
    );
    event.idempotency_key = Some(format!("wrapper-tool-record:{}", result.tool_call_id));
    event.redaction_state = RedactionState::Safe;
    state
        .append_event(
            event,
            &[
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: agent_id.clone(),
                    project_id: project_id.clone(),
                    name: "cli-wrapper".to_string(),
                    status: "active".to_string(),
                    current_session_id: Some(session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id,
                    task_id: None,
                    agent_id,
                    title: format!("CLI wrapper {}", result.tool_id),
                    status: "completed".to_string(),
                    current_goal: format!("Run governed wrapper {}", result.tool_id),
                    latest_summary: Some(result.summary.clone()),
                    latest_confidence: Some(if result.status == "denied" { 40 } else { 80 }),
                    latest_blocker: (result.status == "denied").then(|| result.summary.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: result.status.clone(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: result.tool_call_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: Some("cli-wrapper".to_string()),
                    tool_name: result.tool_id.clone(),
                    tool_origin: "capo_wrapper".to_string(),
                    status: result.status.clone(),
                    input_artifact_id: result
                        .input_artifact
                        .as_ref()
                        .map(|artifact| artifact.artifact_id.clone()),
                    output_artifact_id,
                    updated_sequence: 0,
                }),
            ],
        )
        .map_err(debug_error)
}

fn wrapper_artifact_record(
    artifact: &WrapperArtifact,
    project_id: &ProjectId,
    session_id: &SessionId,
    run_id: &RunId,
) -> Result<ArtifactRecord, String> {
    Ok(ArtifactRecord {
        artifact_id: artifact.artifact_id.clone(),
        project_id: Some(project_id.clone()),
        session_id: Some(session_id.clone()),
        run_id: Some(run_id.clone()),
        kind: artifact.kind.clone(),
        uri: artifact.uri.clone(),
        content_hash: artifact.content_hash.clone(),
        size_bytes: artifact.size_bytes,
        redaction_state: wrapper_redaction_state(&artifact.redaction_state)?,
    })
}

fn wrapper_redaction_state(value: &str) -> Result<RedactionState, String> {
    match value {
        "safe" => Ok(RedactionState::Safe),
        "redacted" => Ok(RedactionState::Redacted),
        other => Err(format!(
            "wrapper artifact redaction state is not persistable: {other}"
        )),
    }
}

fn normalize_wrapper_tool_id(tool: &str) -> Result<String, String> {
    let normalized = match tool {
        "shell_run" | "shell-run" => "capo.shell_run",
        "git_status" | "git-status" => "capo.git_status",
        "git_diff" | "git-diff" => "capo.git_diff",
        "git_commit" | "git-commit" => "capo.git_commit",
        "file_read" | "file-read" => "capo.file_read",
        "file_write" | "file-write" => "capo.file_write",
        "workpad_read" | "workpad-read" => "capo.workpad_read",
        other if other.starts_with("capo.") => other,
        other => {
            return Err(format!(
                "unknown wrapper tool: {other}; expected shell_run, git_status, git_diff, git_commit, file_read, file_write, or workpad_read"
            ));
        }
    };
    Ok(normalized.to_string())
}

fn wrapper_tool_policy(policy: Option<&str>) -> Result<PermissionPolicy, String> {
    match policy.unwrap_or("read-only") {
        "read-only" | "read_only" => Ok(PermissionPolicy::static_read_only_local()),
        "reviewer" => Ok(PermissionPolicy::static_reviewer()),
        "trusted-local" | "trusted_local" => Ok(PermissionPolicy::allow_trusted_local()),
        other => Err(format!(
            "unknown wrapper policy: {other}; expected read-only, reviewer, or trusted-local"
        )),
    }
}

fn wrapper_tool_input(tool_id: &str, args: &[String]) -> Result<serde_json::Value, String> {
    match tool_id {
        "capo.shell_run" => {
            let program = required_arg(args, "--program")?;
            let argv = optional_arg(args, "--argv-json")
                .map(|json| parse_json_array("--argv-json", &json))
                .transpose()?
                .unwrap_or_else(|| serde_json::json!([]));
            let mut input = serde_json::json!({
                "program": program,
                "argv": argv,
            });
            if let Some(cwd) = optional_arg(args, "--cwd") {
                input["cwd"] = serde_json::Value::String(cwd);
            }
            Ok(input)
        }
        "capo.git_status" | "capo.git_diff" => {
            if let Some(path) = optional_arg(args, "--path") {
                Ok(serde_json::json!({ "path": path }))
            } else {
                Ok(serde_json::json!({}))
            }
        }
        "capo.git_commit" => Ok(serde_json::json!({
            "message": required_arg(args, "--message")?
        })),
        "capo.file_read" | "capo.workpad_read" => Ok(serde_json::json!({
            "path": required_arg(args, "--path")?
        })),
        "capo.file_write" => Ok(serde_json::json!({
            "path": required_arg(args, "--path")?,
            "content": required_arg(args, "--content")?
        })),
        other => Err(format!("unsupported wrapper tool: {other}")),
    }
}

fn parse_json_array(label: &str, json: &str) -> Result<serde_json::Value, String> {
    let value = serde_json::from_str::<serde_json::Value>(json)
        .map_err(|error| format!("{label} is not valid JSON: {error}"))?;
    if value.is_array() {
        Ok(value)
    } else {
        Err(format!("{label} must be a JSON array"))
    }
}

fn render_wrapper_artifact(label: &str, artifact: &WrapperArtifact) -> String {
    format!(
        "{label}_artifact={} kind={} uri={} hash={} bytes={} redaction={} summary={}\n",
        artifact.artifact_id,
        artifact.kind,
        artifact.uri,
        artifact.content_hash,
        artifact.size_bytes,
        artifact.redaction_state,
        artifact.summary
    )
}

fn replay_adapter_fixture(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let adapter = required_arg(args, "--adapter")?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let fixture = fs::read_to_string(&fixture_path).map_err(|error| error.to_string())?;
    let adapter_events = parse_adapter_fixture(&adapter, &fixture)?;
    let controller = controller(parsed)?;
    let registration = if state(parsed)?
        .agent_by_name(&agent)
        .map_err(debug_error)?
        .is_some()
    {
        controller
            .registration_for_agent_name(&agent)
            .map_err(debug_error)?
    } else {
        controller.register_agent(&agent).map_err(debug_error)?
    };
    let refs = controller
        .send_task(&registration, &goal)
        .map_err(debug_error)?;
    let report = controller
        .apply_normalized_adapter_events(&refs, &adapter_events)
        .map_err(debug_error)?;
    let mut output = format!(
        "adapter_replayed=true\nadapter={}\nfixture={}\nagent={}\nsession_id={}\nrun_id={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\n",
        adapter_label(&adapter),
        fixture_path.display(),
        agent,
        refs.session_id,
        refs.run_id,
        report.input_event_count,
        report.appended_event_count,
        report.tool_event_count,
        report.summary_event_count,
        report.completed_turn_count
    );
    if let Some(out) = optional_arg(args, "--out") {
        output.push_str(&export_evidence(
            parsed,
            &[
                "--session".to_string(),
                refs.session_id.to_string(),
                "--out".to_string(),
                out,
            ],
        )?);
    }
    Ok(output)
}

fn replay_adapter_dispatch_fixture(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--fixture" | "--out")
    }) {
        return Err(format!("unknown adapter replay-dispatch option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let ready_gate = dashboard.adapter_dispatch_gates.iter().rev().find(|gate| {
        gate.dispatch_plan_id == plan.dispatch_plan_id
            && gate.provider_cli_execution_allowed
            && gate.status == "ready_for_execution"
            && !gate.provider_cli_executed
            && gate.runtime_prompt_policy == "not_rendered"
    });
    let ready_gate = ready_gate.ok_or_else(|| {
        format!(
            "dispatch plan {} has no recorded ready dispatch gate; run adapter dispatch-gate --record after clean smoke evidence",
            plan.dispatch_plan_id
        )
    })?;
    let fixture = fs::read_to_string(&fixture_path).map_err(|error| error.to_string())?;
    let adapter_events = parse_adapter_fixture(&plan.adapter_kind, &fixture)?;
    let controller = controller(parsed)?;
    let registration = controller
        .registration_for_agent_name(&plan.agent_name)
        .map_err(debug_error)?;
    let replay_goal = format!(
        "Replay fixture for dispatch plan {} without provider execution",
        plan.dispatch_plan_id
    );
    let refs = controller
        .send_task(&registration, &replay_goal)
        .map_err(debug_error)?;
    if refs.session_id != plan.session_id || refs.run_id != plan.run_id {
        return Err(format!(
            "dispatch replay ref mismatch for {}: expected session={} run={}, got session={} run={}",
            plan.dispatch_plan_id, plan.session_id, plan.run_id, refs.session_id, refs.run_id
        ));
    }
    let report = controller
        .apply_normalized_adapter_events(&refs, &adapter_events)
        .map_err(debug_error)?;
    let replay = AdapterDispatchReplayProjection {
        dispatch_replay_id: format!(
            "adapter-dispatch-replay-{}",
            stable_cli_hash(&format!(
                "{}:{}:{}",
                plan.dispatch_plan_id,
                ready_gate.dispatch_gate_id,
                stable_cli_hash(&fixture)
            ))
        ),
        project_id: project_id(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        dispatch_gate_id: ready_gate.dispatch_gate_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        fixture_path: fixture_path.display().to_string(),
        fixture_hash: stable_cli_hash(&fixture),
        input_event_count: report.input_event_count as i64,
        appended_event_count: report.appended_event_count as i64,
        tool_event_count: report.tool_event_count as i64,
        summary_event_count: report.summary_event_count as i64,
        completed_turn_count: report.completed_turn_count as i64,
        provider_cli_executed: false,
        raw_content_policy: "content_hashed_not_rendered".to_string(),
        updated_sequence: 0,
    };
    let replay_sequence = state
        .append_event(
            NewEvent {
                event_id: format!(
                    "event-adapter-dispatch-replay-{}",
                    stable_cli_hash(&replay.dispatch_replay_id)
                ),
                kind: EventKind::AdapterDispatchReplayed,
                actor: "local-cli".to_string(),
                project_id: Some(replay.project_id.clone()),
                task_id: Some(refs.task_id.clone()),
                agent_id: Some(refs.agent_id.clone()),
                session_id: Some(refs.session_id.clone()),
                run_id: Some(refs.run_id.clone()),
                turn_id: None,
                item_id: Some(replay.dispatch_replay_id.clone()),
                payload_json: format!(
                    "{{\"dispatch_plan_id\":\"{}\",\"dispatch_gate_id\":\"{}\",\"fixture_hash\":\"{}\",\"provider_cli_executed\":false,\"raw_content_policy\":\"content_hashed_not_rendered\"}}",
                    escape_json(&replay.dispatch_plan_id),
                    escape_json(&replay.dispatch_gate_id),
                    replay.fixture_hash
                ),
                idempotency_key: Some(format!(
                    "adapter-dispatch-replay:{}:{}:{}",
                    replay.project_id, replay.dispatch_plan_id, replay.fixture_hash
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchReplay(replay.clone())],
        )
        .map_err(debug_error)?;
    let mut output = format!(
        "adapter_dispatch_replayed=true\ndispatch_replay={}\ndispatch_plan={}\ndispatch_gate={}\nadapter={}\nfixture={}\nfixture_hash={}\nagent={}\nsession_id={}\nrun_id={}\nprovider_cli_executed=false\nraw_content_policy={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\nrecorded_sequence={}\n",
        replay.dispatch_replay_id,
        plan.dispatch_plan_id,
        ready_gate.dispatch_gate_id,
        plan.adapter_kind,
        fixture_path.display(),
        replay.fixture_hash,
        plan.agent_name,
        refs.session_id,
        refs.run_id,
        replay.raw_content_policy,
        report.input_event_count,
        report.appended_event_count,
        report.tool_event_count,
        report.summary_event_count,
        report.completed_turn_count,
        replay_sequence
    );
    if let Some(out) = optional_arg(args, "--out") {
        output.push_str(&export_evidence(
            parsed,
            &[
                "--session".to_string(),
                refs.session_id.to_string(),
                "--out".to_string(),
                out,
            ],
        )?);
    }
    Ok(output)
}

fn parse_adapter_fixture(
    adapter: &str,
    fixture: &str,
) -> Result<Vec<NormalizedAdapterEvent>, String> {
    let parsed: AdapterFixtureParse = match adapter {
        "codex" | "codex-exec" | "codex_exec" => {
            CodexExecAdapter::parse_jsonl(fixture).map_err(adapter_parse_error)?
        }
        "claude" | "claude-code" | "claude_code" => {
            ClaudeCodeAdapter::parse_stream_json(fixture).map_err(adapter_parse_error)?
        }
        "acp" => AcpAdapter::parse_replay_jsonl(fixture).map_err(adapter_parse_error)?,
        other => {
            return Err(format!(
                "unsupported adapter fixture kind: {other}; expected codex, claude, or acp"
            ));
        }
    };
    Ok(parsed.deduped_by_idempotency())
}

fn adapter_parse_error(error: capo_adapters::AdapterParseError) -> String {
    format!(
        "adapter fixture parse failed at line {}: {}",
        error.line, error.message
    )
}

fn adapter_label(adapter: &str) -> &'static str {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" => "codex_exec",
        "claude" | "claude-code" | "claude_code" => "claude_code",
        "acp" => "acp",
        _ => "unknown",
    }
}

fn adapter_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

fn plan_adapter_launch(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
struct RecordedAdapterDispatchPlan {
    projection: AdapterDispatchPlanProjection,
    prompt_source: AdapterDispatchPromptSourceProjection,
    runtime_safe_arg_count: usize,
    subscription_safe: bool,
    recorded: bool,
    recorded_sequence: Option<i64>,
}

struct DispatchPlanRecordRequest<'a> {
    adapter: &'a str,
    agent: &'a str,
    goal: &'a str,
    workspace: PathBuf,
    artifacts: PathBuf,
    prompt_source: DispatchPromptSourceInput,
    record: bool,
}

fn recordable_adapter_dispatch_plan(
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

fn render_adapter_dispatch_plan(plan: &RecordedAdapterDispatchPlan) -> String {
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
struct DispatchPromptSourceInput {
    source_kind: String,
    source_ref: Option<String>,
    source_hash: Option<String>,
    materialization_status: String,
}

impl DispatchPromptSourceInput {
    fn inline_cli_prompt() -> Self {
        Self {
            source_kind: "inline_cli_prompt".to_string(),
            source_ref: None,
            source_hash: None,
            materialization_status: "manual_prompt_not_replayable".to_string(),
        }
    }

    fn workpad_task(task: &WorkpadTaskProjection, source_hash: String) -> Self {
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

fn validate_local_launch_adapter(adapter: &str) -> Result<(), String> {
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

fn dashboard(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

fn submit_voice(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let transcript = required_arg(args, "--transcript")?;
    let voice_session_id =
        optional_arg(args, "--voice-session").unwrap_or_else(|| "voice-session-cli".to_string());
    let actor_id = optional_arg(args, "--actor").unwrap_or_else(|| "local-user".to_string());
    let confirmed = has_flag(args, "--confirm");
    let redacted_summary = optional_arg(args, "--redacted-summary");
    let retention_policy = if redacted_summary.is_some() {
        TranscriptRetentionPolicy::RetainRedactedSummary
    } else {
        VOICE_TRANSCRIPT_RETENTION_DEFAULT
    };
    if redacted_summary.is_some() && !has_flag(args, "--reviewed-summary") {
        return Err(
            "--redacted-summary requires --reviewed-summary before memory ingestion".to_string(),
        );
    }
    let plan = plan_dummy_transcript(VoiceTranscriptInput {
        voice_session_id,
        actor_id,
        project_id: project_id(),
        transcript_text: transcript,
        asr_confidence: None,
        retention_policy,
    });

    let retained_memory = if let Some(summary) = redacted_summary {
        Some(ingest_reviewed_voice_summary(parsed, &plan, &summary)?)
    } else {
        None
    };

    if plan.intent_kind == VoiceIntentKind::Unknown {
        let mut output = render_voice_header(&plan, None, false, false);
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }
    if plan.requires_visible_confirmation && !confirmed {
        let approval = queue_voice_permission_approval(parsed, &plan)?;
        let mut output = render_voice_header(&plan, plan.command.as_ref(), true, false);
        output.push_str(&render_voice_approval(&approval, None));
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }

    let mut output = render_voice_header(&plan, plan.command.as_ref(), false, false);
    match plan.intent_kind {
        VoiceIntentKind::AdapterSmokeStatus
        | VoiceIntentKind::ConnectivityStatus
        | VoiceIntentKind::DashboardSummary
        | VoiceIntentKind::DispatchStatus
        | VoiceIntentKind::DogfoodReadiness
        | VoiceIntentKind::NextWork
        | VoiceIntentKind::RecentWork
        | VoiceIntentKind::ReviewNeeds
        | VoiceIntentKind::RuntimeTargetReadiness
        | VoiceIntentKind::RuntimeTargetStatus
        | VoiceIntentKind::ToolActivity
        | VoiceIntentKind::AgentStatus => {
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::RedirectSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice redirect plan missing command".to_string())?;
            controller(parsed)?
                .redirect_command(command)
                .map_err(debug_error)?;
            output = render_voice_header(&plan, Some(command), false, true);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::StartNextWork => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice start-next plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let agent = structured_arg_value(command, "agent")
                .ok_or_else(|| "voice start-next plan missing agent".to_string())?
                .to_string();
            let started = start_next_workpad_task(parsed, &["--agent".to_string(), agent.clone()])?;
            output = render_voice_header(&plan, Some(command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!("controlled_agent={agent}\n"));
            output.push_str(&started);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::InterruptSession | VoiceIntentKind::StopSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice session-control plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let durable_reason = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => "voice interrupt confirmed",
                VoiceIntentKind::StopSession => "voice stop confirmed",
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            let command = CommandEnvelope {
                text: Some(durable_reason.to_string()),
                ..command.clone()
            };
            let controller = controller(parsed)?;
            let observation = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => controller
                    .interrupt_command(&command)
                    .map_err(debug_error)?,
                VoiceIntentKind::StopSession => {
                    controller.stop_command(&command).map_err(debug_error)?
                }
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            output = render_voice_header(&plan, Some(&command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!(
                "controlled_session={} session_status={} run_status={}\n",
                observation.session.session_id, observation.session.status, observation.run.status
            ));
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::Unknown => {}
    }
    if let Some(record) = &retained_memory {
        output.push_str(&render_voice_memory_retention(record));
    }
    Ok(output)
}

fn ingest_reviewed_voice_summary(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    redacted_summary: &str,
) -> Result<MemoryRecordProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "reviewed voice summary requires a planned command".to_string())?;
    if redacted_summary.trim().is_empty() {
        return Err("--redacted-summary cannot be empty".to_string());
    }
    if !plan.transcript_policy.redaction_required
        || plan.transcript_policy.memory_ingestion
            != MemoryIngestionPolicy::ReviewedRedactedSummaryOnly
    {
        return Err(
            "voice summary memory ingestion requires reviewed redacted summary policy".to_string(),
        );
    }
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let summary_hash = stable_cli_hash(redacted_summary);
    let record_id = format!(
        "memory-voice-summary-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}",
            command.project_id, voice_session_id, summary_hash
        ))
    );
    let source_id = format!("source-{record_id}");
    let event_id = format!("event-memory-voice-summary-{summary_hash}");
    let record = MemoryRecordProjection {
        memory_record_id: record_id.clone(),
        project_id: command.project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: command.project_id.to_string(),
        subject_ref: Some(voice_session_id.clone()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "summary".to_string(),
        subject: "voice_conversation".to_string(),
        predicate: "retained_reviewed_summary".to_string(),
        object: voice_intent_label(plan.intent_kind).to_string(),
        body: redacted_summary.trim().to_string(),
        confidence: "medium".to_string(),
        review_state: "reviewed".to_string(),
        source_count: 1,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Redacted.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{record_id}")),
        updated_sequence: 0,
    };
    let source = MemorySourceProjection {
        memory_source_id: source_id,
        memory_record_id: record_id.clone(),
        source_kind: "event".to_string(),
        source_event_id: Some(event_id.clone()),
        source_artifact_id: None,
        source_path: None,
        source_anchor: Some("voice:redacted-summary".to_string()),
        source_content_hash: Some(summary_hash.to_string()),
        source_sequence: None,
        quote_artifact_id: None,
        observed_at: Some("cli".to_string()),
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(event_id, EventKind::MemoryRecordIngested, "capo-voice");
    event.project_id = Some(command.project_id.clone());
    event.payload_json = format!(
        "{{\"memory_record_id\":\"{}\",\"origin\":\"voice\",\"review_state\":\"reviewed\",\"redaction_state\":\"redacted\",\"voice_session_id\":\"{}\",\"intent\":\"{}\",\"summary_hash\":{}}}",
        escape_json(&record.memory_record_id),
        escape_json(&voice_session_id),
        voice_intent_label(plan.intent_kind),
        summary_hash
    );
    event.idempotency_key = Some(format!("voice-summary-memory:{record_id}"));
    event.redaction_state = RedactionState::Redacted;
    state(parsed)?
        .append_event(
            event,
            &[
                ProjectionRecord::MemoryRecord(Box::new(record.clone())),
                ProjectionRecord::MemorySource(source),
            ],
        )
        .map_err(debug_error)?;
    Ok(record)
}

fn queue_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let approval = voice_permission_approval(parsed, plan)?;
    let state = state(parsed)?;
    if let Some(existing) = state
        .permission_approval(&approval.project_id, &approval.approval_id)
        .map_err(debug_error)?
    {
        return Ok(existing);
    }
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-approval-queued-{}",
            stable_cli_hash(&approval.approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!(
        "voice-permission-approval:{}",
        approval.approval_id
    ));
    event.redaction_state = RedactionState::Safe;
    state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(approval)
}

fn decide_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    decision: &str,
) -> Result<PermissionApprovalProjection, String> {
    let approval = queue_voice_permission_approval(parsed, plan)?;
    if approval.status == "decided" {
        return Ok(approval);
    }
    let (effect, persistence) = approval_decision_effect(decision)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-voice-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json,
        decision_source: "user_visible_voice_confirmation".to_string(),
        persistence: persistence.to_string(),
        explanation: format!(
            "visible voice confirmation {decision} for {}",
            approval.approval_id
        ),
        updated_sequence: 0,
    };
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.to_string()),
        capability_grant_id: Some(grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-decided-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::PermissionDecided,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(decision),
        escape_json(&grant.capability_grant_id),
        effect,
        persistence
    );
    event.redaction_state = RedactionState::Safe;
    let mut grant_event = NewEvent::new(
        format!(
            "event-voice-capability-grant-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::CapabilityGrantCreated,
        "capo-voice",
    );
    grant_event.project_id = Some(approval.project_id.clone());
    grant_event.session_id = approval.session_id.clone();
    grant_event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&grant.capability_grant_id),
        escape_json(&grant.effect),
        escape_json(&grant.decision_source),
        escape_json(&grant.persistence)
    );
    grant_event.redaction_state = RedactionState::Safe;
    state(parsed)?
        .decide_permission_approval(
            &approval.approval_id,
            event,
            Some(grant_event),
            decided_approval.clone(),
            Some(grant),
        )
        .map_err(debug_error)?;
    Ok(decided_approval)
}

fn voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice privileged plan missing command".to_string())?;
    let session_id = voice_session_id(parsed, plan)?;
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let approval_id = format!(
        "approval-voice-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}",
            command.command_id,
            command.actor_id,
            session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            voice_intent_label(plan.intent_kind)
        ))
    );
    Ok(PermissionApprovalProjection {
        approval_id,
        project_id: command.project_id.clone(),
        session_id,
        tool_call_id: None,
        capability_profile_id: "voice-control".to_string(),
        scope_json: "[\"voice:approve:privileged\"]".to_string(),
        subject_json: format!(
            "{{\"actor\":\"{}\",\"origin\":\"voice\",\"voice_session_id\":\"{}\",\"command_id\":\"{}\",\"intent\":\"{}\"}}",
            escape_json(&command.actor_id),
            escape_json(&voice_session_id),
            escape_json(command.command_id.as_str()),
            voice_intent_label(plan.intent_kind)
        ),
        status: "pending".to_string(),
        requested_by: format!("voice:{}", command.actor_id),
        reason: format!(
            "visible confirmation required for {}",
            voice_intent_label(plan.intent_kind)
        ),
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    })
}

fn voice_session_id(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<Option<SessionId>, String> {
    match &plan.read_contract.query_scope {
        VoiceReadScope::SessionForAgent { agent_name } | VoiceReadScope::Agent { agent_name } => {
            let dashboard = voice_dashboard(parsed, plan)?;
            Ok(dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
                .and_then(|row| row.session.as_ref())
                .map(|row| row.session.session_id.clone()))
        }
        VoiceReadScope::ProjectDashboard
        | VoiceReadScope::ProjectLatestConnectivityExposure { .. }
        | VoiceReadScope::ProjectRuntimeTargetStatus { .. }
        | VoiceReadScope::ProjectRuntimeTargetControlReadiness { .. }
        | VoiceReadScope::ProjectLatestRuntimeTargetStatus { .. }
        | VoiceReadScope::ProjectAdapterSmokeReportStatus { .. }
        | VoiceReadScope::ProjectLatestAdapterSmokeReport { .. }
        | VoiceReadScope::ProjectDispatchStatus { .. }
        | VoiceReadScope::ProjectLatestDispatchStatus { .. }
        | VoiceReadScope::ProjectDogfoodReadiness
        | VoiceReadScope::ProjectNextWork
        | VoiceReadScope::ProjectRecentWork
        | VoiceReadScope::ProjectReviewNeeds
        | VoiceReadScope::ProjectToolActivity
        | VoiceReadScope::AgentToolActivity { .. }
        | VoiceReadScope::None => Ok(None),
    }
}

fn render_voice_approval(
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

fn render_voice_memory_retention(record: &MemoryRecordProjection) -> String {
    format!(
        "memory_record={}\nmemory_review_state={}\nmemory_redaction_state={}\nmemory_ingestion=reviewed_redacted_summary_only\n",
        record.memory_record_id, record.review_state, record.redaction_state
    )
}

fn voice_dashboard(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<ProjectDashboard, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice plan missing query command".to_string())?;
    project_dashboard(
        &state(parsed)?,
        ProjectDashboardQuery::new(command.project_id.clone()),
    )
    .map_err(debug_error)
}

fn render_voice_header(
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

fn render_voice_read_contract(plan: &VoiceCommandPlan, dashboard: &ProjectDashboard) -> String {
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
                "spoken_dogfood_ready={}\nspoken_dogfood_status={}\nspoken_real_agent_connector_ready={}\nspoken_runtime_target_ready={}\nspoken_workpad_bridge_ready={}\nspoken_dispatch_chain_ready={}\nspoken_blockers={}\nspoken_next_actions={}\n",
                readiness.ready,
                readiness.status,
                readiness.real_agent_connector_ready,
                readiness.runtime_target_ready,
                readiness.workpad_bridge_ready,
                readiness.dispatch_chain_ready,
                comma_or_none(&readiness.blockers),
                comma_or_none(&readiness.next_actions)
            ));
            output.push_str(&format!(
                "spoken_connector_evidence_refs={}\nspoken_runtime_target_refs={}\nspoken_workpad_task_refs={}\nspoken_dispatch_chain_refs={}\nspoken_project_evidence_refs={}\n",
                comma_or_none(&readiness.connector_evidence_refs),
                comma_or_none(&readiness.runtime_target_refs),
                comma_or_none(&readiness.workpad_task_refs),
                comma_or_none(&readiness.dispatch_chain_refs),
                comma_or_none(&readiness.project_evidence_refs)
            ));
        }
        VoiceReadScope::ProjectNextWork => {
            output.push_str(&format!(
                "spoken_workpad_tasks={}\nspoken_next_work_candidates={}\n",
                dashboard.workpad_tasks.len(),
                dashboard.next_workpad_candidate_count()
            ));
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

fn voice_intent_label(intent: VoiceIntentKind) -> &'static str {
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

fn structured_arg_value<'a>(command: &'a CommandEnvelope, key: &str) -> Option<&'a str> {
    command
        .structured_args
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
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

fn index_workpads(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let root = PathBuf::from(required_arg(args, "--root")?);
    let index = index_project_workpads(&root)?;
    let command = envelope(
        "workpad-index",
        CommandTarget::Project(project_id()),
        CommandIntent::IndexWorkpads,
        Some(root.display().to_string()),
    );
    let state = state(parsed)?;
    let existing_statuses = state
        .workpad_tasks(&project_id())
        .map_err(debug_error)?
        .into_iter()
        .map(|task| (task.workpad_task_id, task.capo_execution_status))
        .collect::<HashMap<_, _>>();
    let projections = workpad_index_projections(&index, &existing_statuses);
    let index_fingerprint = index
        .files
        .iter()
        .map(|file| file.content_hash.as_str())
        .collect::<Vec<_>>()
        .join(":");
    let next_sequence_hint = state.last_sequence().map_err(debug_error)? + 1;
    let event_suffix = stable_cli_hash(&format!(
        "{}:{}:{index_fingerprint}",
        root.display(),
        next_sequence_hint
    ));
    let mut event = NewEvent::new(
        format!("event-workpad-index-{}-{event_suffix}", index.observed_unix),
        EventKind::WorkpadIndexed,
        "capo-cli",
    );
    event.project_id = Some(project_id());
    event.payload_json = format!(
        "{{\"root\":\"{}\",\"files\":{},\"tasks\":{},\"observed_unix\":{}}}",
        escape_json(&root.display().to_string()),
        index.files.len(),
        index.tasks.len(),
        index.observed_unix
    );
    event.idempotency_key = None;
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(event, &projections)
        .map_err(debug_error)?;
    Ok(format!(
        "workpads_indexed=true\nroot={}\nfiles={}\ntasks={}\nsequence={sequence}\ncommand_id={}\n",
        root.display(),
        index.files.len(),
        index.tasks.len(),
        command.command_id
    ))
}

fn default_workpad_task_id(workpad_task_id: &str) -> String {
    format!("task-workpad-{}", sanitize_id_component(workpad_task_id))
}

fn next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let path_filter = workpad_path_filter(args)?;
    let state = state(parsed)?;
    let (next, candidate_count) = next_workpad_selection(&state, path_filter.as_deref())?;
    Ok(render_next_workpad_task(
        next.as_ref(),
        candidate_count,
        path_filter.as_deref(),
    ))
}

fn plan_next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let options = WorkpadPlanNextOptions::parse(args, &parsed.state_root)?;
    validate_local_launch_adapter(&options.adapter)?;
    let state = state(parsed)?;
    let (next, candidate_count) = next_workpad_selection(&state, options.path_filter.as_deref())?;
    let next = next.ok_or_else(|| "no actionable observed-only workpad task found".to_string())?;
    let workpad_file = state
        .workpad_file(&project_id(), &next.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", next.path))?;
    let goal = workpad_task_goal(&next);
    let plan = recordable_adapter_dispatch_plan(
        parsed,
        DispatchPlanRecordRequest {
            adapter: &options.adapter,
            agent: &options.agent,
            goal: &goal,
            workspace: options.workspace,
            artifacts: options.artifacts,
            prompt_source: DispatchPromptSourceInput::workpad_task(
                &next,
                workpad_file.content_hash,
            ),
            record: options.record,
        },
    )?;
    Ok(format!(
        "workpad_next_planned=true\nagent={}\nadapter={}\nworkpad_task_id={}\ndefault_task_id={}\nsource={}#{}\ntitle={}\nobserved_status={}\ncapo_execution_status={}\ncandidate_count={}\npath_filter={}\n{}\n",
        options.agent,
        plan.projection.adapter_kind,
        next.workpad_task_id,
        default_workpad_task_id(&next.workpad_task_id),
        next.path,
        next.source_anchor,
        next.title,
        next.observed_status,
        next.capo_execution_status,
        candidate_count,
        options.path_filter.as_deref().unwrap_or("none"),
        render_adapter_dispatch_plan(&plan)
    ))
}

fn start_next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let args_without_agent = remove_option(args, "--agent");
    let path_filter = workpad_path_filter(&args_without_agent)?;
    let state = state(parsed)?;
    let (next, _) = next_workpad_selection(&state, path_filter.as_deref())?;
    let next = next.ok_or_else(|| "no actionable observed-only workpad task found".to_string())?;
    if state.agent_by_name(&agent).map_err(debug_error)?.is_none() {
        return Err(format!("missing registered agent: {agent}"));
    }
    let task_id = default_workpad_task_id(&next.workpad_task_id);
    import_workpad_task(
        parsed,
        &[
            "--workpad-task".to_string(),
            next.workpad_task_id.clone(),
            "--task".to_string(),
            task_id.clone(),
        ],
    )?;
    let goal = format!(
        "Work on {} from {}#{} (workpad_task_id={})",
        next.title, next.path, next.source_anchor, next.workpad_task_id
    );
    let mut command = envelope(
        "workpad-start-next",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::SendTask,
        Some(goal),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    command
        .structured_args
        .push(("scenario".to_string(), "workpad".to_string()));
    command
        .structured_args
        .push(("task_id".to_string(), task_id.clone()));
    let refs = controller(parsed)?
        .send_task_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "workpad_next_started=true\nagent={agent}\nworkpad_task_id={}\ntask_id={}\nsession_id={}\nrun_id={}\nsource={}#{}\nobserved_status={}\ncapo_execution_status=active\ncommand_id={}\n",
        next.workpad_task_id,
        refs.task_id,
        refs.session_id,
        refs.run_id,
        next.path,
        next.source_anchor,
        next.observed_status,
        command.command_id
    ))
}

fn workpad_task_goal(task: &WorkpadTaskProjection) -> String {
    format!(
        "Work on {} from {}#{} (workpad_task_id={})",
        task.title, task.path, task.source_anchor, task.workpad_task_id
    )
}

fn workpad_path_filter(args: &[String]) -> Result<Option<String>, String> {
    let mut path_filter = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "--path requires a value".to_string())?;
                path_filter = Some(value.clone());
                index += 2;
            }
            other => return Err(format!("unknown workpad next option: {other}")),
        }
    }
    Ok(path_filter)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkpadPlanNextOptions {
    agent: String,
    adapter: String,
    path_filter: Option<String>,
    workspace: PathBuf,
    artifacts: PathBuf,
    record: bool,
}

impl WorkpadPlanNextOptions {
    fn parse(args: &[String], state_root: &Path) -> Result<Self, String> {
        let mut agent = None;
        let mut adapter = None;
        let mut path_filter = None;
        let mut workspace = None;
        let mut artifacts = None;
        let mut record = false;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--agent" => {
                    agent = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--agent requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--adapter" => {
                    adapter = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--adapter requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--path" => {
                    path_filter = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--path requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--workspace" => {
                    workspace = Some(PathBuf::from(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--workspace requires a value".to_string())?,
                    ));
                    index += 2;
                }
                "--artifacts" => {
                    artifacts = Some(PathBuf::from(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--artifacts requires a value".to_string())?,
                    ));
                    index += 2;
                }
                "--record" => {
                    record = true;
                    index += 1;
                }
                other => return Err(format!("unknown workpad plan-next option: {other}")),
            }
        }
        Ok(Self {
            agent: agent.ok_or_else(|| "--agent is required".to_string())?,
            adapter: adapter.ok_or_else(|| "--adapter is required".to_string())?,
            path_filter,
            workspace: workspace
                .unwrap_or_else(|| state_root.join("workpad-plan-next").join("workspace")),
            artifacts: artifacts
                .unwrap_or_else(|| state_root.join("workpad-plan-next").join("artifacts")),
            record,
        })
    }
}

fn next_workpad_selection(
    state: &SqliteStateStore,
    path_filter: Option<&str>,
) -> Result<(Option<WorkpadTaskProjection>, usize), String> {
    let mut query = ProjectDashboardQuery::new(project_id());
    if let Some(path) = path_filter {
        query = query.with_workpad_path(path);
    }
    let dashboard = project_dashboard(state, query).map_err(debug_error)?;
    Ok((
        dashboard.next_workpad_task().cloned(),
        dashboard.next_workpad_candidate_count(),
    ))
}

fn render_next_workpad_task(
    next: Option<&WorkpadTaskProjection>,
    candidate_count: usize,
    path_filter: Option<&str>,
) -> String {
    let Some(next) = next else {
        return format!(
            "workpad_next_found=false\ncandidate_count=0\npath_filter={}\n",
            path_filter.unwrap_or("none")
        );
    };
    format!(
        "workpad_next_found=true\ncandidate_count={}\nworkpad_task_id={}\ndefault_task_id={}\npath={}\nsource_anchor={}\nsource={}#{}\ntitle={}\nobserved_status={}\ncapo_execution_status={}\npath_filter={}\n",
        candidate_count,
        next.workpad_task_id,
        default_workpad_task_id(&next.workpad_task_id),
        next.path,
        next.source_anchor,
        next.path,
        next.source_anchor,
        next.title,
        next.observed_status,
        next.capo_execution_status,
        path_filter.unwrap_or("none")
    )
}

fn remove_option(args: &[String], key: &str) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == key {
            index += 2;
        } else {
            filtered.push(args[index].clone());
            index += 1;
        }
    }
    filtered
}

fn import_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let workpad_task_id = required_arg(args, "--workpad-task")?;
    let state = state(parsed)?;
    let project_id = project_id();
    let workpad_task = state
        .workpad_task(&project_id, &workpad_task_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad task read model: {workpad_task_id}"))?;
    let workpad_file = state
        .workpad_file(&project_id, &workpad_task.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", workpad_task.path))?;

    if let Some(expected_hash) = optional_arg(args, "--expected-hash")
        && expected_hash != workpad_file.content_hash
    {
        return Err(format!(
            "source drift detected for {}: expected_hash={} current_hash={}",
            workpad_task.path, expected_hash, workpad_file.content_hash
        ));
    }

    let task_id = TaskId::new(
        optional_arg(args, "--task")
            .unwrap_or_else(|| default_workpad_task_id(&workpad_task.workpad_task_id)),
    );
    if let Some(existing_task) = state.task(&task_id).map_err(debug_error)? {
        let same_source = existing_task
            .latest_summary
            .as_deref()
            .is_some_and(|summary| {
                summary.contains(&format!("workpad_task_id={}", workpad_task.workpad_task_id))
            });
        if !same_source
            || existing_task.capo_execution_status != "ready"
            || existing_task.active_session_id.is_some()
        {
            return Err(format!(
                "refusing to overwrite existing Capo task read model: {task_id}"
            ));
        }
    }
    let mut command = envelope(
        "workpad-import",
        CommandTarget::Task(task_id.clone()),
        CommandIntent::ImportWorkpadTask,
        Some(workpad_task.title.clone()),
    );
    command.structured_args.push((
        "workpad_task_id".to_string(),
        workpad_task.workpad_task_id.clone(),
    ));
    command
        .structured_args
        .push(("source_hash".to_string(), workpad_file.content_hash.clone()));
    let source_ref = format!("{}#{}", workpad_task.path, workpad_task.source_anchor);
    let latest_summary = format!(
        "source={} hash={} observed_status={} workpad_task_id={}",
        source_ref,
        workpad_file.content_hash,
        workpad_task.observed_status,
        workpad_task.workpad_task_id
    );
    let imported_workpad_task = WorkpadTaskProjection {
        capo_execution_status: "imported".to_string(),
        ..workpad_task.clone()
    };
    let task_projection = ProjectionRecord::Task(capo_state::TaskProjection {
        task_id: task_id.clone(),
        project_id: project_id.clone(),
        title: workpad_task.title.clone(),
        capo_execution_status: "ready".to_string(),
        active_session_id: None,
        latest_summary: Some(latest_summary),
        evidence_id: None,
        updated_sequence: 0,
    });
    let mut event = NewEvent::new(
        format!(
            "event-workpad-import-{}",
            stable_cli_hash(&format!(
                "{}:{}:{}",
                task_id, workpad_task.workpad_task_id, workpad_file.content_hash
            ))
        ),
        EventKind::WorkpadTaskImported,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.task_id = Some(task_id.clone());
    event.payload_json = format!(
        "{{\"task_id\":\"{}\",\"workpad_task_id\":\"{}\",\"path\":\"{}\",\"source_anchor\":\"{}\",\"content_hash\":\"{}\",\"observed_status\":\"{}\"}}",
        escape_json(task_id.as_str()),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&workpad_task.path),
        escape_json(&workpad_task.source_anchor),
        escape_json(&workpad_file.content_hash),
        escape_json(&workpad_task.observed_status)
    );
    event.idempotency_key = Some(format!(
        "workpad-import:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[
                task_projection,
                ProjectionRecord::WorkpadTask(imported_workpad_task),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "workpad_task_imported=true\nworkpad_task_id={}\ntask_id={}\nsource={}#{}\nsource_hash={}\nobserved_status={}\ncapo_execution_status=ready\nsequence={sequence}\ncommand_id={}\n",
        workpad_task.workpad_task_id,
        task_id,
        workpad_task.path,
        workpad_task.source_anchor,
        workpad_file.content_hash,
        workpad_task.observed_status,
        command.command_id
    ))
}

fn propose_workpad_update(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let workpad_task_id = required_arg(args, "--workpad-task")?;
    let out = PathBuf::from(required_arg(args, "--out")?);
    let state = state(parsed)?;
    let project_id = project_id();
    let workpad_task = state
        .workpad_task(&project_id, &workpad_task_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad task read model: {workpad_task_id}"))?;
    let workpad_file = state
        .workpad_file(&project_id, &workpad_task.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", workpad_task.path))?;

    if let Some(expected_hash) = optional_arg(args, "--expected-hash")
        && expected_hash != workpad_file.content_hash
    {
        return Err(format!(
            "source drift detected for {}: expected_hash={} current_hash={}",
            workpad_task.path, expected_hash, workpad_file.content_hash
        ));
    }

    let task_id = TaskId::new(
        optional_arg(args, "--task").unwrap_or_else(|| default_workpad_task_id(&workpad_task_id)),
    );
    let summary = optional_arg(args, "--summary").unwrap_or_else(|| {
        format!(
            "Review imported workpad task `{}` before any source markdown update.",
            workpad_task.workpad_task_id
        )
    });
    let command = envelope(
        "workpad-propose",
        CommandTarget::Task(task_id.clone()),
        CommandIntent::WriteWorkpadProposal,
        Some(summary.clone()),
    );
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let proposal_identity = stable_cli_hash(&format!(
        "{}:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash, summary
    ));
    let artifact_id = format!("artifact-workpad-proposal-{proposal_identity}");
    let path = out.join(format!("{artifact_id}.md"));
    let markdown = render_workpad_proposal(
        &task_id,
        &workpad_task,
        &workpad_file,
        &summary,
        &artifact_id,
    );
    write_workpad_proposal_file(&path, &markdown)?;
    let content_hash = stable_cli_hash(&markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "workpad_update_proposal".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;
    let evidence_id = format!("evidence-{artifact_id}");
    let mut event = NewEvent::new(
        format!("event-workpad-proposal-{}", stable_cli_hash(&artifact_id)),
        EventKind::WorkpadProposalWritten,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.task_id = Some(task_id.clone());
    event.payload_json = format!(
        "{{\"task_id\":\"{}\",\"workpad_task_id\":\"{}\",\"artifact_id\":\"{}\",\"path\":\"{}\",\"content_hash\":\"{}\",\"source_hash\":\"{}\"}}",
        escape_json(task_id.as_str()),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&artifact_id),
        escape_json(&path.display().to_string()),
        escape_json(&content_hash),
        escape_json(&workpad_file.content_hash)
    );
    event.idempotency_key = Some(format!(
        "workpad-proposal:{}:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash, proposal_identity
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id,
                task_id: Some(task_id.clone()),
                session_id: None,
                run_id: None,
                kind: "workpad_update_proposal".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: 80,
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "workpad_proposal_written=true\nworkpad_task_id={}\ntask_id={}\nartifact_id={artifact_id}\npath={}\nsource_hash={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        workpad_task.workpad_task_id,
        task_id,
        path.display(),
        workpad_file.content_hash,
        command.command_id
    ))
}

fn apply_workpad_proposal(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let proposal = PathBuf::from(required_arg(args, "--proposal")?);
    let command = envelope(
        "workpad-apply",
        CommandTarget::Project(project_id()),
        CommandIntent::ApplyWorkpadProposal,
        Some(proposal.display().to_string()),
    );
    if !has_flag(args, "--confirm") {
        return Err(
            "explicit --confirm is required before Capo applies workpad source changes".to_string(),
        );
    }
    let markdown = fs::read_to_string(&proposal).map_err(|error| error.to_string())?;
    if !markdown.starts_with("<!-- capo:workpad-proposal -->") {
        return Err(format!(
            "refusing to apply non-Capo workpad proposal: {}",
            proposal.display()
        ));
    }
    let _state = state(parsed)?;
    Ok(format!(
        "workpad_apply_supported=false\nproposal={}\nsource_modified=false\nreason=DB3 only supports reviewed proposal artifacts; apply manually after review using the rollback instructions in the proposal.\ncommand_id={}\n",
        proposal.display(),
        command.command_id
    ))
}

fn export_evidence(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let out = PathBuf::from(required_arg(args, "--out")?);
    let command = envelope(
        "evidence-export",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(out.display().to_string()),
    );
    let state = state(parsed)?;
    let session = state
        .session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing session read model: {session_id}"))?;
    let evidence = state
        .evidence_for_session(&session_id)
        .map_err(debug_error)?;
    let events = state
        .recent_events_for_session(&session_id, 20)
        .map_err(debug_error)?;
    let run = state
        .run_for_session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing run read model for session: {session_id}"))?;
    let tool_calls = state
        .tool_calls_for_session(&session_id)
        .map_err(debug_error)?;
    let tool_observations = state
        .tool_observations_for_session(&session_id)
        .map_err(debug_error)?;
    let memory_packets = state
        .memory_packets_for_session(&session_id)
        .map_err(debug_error)?;
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{session_id}.md"));
    write_evidence_file(
        &path,
        &render_evidence(
            &session,
            &run,
            &evidence,
            &tool_calls,
            &tool_observations,
            &memory_packets,
            &events,
        ),
    )?;
    Ok(format!(
        "evidence_exported=true\npath={}\ncommand_id={}\n",
        path.display(),
        command.command_id
    ))
}

fn export_task_outcome_report(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let out = PathBuf::from(required_arg(args, "--out")?);
    let command = envelope(
        "eval-task-outcome",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(out.display().to_string()),
    );
    let state = state(parsed)?;
    let review_outcome = derive_review_outcome(&state, &session_id)?;
    let report = TaskOutcomeReport::from_state(&state, &session_id, review_outcome.clone(), None)?;
    let artifact_seed = format!(
        "{}:{}",
        report.projection.task_outcome_report_id, review_outcome
    );
    let artifact_id = format!("artifact-task-outcome-{}", stable_cli_hash(&artifact_seed));
    let mut projection = report.projection.clone();
    projection.report_artifact_id = Some(artifact_id.clone());
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{artifact_id}.md"));
    write_task_outcome_report_file(&path, &report.markdown)?;
    let content_hash = stable_cli_hash(&report.markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(projection.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            kind: "task_outcome_report".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: report.markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_id = format!("evidence-{artifact_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!(
                    "event-task-outcome-{}",
                    stable_cli_hash(&projection.task_outcome_report_id)
                ),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "cli".to_string(),
                project_id: Some(projection.project_id.clone()),
                task_id: Some(projection.task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(projection.run_id.clone()),
                turn_id: None,
                item_id: Some(projection.task_outcome_report_id.clone()),
                payload_json: format!(
                    "{{\"task_outcome_report_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"review_outcome\":\"{}\"}}",
                    escape_json(&projection.task_outcome_report_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&review_outcome)
                ),
                idempotency_key: Some(format!(
                    "task-outcome:{}:{}:{}:{}",
                    projection.task_id,
                    session_id,
                    review_outcome,
                    projection.completed_sequence
                )),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::TaskOutcomeReport(projection.clone()),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                    project_id: projection.project_id.clone(),
                    task_id: Some(projection.task_id.clone()),
                    session_id: Some(session_id.clone()),
                    run_id: Some(projection.run_id.clone()),
                    kind: "task_outcome_report".to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: projection.confidence.unwrap_or(0),
                    updated_sequence: 0,
                }),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "task_outcome_report_exported=true\nreport_id={}\ntask_id={}\nsession_id={session_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        projection.task_outcome_report_id,
        projection.task_id,
        path.display(),
        command.command_id
    ))
}

fn record_review_finding(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let reviewer = required_arg(args, "--reviewer")?;
    let finding_kind = required_arg(args, "--kind")?;
    if !matches!(finding_kind.as_str(), "blocker" | "finding" | "no_blockers") {
        return Err("--kind must be blocker, finding, or no_blockers".to_string());
    }
    let summary = required_arg(args, "--summary")?;
    let out = PathBuf::from(required_arg(args, "--out")?);
    let severity =
        optional_arg(args, "--severity").unwrap_or_else(|| default_review_severity(&finding_kind));
    let tool_call_id = optional_arg(args, "--tool-call").map(ToolCallId::new);
    let follow_up_workpad_task_id = optional_arg(args, "--follow-up-workpad-task");
    let state = state(parsed)?;
    let session = state
        .session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing session read model: {session_id}"))?;
    let task_id = session
        .task_id
        .clone()
        .ok_or_else(|| format!("session is not linked to a task: {session_id}"))?;
    let run = state.run_for_session(&session_id).map_err(debug_error)?;
    let run_id = run.as_ref().map(|run| run.run_id.clone());
    if let Some(tool_call_id) = &tool_call_id {
        let session_tool_calls = state
            .tool_calls_for_session(&session_id)
            .map_err(debug_error)?;
        if !session_tool_calls
            .iter()
            .any(|tool_call| &tool_call.tool_call_id == tool_call_id)
        {
            return Err(format!(
                "tool call is not linked to session: {}",
                tool_call_id
            ));
        }
    }
    if let Some(workpad_task_id) = &follow_up_workpad_task_id
        && state
            .workpad_task(&session.project_id, workpad_task_id)
            .map_err(debug_error)?
            .is_none()
    {
        return Err(format!("missing follow-up workpad task: {workpad_task_id}"));
    }
    let command = envelope(
        "review-record",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::RecordReviewFinding,
        Some(summary.clone()),
    );
    let finding_seed = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        session_id,
        reviewer,
        finding_kind,
        severity,
        summary,
        tool_call_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        follow_up_workpad_task_id.as_deref().unwrap_or("none")
    );
    let review_finding_id = format!("review-finding-{}", stable_cli_hash(&finding_seed));
    let artifact_id = format!("artifact-{review_finding_id}");
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{artifact_id}.md"));
    let markdown = render_review_finding_artifact(
        &session,
        run.as_ref(),
        &review_finding_id,
        &artifact_id,
        &reviewer,
        &finding_kind,
        &severity,
        &summary,
        tool_call_id.as_ref(),
        follow_up_workpad_task_id.as_deref(),
    );
    write_review_finding_file(&path, &markdown)?;
    let content_hash = stable_cli_hash(&markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(session.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: run_id.clone(),
            kind: "review".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_kind = review_evidence_kind(&finding_kind);
    let evidence_id = format!("evidence-{review_finding_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!("event-{}", review_finding_id),
                kind: EventKind::ReviewFindingRecorded,
                actor: "cli".to_string(),
                project_id: Some(session.project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: Some(session.agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: run_id.clone(),
                turn_id: None,
                item_id: Some(review_finding_id.clone()),
                payload_json: format!(
                    "{{\"review_finding_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"finding_kind\":\"{}\",\"severity\":\"{}\"}}",
                    escape_json(&review_finding_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&finding_kind),
                    escape_json(&severity)
                ),
                idempotency_key: Some(format!("review-finding:{review_finding_id}")),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                    review_finding_id: review_finding_id.clone(),
                    project_id: session.project_id.clone(),
                    task_id: task_id.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    workpad_task_id: follow_up_workpad_task_id.clone(),
                    reviewer: reviewer.clone(),
                    finding_kind: finding_kind.clone(),
                    severity: severity.clone(),
                    summary: summary.clone(),
                    status: review_finding_status(&finding_kind).to_string(),
                    evidence_artifact_id: Some(artifact_id.clone()),
                    follow_up: follow_up_workpad_task_id.clone(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                    project_id: session.project_id,
                    task_id: Some(task_id),
                    session_id: Some(session_id.clone()),
                    run_id,
                    kind: evidence_kind.to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: review_confidence(&finding_kind),
                    updated_sequence: 0,
                }),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "review_finding_recorded=true\nreview_finding_id={review_finding_id}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\nsequence={sequence}\ncommand_id={}\n",
        path.display(),
        command.command_id
    ))
}

fn workpad_index_projections(
    index: &WorkpadIndex,
    existing_statuses: &HashMap<String, String>,
) -> Vec<ProjectionRecord> {
    let project_id = project_id();
    let mut projections = vec![ProjectionRecord::WorkpadIndexReset(
        WorkpadIndexResetProjection {
            project_id: project_id.clone(),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        },
    )];
    for file in &index.files {
        projections.push(ProjectionRecord::WorkpadFile(WorkpadFileProjection {
            path: file.path.clone(),
            project_id: project_id.clone(),
            content_hash: file.content_hash.clone(),
            headings: file.headings.join("\n"),
            objective: file.objective.clone(),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        }));
    }
    for task in &index.tasks {
        projections.push(ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
            workpad_task_id: task.workpad_task_id.clone(),
            project_id: project_id.clone(),
            path: task.path.clone(),
            source_anchor: task.source_anchor.clone(),
            title: task.title.clone(),
            observed_status: task.observed_status.clone(),
            capo_execution_status: existing_statuses
                .get(&task.workpad_task_id)
                .cloned()
                .unwrap_or_else(|| task.capo_execution_status.clone()),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        }));
    }
    projections
}

fn sanitize_id_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "workpad-task".to_string()
    } else {
        trimmed
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

fn derive_review_outcome(
    state: &SqliteStateStore,
    session_id: &SessionId,
) -> Result<String, String> {
    let findings = state
        .review_findings_for_session(session_id)
        .map_err(debug_error)?;
    if let Some(finding) = findings.iter().max_by_key(|finding| {
        (
            finding.updated_sequence,
            review_finding_precedence(&finding.finding_kind),
        )
    }) {
        return Ok(match finding.finding_kind.as_str() {
            "blocker" | "finding" => "reviewed_with_findings",
            "no_blockers" => "reviewed_no_blockers",
            _ => "not_reviewed",
        }
        .to_string());
    }

    let evidence = state
        .evidence_for_session(session_id)
        .map_err(debug_error)?;
    let latest = evidence
        .iter()
        .filter_map(|item| {
            let rank = match item.kind.as_str() {
                "review_blockers" | "review_findings" => 2,
                "review_no_blockers" | "reviewed_no_blockers" => 1,
                _ => return None,
            };
            Some((item.updated_sequence, rank, item.kind.as_str()))
        })
        .max_by_key(|(sequence, rank, _)| (*sequence, *rank));

    Ok(match latest.map(|(_, _, kind)| kind) {
        Some("review_blockers" | "review_findings") => "reviewed_with_findings",
        Some("review_no_blockers" | "reviewed_no_blockers") => "reviewed_no_blockers",
        _ => "not_reviewed",
    }
    .to_string())
}

fn review_finding_precedence(finding_kind: &str) -> i64 {
    match finding_kind {
        "blocker" => 3,
        "finding" => 2,
        "no_blockers" => 1,
        _ => 0,
    }
}

fn review_evidence_kind(finding_kind: &str) -> &'static str {
    match finding_kind {
        "blocker" => "review_blockers",
        "finding" => "review_findings",
        "no_blockers" => "review_no_blockers",
        _ => "review_findings",
    }
}

fn review_finding_status(finding_kind: &str) -> &'static str {
    match finding_kind {
        "no_blockers" => "closed",
        _ => "open",
    }
}

fn review_confidence(finding_kind: &str) -> i64 {
    match finding_kind {
        "no_blockers" => 90,
        "blocker" => 80,
        _ => 70,
    }
}

fn default_review_severity(finding_kind: &str) -> String {
    match finding_kind {
        "blocker" => "high",
        "finding" => "medium",
        "no_blockers" => "none",
        _ => "medium",
    }
    .to_string()
}

fn render_evidence(
    session: &SessionProjection,
    run: &RunProjection,
    evidence: &[EvidenceProjection],
    tool_calls: &[ToolCallProjection],
    tool_observations: &[ToolObservationProjection],
    memory_packets: &[MemoryPacketProjection],
    events: &[EventRecord],
) -> String {
    let mut markdown = format!(
        "<!-- capo:evidence-export -->\n# Capo Evidence - {}\n\n## Objective\n\n{}\n\n## State Refs\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Session status: `{}`\n- Run: `{}`\n- Run status: `{}`\n- Agent: `{}`\n- Latest summary: {}\n- Confidence: `{}`\n- Blocker: {}\n\n## Evidence Refs\n\n",
        session.title,
        session.current_goal,
        session.project_id,
        session
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session.session_id,
        session.status,
        run.run_id,
        run.status,
        session.agent_id,
        session.latest_summary.as_deref().unwrap_or("none"),
        session
            .latest_confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session.latest_blocker.as_deref().unwrap_or("none")
    );
    if evidence.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for item in evidence {
            markdown.push_str(&format!(
                "- `{}` kind=`{}` artifact=`{}` confidence=`{}`\n",
                item.evidence_id,
                item.kind,
                item.artifact_id.as_deref().unwrap_or("none"),
                item.confidence
            ));
        }
    }
    markdown.push_str("\n## Tool Calls\n\n");
    if tool_calls.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for tool_call in tool_calls {
            markdown.push_str(&format!(
                "- `{}` name=`{}` origin=`{}` status=`{}` input_artifact=`{}` output_artifact=`{}`\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id.as_deref().unwrap_or("none"),
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Tool Observations\n\n");
    if tool_observations.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for observation in tool_observations {
            markdown.push_str(&format!(
                "- `{}` name=`{}` source=`{}` observed_status=`{}` instrumentation=`{}` confidence=`{}` external_ref=`{}` artifact=`{}` raw_event_hash=`{}`\n",
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
    }
    markdown.push_str("\n## Memory Packets\n\n");
    if memory_packets.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for packet in memory_packets {
            markdown.push_str(&format!(
                "- `{}` purpose=`{}` run=`{}` turn=`{}` artifact=`{}`\n",
                packet.memory_packet_id,
                packet.purpose,
                packet
                    .run_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "none".to_string()),
                packet.turn_id.as_deref().unwrap_or("none"),
                packet.packet_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Recent Events\n\n");
    for event in events {
        markdown.push_str(&format!(
            "- `{}` `{}` id=`{}` turn=`{}` item=`{}`\n",
            event.sequence,
            event.kind,
            event.event_id,
            event.turn_id.as_deref().unwrap_or("none"),
            event.item_id.as_deref().unwrap_or("none")
        ));
    }
    markdown
}

#[allow(clippy::too_many_arguments)]
fn render_review_finding_artifact(
    session: &SessionProjection,
    run: Option<&RunProjection>,
    review_finding_id: &str,
    artifact_id: &str,
    reviewer: &str,
    finding_kind: &str,
    severity: &str,
    summary: &str,
    tool_call_id: Option<&ToolCallId>,
    follow_up_workpad_task_id: Option<&str>,
) -> String {
    format!(
        "<!-- capo:review-finding -->\n# Capo Review Finding - {}\n\n## Review\n\n- Review finding: `{}`\n- Reviewer: `{}`\n- Kind: `{}`\n- Severity: `{}`\n- Status: `{}`\n- Artifact: `{}`\n\n## Links\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Run: `{}`\n- Tool call: `{}`\n- Follow-up workpad task: `{}`\n\n## Summary\n\n{}\n",
        session.title,
        review_finding_id,
        reviewer,
        finding_kind,
        severity,
        review_finding_status(finding_kind),
        artifact_id,
        session.project_id,
        session
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session.session_id,
        run.map(|run| run.run_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        tool_call_id
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        follow_up_workpad_task_id.unwrap_or("none"),
        summary
    )
}

fn render_workpad_proposal(
    task_id: &TaskId,
    workpad_task: &WorkpadTaskProjection,
    workpad_file: &WorkpadFileProjection,
    summary: &str,
    artifact_id: &str,
) -> String {
    format!(
        "<!-- capo:workpad-proposal -->\n# Capo Workpad Proposal - {}\n\n## Objective\n\nReview a Capo-owned proposal artifact before any source markdown is edited.\n\n## Source\n\n- Capo task: `{}`\n- Workpad task: `{}`\n- Source path: `{}`\n- Source anchor: `{}`\n- Source hash: `{}`\n- Observed markdown status: `{}`\n- Capo workpad execution status: `{}`\n- Artifact: `{}`\n\n## Proposed Update\n\n{}\n\n## Apply Policy\n\nCapo has not modified `{}`. Automated source writeback is disabled for this proposal. Any source update must be reviewed by a human and must require an explicit confirmation step in Capo before future automated apply support can write markdown.\n\n## Rollback And Fallback\n\n- Fallback: leave the source markdown unchanged and keep this proposal as evidence.\n- Manual apply: edit `{}` by hand after review, then run the normal git diff and test gates.\n- Rollback after manual edits: use git to inspect or restore only the reviewed source file before committing.\n- Recovery: re-run `capo workpad index --root <project> --state <state>` to refresh Capo's observed workpad refs after any manual change.\n",
        workpad_task.title,
        task_id,
        workpad_task.workpad_task_id,
        workpad_task.path,
        workpad_task.source_anchor,
        workpad_file.content_hash,
        workpad_task.observed_status,
        workpad_task.capo_execution_status,
        artifact_id,
        summary,
        workpad_task.path,
        workpad_task.path
    )
}

fn write_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path)
        && !existing.starts_with("<!-- capo:evidence-export -->")
    {
        return Err(format!(
            "refusing to overwrite non-Capo evidence file: {}",
            path.display()
        ));
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
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

fn write_task_outcome_report_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:task-outcome-report -->") {
            return Err(format!(
                "refusing to overwrite non-Capo task outcome report file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo task outcome report file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_review_finding_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:review-finding -->") {
            return Err(format!(
                "refusing to overwrite non-Capo review finding file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo review finding file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_workpad_proposal_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:workpad-proposal -->") {
            return Err(format!(
                "refusing to overwrite non-Capo workpad proposal file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo workpad proposal file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn controller(parsed: &ParsedArgs) -> Result<FakeBoundaryController, String> {
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

fn approval_decision_effect(decision: &str) -> Result<(&'static str, &'static str), String> {
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

fn approval_subject_json(approval: &PermissionApprovalProjection) -> Result<String, String> {
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
