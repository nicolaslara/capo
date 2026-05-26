use capo_core::{CommandIntent, CommandTarget};
use capo_query::{ProjectDashboardQuery, RuntimeTargetControlReadiness, project_dashboard};
use capo_state::{EventKind, NewEvent, ProjectionRecord, RedactionState, RuntimeTargetProjection};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{debug_error, envelope, project_id, stable_cli_hash, state};

pub(crate) fn register_runtime_target(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--target"
                    | "--name"
                    | "--runner"
                    | "--workspace"
                    | "--artifacts"
                    | "--cwd"
                    | "--capability-profile"
                    | "--endpoint"
                    | "--status"
            )
    }) {
        return Err(format!("unknown runtime target register option: {unknown}"));
    }
    let runtime_target_id = required_arg(args, "--target")?;
    let name = required_arg(args, "--name")?;
    let runner_kind = parse_runtime_runner_kind(&required_arg(args, "--runner")?)?;
    let workspace_root = required_arg(args, "--workspace")?;
    let artifact_root = required_arg(args, "--artifacts")?;
    let default_cwd = optional_arg(args, "--cwd").unwrap_or_else(|| workspace_root.clone());
    let capability_profile_id =
        optional_arg(args, "--capability-profile").unwrap_or_else(|| "read-only-local".to_string());
    let connectivity_endpoint_id = optional_arg(args, "--endpoint");
    let status = parse_runtime_target_status(
        optional_arg(args, "--status")
            .as_deref()
            .unwrap_or("available"),
    )?;
    let target = RuntimeTargetProjection {
        runtime_target_id: runtime_target_id.clone(),
        project_id: project_id(),
        name,
        runner_kind,
        workspace_root,
        artifact_root,
        default_cwd,
        capability_profile_id,
        connectivity_endpoint_id,
        status,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-runtime-target-{}",
            stable_cli_hash(&target.runtime_target_id)
        ),
        EventKind::RuntimeTargetRegistered,
        "capo-cli",
    );
    event.project_id = Some(target.project_id.clone());
    event.item_id = Some(target.runtime_target_id.clone());
    event.payload_json = serde_json::json!({
        "runtime_target_id": target.runtime_target_id,
        "name": target.name,
        "runner_kind": target.runner_kind,
        "workspace_root": target.workspace_root,
        "artifact_root": target.artifact_root,
        "default_cwd": target.default_cwd,
        "capability_profile_id": target.capability_profile_id,
        "connectivity_endpoint_id": target.connectivity_endpoint_id,
        "status": target.status,
        "provider_cli_executed": false,
        "tunnel_opened": false,
    })
    .to_string();
    event.idempotency_key = Some(format!(
        "runtime-target:{}:{}",
        target.project_id, target.runtime_target_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state(parsed)?
        .append_event(event, &[ProjectionRecord::RuntimeTarget(target.clone())])
        .map_err(debug_error)?;
    Ok(render_runtime_target_registration(&target, sequence))
}

pub(crate) fn list_runtime_targets(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "runtime-target-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let targets = state(parsed)?
        .runtime_targets(&project_id())
        .map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\nruntime_targets={}\n",
        command.command_id,
        targets.len()
    );
    for target in &targets {
        output.push_str(&render_runtime_target_row("runtime_target", target));
    }
    Ok(output)
}

pub(crate) fn runtime_target_status(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--target" | "--latest" | "--runner" | "--status"
            )
    }) {
        return Err(format!("unknown runtime target status option: {unknown}"));
    }
    let latest = has_flag(args, "--latest");
    let runtime_target_id = optional_arg(args, "--target");
    let runner_kind = optional_arg(args, "--runner")
        .map(|runner| parse_runtime_runner_kind(&runner))
        .transpose()?;
    let status = optional_arg(args, "--status")
        .map(|status| parse_runtime_target_status(&status))
        .transpose()?;
    if latest && runtime_target_id.is_some() {
        return Err("runtime target status accepts either --target or --latest".to_string());
    }
    if !latest && (runner_kind.is_some() || status.is_some()) {
        return Err("runtime target status --runner/--status filters require --latest".to_string());
    }
    let command_slug = runtime_target_id
        .as_ref()
        .map(|target| format!("runtime-target-status-{target}"))
        .unwrap_or_else(|| "runtime-target-status-latest".to_string());
    let command = envelope(
        &command_slug,
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let target = if latest {
        dashboard
            .latest_runtime_target(runner_kind.as_deref(), status.as_deref())
            .ok_or_else(|| {
                let mut filters = Vec::new();
                if let Some(runner_kind) = &runner_kind {
                    filters.push(format!("runner={runner_kind}"));
                }
                if let Some(status) = &status {
                    filters.push(format!("status={status}"));
                }
                if filters.is_empty() {
                    "no recorded runtime targets".to_string()
                } else {
                    format!("no recorded runtime targets matching {}", filters.join(" "))
                }
            })?
    } else {
        let runtime_target_id = runtime_target_id
            .ok_or_else(|| "runtime target status requires --target or --latest".to_string())?;
        dashboard
            .runtime_target_status(&runtime_target_id)
            .ok_or_else(|| format!("missing runtime target: {runtime_target_id}"))?
    };
    let mut output = format!(
        "command_id={}\nruntime_target_status_found=true\n",
        command.command_id
    );
    if latest {
        output.push_str("runtime_target_selector=latest\n");
        output.push_str(&format!(
            "runtime_target_filter_runner={}\nruntime_target_filter_status={}\n",
            runner_kind.as_deref().unwrap_or("any"),
            status.as_deref().unwrap_or("any")
        ));
    } else {
        output.push_str("runtime_target_selector=exact\n");
    }
    output.push_str(&render_runtime_target_row("runtime_target", target));
    output.push_str(
        "provider_cli_executed=false tunnel_opened=false runtime_process_started=false state_mutated=false\n",
    );
    Ok(output)
}

pub(crate) fn runtime_target_readiness(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--target" | "--latest" | "--runner" | "--status"
            )
    }) {
        return Err(format!(
            "unknown runtime target readiness option: {unknown}"
        ));
    }
    let latest = has_flag(args, "--latest");
    let runtime_target_id = optional_arg(args, "--target");
    let runner_kind = optional_arg(args, "--runner")
        .map(|runner| parse_runtime_runner_kind(&runner))
        .transpose()?;
    let status = optional_arg(args, "--status")
        .map(|status| parse_runtime_target_status(&status))
        .transpose()?;
    if latest && runtime_target_id.is_some() {
        return Err("runtime target readiness accepts either --target or --latest".to_string());
    }
    if !latest && (runner_kind.is_some() || status.is_some()) {
        return Err(
            "runtime target readiness --runner/--status filters require --latest".to_string(),
        );
    }
    let project_id = project_id();
    let command_slug = runtime_target_id
        .as_ref()
        .map(|target| format!("runtime-target-readiness-{target}"))
        .unwrap_or_else(|| "runtime-target-readiness-latest".to_string());
    let command = envelope(
        &command_slug,
        CommandTarget::Project(project_id.clone()),
        CommandIntent::QueryStatus,
        runtime_target_id.clone(),
    );
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id)).map_err(debug_error)?;
    let selected_target_id = if latest {
        dashboard
            .latest_runtime_target(runner_kind.as_deref(), status.as_deref())
            .map(|target| target.runtime_target_id.clone())
            .ok_or_else(|| {
                let mut filters = Vec::new();
                if let Some(runner_kind) = &runner_kind {
                    filters.push(format!("runner={runner_kind}"));
                }
                if let Some(status) = &status {
                    filters.push(format!("status={status}"));
                }
                if filters.is_empty() {
                    "no recorded runtime targets".to_string()
                } else {
                    format!("no recorded runtime targets matching {}", filters.join(" "))
                }
            })?
    } else {
        runtime_target_id
            .ok_or_else(|| "runtime target readiness requires --target or --latest".to_string())?
    };
    let readiness = dashboard
        .runtime_target_control_readiness(&selected_target_id)
        .ok_or_else(|| format!("missing runtime target: {selected_target_id}"))?;
    let mut output = format!(
        "command_id={}\nruntime_target_control_readiness_found=true\n",
        command.command_id
    );
    if latest {
        output.push_str("runtime_target_selector=latest\n");
        output.push_str(&format!(
            "runtime_target_filter_runner={}\nruntime_target_filter_status={}\n",
            runner_kind.as_deref().unwrap_or("any"),
            status.as_deref().unwrap_or("any")
        ));
    } else {
        output.push_str("runtime_target_selector=exact\n");
    }
    output.push_str(&render_runtime_target_control_readiness(&readiness));
    output.push_str(
        "provider_cli_executed=false tunnel_opened=false runtime_process_started=false state_mutated=false\n",
    );
    Ok(output)
}

pub(crate) fn set_runtime_target_status(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--target" | "--status"))
    {
        return Err(format!(
            "unknown runtime target set-status option: {unknown}"
        ));
    }
    let runtime_target_id = required_arg(args, "--target")?;
    let status = parse_runtime_target_status(&required_arg(args, "--status")?)?;
    let state = state(parsed)?;
    let mut target = state
        .runtime_targets(&project_id())
        .map_err(debug_error)?
        .into_iter()
        .find(|target| target.runtime_target_id == runtime_target_id)
        .ok_or_else(|| format!("missing runtime target: {runtime_target_id}"))?;
    target.status = status;
    target.updated_sequence = 0;
    let mut event = NewEvent::new(
        format!(
            "event-runtime-target-status-{}",
            stable_cli_hash(&format!("{}:{}", target.runtime_target_id, target.status))
        ),
        EventKind::RuntimeTargetStatusChanged,
        "capo-cli",
    );
    event.project_id = Some(target.project_id.clone());
    event.item_id = Some(target.runtime_target_id.clone());
    event.payload_json = serde_json::json!({
        "runtime_target_id": target.runtime_target_id.clone(),
        "status": target.status.clone(),
        "provider_cli_executed": false,
        "tunnel_opened": false,
        "runtime_process_started": false,
    })
    .to_string();
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(event, &[ProjectionRecord::RuntimeTarget(target.clone())])
        .map_err(debug_error)?;
    Ok(format!(
        "runtime_target_status_updated=true\nruntime_target={} status={} provider_cli_executed=false tunnel_opened=false runtime_process_started=false sequence={sequence}\n",
        target.runtime_target_id, target.status
    ))
}

fn render_runtime_target_registration(target: &RuntimeTargetProjection, sequence: i64) -> String {
    format!(
        "runtime_target_registered=true\nruntime_target={} name={} runner={} workspace={} artifacts={} default_cwd={} capability_profile={} endpoint={} status={} provider_cli_executed=false tunnel_opened=false sequence={sequence}\n",
        target.runtime_target_id,
        target.name,
        target.runner_kind,
        target.workspace_root,
        target.artifact_root,
        target.default_cwd,
        target.capability_profile_id,
        target.connectivity_endpoint_id.as_deref().unwrap_or("none"),
        target.status
    )
}

pub(crate) fn render_runtime_target_row(label: &str, target: &RuntimeTargetProjection) -> String {
    format!(
        "{label}={} name={} runner={} workspace={} artifacts={} default_cwd={} capability_profile={} endpoint={} status={} updated_sequence={}\n",
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
    )
}

pub(crate) fn render_runtime_target_control_readiness(
    readiness: &RuntimeTargetControlReadiness,
) -> String {
    format!(
        "runtime_target={} runner={} target_status={} target_ready={} control_exposure_ready={} control_exposure={} control_exposure_status={} control_exposure_scope={} control_exposure_permission_scope={} control_exposure_reachable={} ready={} blockers={} next_action={}\n",
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
    )
}

pub(crate) fn parse_runtime_runner_kind(value: &str) -> Result<String, String> {
    match value {
        "local-process" | "remote-process" | "container" => Ok(value.to_string()),
        other => Err(format!(
            "unsupported runtime runner kind: {other}; expected local-process, remote-process, or container"
        )),
    }
}

pub(crate) fn parse_runtime_target_status(value: &str) -> Result<String, String> {
    match value {
        "available" | "disabled" | "unhealthy" => Ok(value.to_string()),
        other => Err(format!(
            "unsupported runtime target status: {other}; expected available, disabled, or unhealthy"
        )),
    }
}
