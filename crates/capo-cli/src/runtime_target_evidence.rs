use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, ProjectId};
use capo_query::{ProjectDashboardQuery, RuntimeTargetControlReadiness, project_dashboard};
use capo_state::{
    ArtifactRecord, EventKind, EvidenceProjection, NewEvent, ProjectionRecord, RedactionState,
    RuntimeTargetProjection,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::runtime_target::{parse_runtime_runner_kind, parse_runtime_target_status};
use crate::{debug_error, envelope, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn runtime_target_readiness_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--target" | "--latest" | "--runner" | "--status" | "--out"
            )
    }) {
        return Err(format!(
            "unknown runtime target readiness-evidence option: {unknown}"
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
        return Err(
            "runtime target readiness-evidence accepts either --target or --latest".to_string(),
        );
    }
    if !latest && (runner_kind.is_some() || status.is_some()) {
        return Err(
            "runtime target readiness-evidence --runner/--status filters require --latest"
                .to_string(),
        );
    }
    let out = PathBuf::from(required_arg(args, "--out")?);
    let project_id = project_id();
    let command_item_id = runtime_target_id
        .clone()
        .unwrap_or_else(|| "latest".to_string());
    let command = envelope(
        "runtime-target-readiness-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        Some(command_item_id),
    );
    let state = state(parsed)?;
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
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
        runtime_target_id.ok_or_else(|| {
            "runtime target readiness-evidence requires --target or --latest".to_string()
        })?
    };
    let readiness = dashboard
        .runtime_target_control_readiness(&selected_target_id)
        .ok_or_else(|| format!("missing runtime target: {selected_target_id}"))?;
    let markdown = render_runtime_target_readiness_evidence(&project_id, &readiness);
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-runtime-target-readiness-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_runtime_target_readiness_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "runtime_target_readiness_evidence".to_string(),
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
                    "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"runtime_target_id\":\"{}\",\"ready\":{},\"control_exposure_status\":\"{}\"}}",
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&readiness.runtime_target_id),
                    readiness.ready,
                    escape_json(&readiness.control_exposure_status)
                ),
                idempotency_key: Some(format!(
                    "runtime-target-readiness-evidence:{}:{}:{content_hash}",
                    project_id, readiness.runtime_target_id
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "runtime_target_readiness_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: runtime_target_readiness_evidence_confidence(&readiness),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    let mut output = format!(
        "runtime_target_readiness_evidence_exported=true\nruntime_target={}\nready={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        readiness.runtime_target_id,
        readiness.ready,
        path.display(),
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
    output.push_str(
        "provider_cli_executed=false tunnel_opened=false runtime_process_started=false\n",
    );
    Ok(output)
}

pub(crate) fn runtime_target_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--target" | "--latest" | "--runner" | "--status" | "--out"
            )
    }) {
        return Err(format!("unknown runtime target evidence option: {unknown}"));
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
        return Err("runtime target evidence accepts either --target or --latest".to_string());
    }
    if !latest && (runner_kind.is_some() || status.is_some()) {
        return Err(
            "runtime target evidence --runner/--status filters require --latest".to_string(),
        );
    }
    let out = PathBuf::from(required_arg(args, "--out")?);
    let project_id = project_id();
    let command_item_id = runtime_target_id
        .clone()
        .unwrap_or_else(|| "latest".to_string());
    let command = envelope(
        "runtime-target-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        Some(command_item_id),
    );
    let state = state(parsed)?;
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
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
            .clone()
    } else {
        let runtime_target_id = runtime_target_id
            .ok_or_else(|| "runtime target evidence requires --target or --latest".to_string())?;
        dashboard
            .runtime_target_status(&runtime_target_id)
            .ok_or_else(|| format!("missing runtime target: {runtime_target_id}"))?
            .clone()
    };
    let markdown = render_runtime_target_evidence(&project_id, &target);
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-runtime-target-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_runtime_target_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "runtime_target_evidence".to_string(),
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
                    "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"runtime_target_id\":\"{}\",\"status\":\"{}\"}}",
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&target.runtime_target_id),
                    escape_json(&target.status)
                ),
                idempotency_key: Some(format!(
                    "runtime-target-evidence:{}:{}:{content_hash}",
                    project_id, target.runtime_target_id
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "runtime_target_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: runtime_target_evidence_confidence(&target),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    let mut output = format!(
        "runtime_target_evidence_exported=true\nruntime_target={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        target.runtime_target_id,
        path.display(),
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
    output.push_str(
        "provider_cli_executed=false tunnel_opened=false runtime_process_started=false state_mutated=false\n",
    );
    Ok(output)
}

fn runtime_target_readiness_evidence_confidence(readiness: &RuntimeTargetControlReadiness) -> i64 {
    if readiness.ready {
        85
    } else if readiness.target_ready {
        70
    } else {
        60
    }
}

fn runtime_target_evidence_confidence(target: &RuntimeTargetProjection) -> i64 {
    match target.status.as_str() {
        "available" => 80,
        "disabled" => 75,
        "unhealthy" => 70,
        _ => 60,
    }
}

fn render_runtime_target_evidence(
    project_id: &ProjectId,
    target: &RuntimeTargetProjection,
) -> String {
    format!(
        "<!-- capo:runtime-target-evidence -->\n# Capo Runtime Target Evidence - {}\n\n## Objective\n\nReview a registered runtime target without launching runtime/provider processes or opening connectivity tunnels.\n\n## Runtime Target\n\n- Project: `{}`\n- Runtime target: `{}`\n- Name: `{}`\n- Runner: `{}`\n- Workspace root: `{}`\n- Artifact root: `{}`\n- Default cwd: `{}`\n- Capability profile: `{}`\n- Connectivity endpoint: `{}`\n- Status: `{}`\n- Updated sequence: `{}`\n\n## Review Notes\n\n- Runtime targets describe execution placement metadata; they are not proof that a process is running.\n- Connectivity endpoint references are binding metadata for later tunnel/exposure validation, not evidence that a tunnel is open.\n- Disabled or unhealthy targets remain visible for operator review but should fail recorded exposure guards.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo runtime target read models only.\n- It does not launch runtimes, launch provider CLIs, open tunnels, inspect credentials, materialize prompts, request approvals, activate grants, retain raw transcripts, or mutate runtime target state.\n- Credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are not rendered.\n",
        target.runtime_target_id,
        project_id,
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

fn render_runtime_target_readiness_evidence(
    project_id: &ProjectId,
    readiness: &RuntimeTargetControlReadiness,
) -> String {
    format!(
        "<!-- capo:runtime-target-readiness-evidence -->\n# Capo Runtime Target Control Readiness Evidence - {}\n\n## Objective\n\nReview whether a runtime target is ready for remote control from persisted Capo read models, without opening tunnels or launching runtime/provider processes.\n\n## Runtime Target Control Readiness\n\n- Project: `{}`\n- Runtime target: `{}`\n- Runner: `{}`\n- Target status: `{}`\n- Target ready: `{}`\n- Control exposure ready: `{}`\n- Control exposure: `{}`\n- Control exposure status: `{}`\n- Control exposure scope: `{}`\n- Control exposure permission scope: `{}`\n- Control exposure reachable: `{}`\n- Ready for control: `{}`\n- Blockers: `{}`\n- Next action: `{}`\n\n## Review Notes\n\n- Runtime target control readiness is an aggregate over runtime target metadata and the latest runtime-target-owned `control` connectivity exposure.\n- A target is ready only when the target is `available` and the latest control exposure is `active` and reachable.\n- This report is operator guidance. It does not prove live network reachability beyond the stored exposure health/reachability projection.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo runtime target and connectivity exposure read models only.\n- It does not launch runtimes, launch provider CLIs, open tunnels, inspect credentials, materialize prompts, request approvals, activate grants, retain raw transcripts, or mutate runtime target/connectivity state.\n- Credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are not rendered.\n",
        readiness.runtime_target_id,
        project_id,
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

fn write_runtime_target_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:runtime-target-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo runtime target evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo runtime target evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_runtime_target_readiness_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:runtime-target-readiness-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo runtime target readiness evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo runtime target readiness evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
