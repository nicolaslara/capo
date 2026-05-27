use capo_core::{CommandIntent, CommandTarget, TaskId};
use capo_state::{
    EventKind, NewEvent, ProjectionRecord, RedactionState, SourceBindingProjection, TaskProjection,
    WorkpadTaskProjection,
};

use crate::cli_surface::ParsedArgs;
use crate::{debug_error, envelope, escape_json, project_id, stable_cli_hash, state};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceTaskImportRequest {
    pub source_task_id: String,
    pub task_id: Option<TaskId>,
    pub expected_hash: Option<String>,
    pub command_slug: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportedSourceTask {
    pub source_task_id: String,
    pub compatibility_workpad_task_id: String,
    pub task_id: TaskId,
    pub source_binding_id: String,
    pub source_path: String,
    pub source_anchor: String,
    pub source_hash: String,
    pub observed_source_status: String,
    pub capo_binding_status: String,
    pub sequence: i64,
    pub command_id: String,
}

pub(crate) fn default_source_task_task_id(source_task_id: &str) -> String {
    format!("task-workpad-{}", sanitize_id_component(source_task_id))
}

pub(crate) fn source_binding_id(task_id: &TaskId) -> String {
    format!("source-binding-{task_id}")
}

pub(crate) fn import_markdown_source_task(
    parsed: &ParsedArgs,
    request: SourceTaskImportRequest,
) -> Result<ImportedSourceTask, String> {
    let state = state(parsed)?;
    let project_id = project_id();
    let workpad_task = state
        .workpad_task(&project_id, &request.source_task_id)
        .map_err(debug_error)?
        .ok_or_else(|| {
            format!(
                "missing markdown source task read model: {}",
                request.source_task_id
            )
        })?;
    let workpad_file = state
        .workpad_file(&project_id, &workpad_task.path)
        .map_err(debug_error)?
        .ok_or_else(|| {
            format!(
                "missing markdown source file read model: {}",
                workpad_task.path
            )
        })?;

    if let Some(expected_hash) = &request.expected_hash
        && expected_hash != &workpad_file.content_hash
    {
        return Err(format!(
            "source drift detected for {}: expected_hash={} current_hash={}",
            workpad_task.path, expected_hash, workpad_file.content_hash
        ));
    }

    let task_id = request
        .task_id
        .unwrap_or_else(|| TaskId::new(default_source_task_task_id(&workpad_task.workpad_task_id)));
    if let Some(existing_task) = state.task(&task_id).map_err(debug_error)? {
        let same_source_binding = state
            .source_binding_for_task(&task_id)
            .map_err(debug_error)?
            .is_some_and(|binding| {
                binding.source_task_id == workpad_task.workpad_task_id
                    && binding.source_hash == workpad_file.content_hash
            });
        let same_source_summary = existing_task
            .latest_summary
            .as_deref()
            .is_some_and(|summary| {
                summary.contains(&format!("workpad_task_id={}", workpad_task.workpad_task_id))
            });
        let same_source = same_source_binding || same_source_summary;
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
        request.command_slug,
        CommandTarget::Task(task_id.clone()),
        CommandIntent::ImportWorkpadTask,
        Some(workpad_task.title.clone()),
    );
    command.structured_args.push((
        "source_task_id".to_string(),
        workpad_task.workpad_task_id.clone(),
    ));
    command.structured_args.push((
        "compatibility_workpad_task_id".to_string(),
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
    let source_binding_id = source_binding_id(&task_id);
    let source_binding = ProjectionRecord::SourceBinding(SourceBindingProjection {
        source_binding_id: source_binding_id.clone(),
        project_id: project_id.clone(),
        task_id: task_id.clone(),
        source_kind: "markdown".to_string(),
        source_task_id: workpad_task.workpad_task_id.clone(),
        source_path: workpad_task.path.clone(),
        source_anchor: workpad_task.source_anchor.clone(),
        source_hash: workpad_file.content_hash.clone(),
        binding_status: "active".to_string(),
        updated_sequence: 0,
    });
    let task_projection = ProjectionRecord::Task(TaskProjection {
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
            "event-{}-{}",
            request.command_slug,
            stable_cli_hash(&format!(
                "{}:{}:{}",
                task_id, workpad_task.workpad_task_id, workpad_file.content_hash
            ))
        ),
        EventKind::WorkpadTaskImported,
        "capo-cli",
    );
    event.project_id = Some(project_id);
    event.task_id = Some(task_id.clone());
    event.payload_json = format!(
        "{{\"task_id\":\"{}\",\"source_task_id\":\"{}\",\"compatibility_workpad_task_id\":\"{}\",\"path\":\"{}\",\"source_anchor\":\"{}\",\"content_hash\":\"{}\",\"observed_status\":\"{}\"}}",
        escape_json(task_id.as_str()),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&workpad_task.path),
        escape_json(&workpad_task.source_anchor),
        escape_json(&workpad_file.content_hash),
        escape_json(&workpad_task.observed_status)
    );
    event.idempotency_key = Some(format!(
        "{}:{}:{}:{}",
        request.command_slug, task_id, workpad_task.workpad_task_id, workpad_file.content_hash
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[
                task_projection,
                source_binding,
                ProjectionRecord::WorkpadTask(imported_workpad_task),
            ],
        )
        .map_err(debug_error)?;

    Ok(ImportedSourceTask {
        source_task_id: workpad_task.workpad_task_id.clone(),
        compatibility_workpad_task_id: workpad_task.workpad_task_id,
        task_id,
        source_binding_id,
        source_path: workpad_task.path,
        source_anchor: workpad_task.source_anchor,
        source_hash: workpad_file.content_hash,
        observed_source_status: workpad_task.observed_status,
        capo_binding_status: "active".to_string(),
        sequence,
        command_id: command.command_id.to_string(),
    })
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
