use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, EvidenceId, ProjectId};
use capo_query::{
    ProjectDashboardQuery, ProjectDogfoodReadiness, project_dashboard, project_dogfood_readiness,
};
use capo_state::{
    ArtifactRecord, EventKind, EvidenceProjection, NewEvent, ProjectionRecord, RedactionState,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg};
use crate::{
    comma_or_none, debug_error, envelope, escape_json, project_id, stable_cli_hash, state,
};

pub(crate) fn dogfood_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
                    evidence_id: EvidenceId::new(evidence_id.clone()),
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
