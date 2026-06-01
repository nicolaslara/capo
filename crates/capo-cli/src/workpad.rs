use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{AgentId, CommandIntent, CommandTarget, EvidenceId, TaskId};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{
    ArtifactRecord, EventKind, EvidenceProjection, NewEvent, ProjectionRecord, RedactionState,
    SqliteStateStore, WorkpadFileProjection, WorkpadIndexResetProjection, WorkpadTaskProjection,
};
use capo_workpads::{WorkpadIndex, index_project_workpads};

use crate::adapter_launch::{
    DispatchPlanRecordRequest, DispatchPromptSourceInput, recordable_adapter_dispatch_plan,
    render_adapter_dispatch_plan, validate_local_launch_adapter,
};
use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::project_memory_flow::{
    SourceTaskImportRequest, default_source_task_task_id, import_markdown_source_task,
};
use crate::{
    debug_error, envelope, escape_json, project_id, real_controller, stable_cli_hash, state,
};

pub(crate) fn index_workpads(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

pub(crate) fn default_workpad_task_id(workpad_task_id: &str) -> String {
    default_source_task_task_id(workpad_task_id)
}

pub(crate) fn next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let path_filter = workpad_path_filter(args)?;
    let state = state(parsed)?;
    let (next, candidate_count) = next_workpad_selection(&state, path_filter.as_deref())?;
    Ok(render_next_workpad_task(
        next.as_ref(),
        candidate_count,
        path_filter.as_deref(),
    ))
}

pub(crate) fn plan_next_workpad_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
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

pub(crate) fn start_next_workpad_task(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
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
    // AI3: `capo workpad-next` (and `project memory start-next`, which delegates
    // here) dispatches the per-turn summary tool through the REAL
    // `authorize_and_invoke` seam, not the fake summary shim.
    let refs = real_controller(parsed)?
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

pub(crate) fn workpad_task_goal(task: &WorkpadTaskProjection) -> String {
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

pub(crate) fn import_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let workpad_task_id = required_arg(args, "--workpad-task")?;
    let imported = import_markdown_source_task(
        parsed,
        SourceTaskImportRequest {
            source_task_id: workpad_task_id,
            task_id: optional_arg(args, "--task").map(TaskId::new),
            expected_hash: optional_arg(args, "--expected-hash"),
            command_slug: "workpad-import",
        },
    )?;

    Ok(format!(
        "workpad_task_imported=true\nworkpad_task_id={}\ntask_id={}\nsource_binding_id={}\nsource={}#{}\nsource_hash={}\nobserved_status={}\ncapo_execution_status=ready\nsequence={}\ncommand_id={}\n",
        imported.compatibility_workpad_task_id,
        imported.task_id,
        imported.source_binding_id,
        imported.source_path,
        imported.source_anchor,
        imported.source_hash,
        imported.observed_source_status,
        imported.sequence,
        imported.command_id
    ))
}

pub(crate) fn propose_workpad_update(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
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
                evidence_id: EvidenceId::new(evidence_id.clone()),
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

pub(crate) fn apply_workpad_proposal(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
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
