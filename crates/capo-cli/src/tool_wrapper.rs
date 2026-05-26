use std::path::PathBuf;

use capo_core::{AgentId, ProjectId, RunId, SessionId, ToolCallId};
use capo_state::{
    AgentProjection, ArtifactRecord, EventKind, NewEvent, ProjectionRecord, RedactionState,
    RunProjection, SessionProjection, ToolCallProjection,
};
use capo_tools::{
    PermissionPolicy, RuntimeToolConfig, RuntimeToolWrappers, WrapperArtifact, WrapperToolRequest,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{debug_error, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn run_wrapper_tool(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
