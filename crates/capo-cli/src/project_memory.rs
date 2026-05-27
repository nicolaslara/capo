use crate::cli_surface::ParsedArgs;
use crate::project_memory_flow::{SourceTaskImportRequest, import_markdown_source_task};
use crate::workpad::{
    apply_workpad_proposal, index_workpads, next_workpad_task, plan_next_workpad_task,
    propose_workpad_update, start_next_workpad_task,
};
use crate::{debug_error, project_id, state};
use capo_core::TaskId;
use capo_query::{ProjectDashboardQuery, SourceTaskProjection, project_dashboard};

pub(crate) fn index_project_memory(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    annotate("project_memory_indexed=true", index_workpads(parsed, args)?)
}

pub(crate) fn next_project_memory_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let selection = select_next_source_task(parsed, &project_memory_path_args(args)?)?;
    let mut output = format!(
        "project_memory_next_found={}\nproject_memory_source=markdown\ncompatibility_adapter=workpad\ncandidate_count={}\npath_filter={}\n",
        selection.next.is_some(),
        selection.candidate_count,
        selection.path_filter.as_deref().unwrap_or("none")
    );
    if let Some(next) = &selection.next {
        output.push_str(&render_source_task_fields(next));
    }
    output.push_str(&next_workpad_task(parsed, args)?);
    Ok(output)
}

pub(crate) fn plan_next_project_memory_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let selection = select_next_source_task(parsed, &project_memory_path_args(args)?)?;
    let mut output = annotate(
        "project_memory_next_planned=true",
        plan_next_workpad_task(parsed, args)?,
    )?;
    if let Some(next) = &selection.next {
        output = format!(
            "project_memory_helper=source_task_selection\n{}{}",
            render_source_task_fields(next),
            output
        );
    }
    Ok(output)
}

pub(crate) fn start_next_project_memory_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let selection = select_next_source_task(parsed, &project_memory_path_args(args)?)?;
    let mut output = annotate(
        "project_memory_next_started=true",
        start_next_workpad_task(parsed, args)?,
    )?;
    if let Some(next) = &selection.next {
        output = format!(
            "project_memory_helper=source_task_selection\n{}{}",
            render_source_task_fields(next),
            output
        );
    }
    Ok(output)
}

pub(crate) fn import_project_memory_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let request = parse_source_task_import(args, "project-memory-import")?;
    let imported = import_markdown_source_task(parsed, request)?;
    Ok(format!(
        "project_memory_task_imported=true\nproject_memory_source=markdown\ncompatibility_adapter=workpad\nsource_task_id={}\ncompatibility_workpad_task_id={}\ntask_id={}\nsource_binding_id={}\nsource={}#{}\nsource_path={}\nsource_anchor={}\nsource_hash={}\nobserved_source_status={}\ncapo_binding_status={}\nsequence={}\ncommand_id={}\nworkpad_task_imported=true\nworkpad_task_id={}\nobserved_status={}\ncapo_execution_status=ready\n",
        imported.source_task_id,
        imported.compatibility_workpad_task_id,
        imported.task_id,
        imported.source_binding_id,
        imported.source_path,
        imported.source_anchor,
        imported.source_path,
        imported.source_anchor,
        imported.source_hash,
        imported.observed_source_status,
        imported.capo_binding_status,
        imported.sequence,
        imported.command_id,
        imported.compatibility_workpad_task_id,
        imported.observed_source_status
    ))
}

pub(crate) fn propose_project_memory_update(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    annotate(
        "project_memory_proposal_written=true",
        propose_workpad_update(parsed, &normalize_source_task_arg(args)?)?,
    )
}

pub(crate) fn apply_project_memory_proposal(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let output = apply_workpad_proposal(parsed, args)?;
    let supported = if output.contains("workpad_apply_supported=true") {
        "true"
    } else {
        "false"
    };
    annotate(
        &format!("project_memory_apply_supported={supported}"),
        output,
    )
}

fn annotate(prefix: &str, output: String) -> Result<String, String> {
    let mut aliases = Vec::new();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("workpad_task_id=") {
            aliases.push(format!("source_task_id={value}"));
        } else if let Some(value) = line.strip_prefix("workpad_next_started=") {
            aliases.push(format!("project_memory_next_started={value}"));
        } else if let Some(value) = line.strip_prefix("workpad_next_planned=") {
            aliases.push(format!("project_memory_next_planned={value}"));
        } else if let Some(value) = line.strip_prefix("workpad_task_imported=") {
            aliases.push(format!("project_memory_task_imported={value}"));
        } else if let Some(value) = line.strip_prefix("workpad_proposal_written=") {
            aliases.push(format!("project_memory_proposal_written={value}"));
        }
    }
    let alias_block = if aliases.is_empty() {
        String::new()
    } else {
        format!("{}\n", aliases.join("\n"))
    };
    Ok(format!(
        "{prefix}\nproject_memory_source=markdown\ncompatibility_adapter=workpad\n{alias_block}{output}"
    ))
}

fn normalize_source_task_arg(args: &[String]) -> Result<Vec<String>, String> {
    let has_source_task = args.iter().any(|arg| arg == "--source-task");
    let has_workpad_task = args.iter().any(|arg| arg == "--workpad-task");
    if has_source_task && has_workpad_task {
        return Err(
            "use either --source-task or compatibility --workpad-task, not both".to_string(),
        );
    }

    Ok(args
        .iter()
        .map(|arg| {
            if arg == "--source-task" {
                "--workpad-task".to_string()
            } else {
                arg.clone()
            }
        })
        .collect())
}

struct SourceTaskSelection {
    next: Option<SourceTaskProjection>,
    candidate_count: usize,
    path_filter: Option<String>,
}

fn select_next_source_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<SourceTaskSelection, String> {
    let path_filter = project_memory_path_filter(args)?;
    let state = state(parsed)?;
    let mut query = ProjectDashboardQuery::new(project_id());
    if let Some(path) = &path_filter {
        query = query.with_workpad_path(path);
    }
    let dashboard = project_dashboard(&state, query).map_err(debug_error)?;
    Ok(SourceTaskSelection {
        next: dashboard.next_source_task(),
        candidate_count: dashboard.next_source_task_candidate_count(),
        path_filter,
    })
}

fn render_source_task_fields(next: &SourceTaskProjection) -> String {
    format!(
        "source_task_id={}\ncompatibility_workpad_task_id={}\nsource={}#{}\nsource_path={}\nsource_anchor={}\ntitle={}\nobserved_source_status={}\ncapo_binding_status={}\n",
        next.source_task_id,
        next.compatibility_workpad_task_id,
        next.source_path,
        next.source_anchor,
        next.source_path,
        next.source_anchor,
        next.title,
        next.observed_source_status,
        next.capo_binding_status
    )
}

fn project_memory_path_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut path_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "--path requires a value".to_string())?
                    .clone();
                path_args.push("--path".to_string());
                path_args.push(value);
                index += 2;
            }
            "--agent" | "--adapter" | "--workspace" | "--artifacts" => {
                if args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .is_none()
                {
                    return Err(format!("{} requires a value", args[index]));
                }
                index += 2;
            }
            "--record" => {
                index += 1;
            }
            other => return Err(format!("unknown project memory next option: {other}")),
        }
    }
    Ok(path_args)
}

fn parse_source_task_import(
    args: &[String],
    command_slug: &'static str,
) -> Result<SourceTaskImportRequest, String> {
    let mut source_task_id = None;
    let mut task_id = None;
    let mut expected_hash = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-task" | "--workpad-task" => {
                if source_task_id.is_some() {
                    return Err("use only one source task selector".to_string());
                }
                source_task_id = Some(
                    args.get(index + 1)
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| format!("{} requires a value", args[index]))?
                        .clone(),
                );
                index += 2;
            }
            "--task" => {
                task_id = Some(TaskId::new(
                    args.get(index + 1)
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| "--task requires a value".to_string())?
                        .clone(),
                ));
                index += 2;
            }
            "--expected-hash" => {
                expected_hash = Some(
                    args.get(index + 1)
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| "--expected-hash requires a value".to_string())?
                        .clone(),
                );
                index += 2;
            }
            other => return Err(format!("unknown project memory import option: {other}")),
        }
    }
    Ok(SourceTaskImportRequest {
        source_task_id: source_task_id.ok_or_else(|| "--source-task is required".to_string())?,
        task_id,
        expected_hash,
        command_slug,
    })
}

fn project_memory_path_filter(args: &[String]) -> Result<Option<String>, String> {
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
            other => return Err(format!("unknown project memory next option: {other}")),
        }
    }
    Ok(path_filter)
}
