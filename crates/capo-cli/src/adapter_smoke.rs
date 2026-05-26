use std::fs;
use std::path::{Path, PathBuf};

use capo_adapters::{LocalAdapterSmokeError, scan_artifacts_for_sensitive_markers};
use capo_core::{CommandIntent, CommandTarget, ProjectId};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{
    AdapterSmokeReportProjection, ArtifactRecord, EventKind, EvidenceProjection, NewEvent,
    ProjectionRecord, RedactionState,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{
    adapter_label, debug_error, envelope, escape_json, project_id, stable_cli_hash, state,
};

pub(crate) fn record_adapter_smoke_report(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let adapter = adapter_label(&required_arg(args, "--adapter")?).to_string();
    if !matches!(adapter.as_str(), "codex_exec" | "claude_code") {
        return Err("adapter smoke reports currently support codex or claude".to_string());
    }
    let smoke_status = required_arg(args, "--status")?;
    if !matches!(smoke_status.as_str(), "skipped" | "passed" | "failed") {
        return Err("--status must be skipped, passed, or failed".to_string());
    }
    let credential_scan_status = required_arg(args, "--credential-scan")?;
    if !matches!(
        credential_scan_status.as_str(),
        "clean" | "blocked" | "not_run"
    ) {
        return Err("--credential-scan must be clean, blocked, or not_run".to_string());
    }
    let reason = required_arg(args, "--reason")?;
    let marker_found = has_flag(args, "--marker-found");
    let artifact_root = optional_arg(args, "--artifact-root");
    if smoke_status == "passed" && (credential_scan_status != "clean" || !marker_found) {
        return Err(
            "passed smoke reports require --credential-scan clean and --marker-found".to_string(),
        );
    }
    if smoke_status == "passed" {
        let artifact_root = artifact_root
            .as_ref()
            .ok_or_else(|| "passed smoke reports require --artifact-root".to_string())?;
        scan_artifact_root(Path::new(artifact_root))?;
    }
    let smoke_report_id = format!(
        "adapter-smoke-{}-{}",
        adapter,
        stable_cli_hash(&format!(
            "{adapter}:{smoke_status}:{credential_scan_status}:{marker_found}:{reason}"
        ))
    );
    let dogfood_readiness_effect =
        if smoke_status == "passed" && credential_scan_status == "clean" && marker_found {
            "real_agent_connector_proven"
        } else {
            "real_subscription_smoke_not_recorded"
        };
    let report = AdapterSmokeReportProjection {
        smoke_report_id: smoke_report_id.clone(),
        project_id: project_id(),
        adapter_kind: adapter.clone(),
        smoke_status: smoke_status.clone(),
        credential_scan_status: credential_scan_status.clone(),
        marker_found,
        artifact_root: artifact_root.clone(),
        reason: reason.clone(),
        dogfood_readiness_effect: dogfood_readiness_effect.to_string(),
        updated_sequence: 0,
    };
    let event = NewEvent {
        event_id: format!("event-adapter-smoke-{}", stable_cli_hash(&smoke_report_id)),
        kind: EventKind::AdapterSmokeRecorded,
        actor: "local-cli".to_string(),
        project_id: Some(project_id()),
        task_id: None,
        agent_id: None,
        session_id: None,
        run_id: None,
        turn_id: None,
        item_id: Some(smoke_report_id.clone()),
        payload_json: format!(
            "{{\"adapter\":\"{}\",\"smoke_status\":\"{}\",\"credential_scan_status\":\"{}\",\"dogfood_readiness_effect\":\"{}\"}}",
            escape_json(&adapter),
            escape_json(&smoke_status),
            escape_json(&credential_scan_status),
            escape_json(dogfood_readiness_effect)
        ),
        idempotency_key: Some(format!("adapter-smoke-report:{smoke_report_id}")),
        redaction_state: RedactionState::Safe,
    };
    let sequence = state(parsed)?
        .append_event(event, &[ProjectionRecord::AdapterSmokeReport(report)])
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_smoke_report_recorded=true\nsmoke_report_id={smoke_report_id}\nadapter={adapter}\nsmoke_status={smoke_status}\ncredential_scan_status={credential_scan_status}\nmarker_found={marker_found}\ndogfood_readiness_effect={dogfood_readiness_effect}\nartifact_root={}\nsequence={sequence}\n",
        artifact_root.as_deref().unwrap_or("none")
    ))
}

pub(crate) fn adapter_smoke_report_status(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let smoke_report_id = optional_arg(args, "--smoke-report");
    let adapter = optional_arg(args, "--adapter")
        .map(|adapter| adapter_label(&adapter).to_string())
        .filter(|adapter| adapter != "unknown");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(arg.as_str(), "--smoke-report" | "--latest" | "--adapter")
    }) {
        return Err(format!(
            "unknown adapter smoke-report status option: {unknown}"
        ));
    }
    if optional_arg(args, "--adapter").is_some() && adapter.is_none() {
        return Err("--adapter must be codex or claude".to_string());
    }
    if latest && smoke_report_id.is_some() {
        return Err(
            "adapter smoke-report status accepts either --smoke-report or --latest".to_string(),
        );
    }
    if !latest && adapter.is_some() {
        return Err("adapter smoke-report status --adapter filter requires --latest".to_string());
    }

    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let report = if latest {
        dashboard
            .latest_adapter_smoke_report(adapter.as_deref())
            .ok_or_else(|| {
                adapter
                    .as_ref()
                    .map(|adapter| {
                        format!("no recorded adapter smoke reports matching adapter={adapter}")
                    })
                    .unwrap_or_else(|| "no recorded adapter smoke reports".to_string())
            })?
    } else {
        let smoke_report_id = smoke_report_id.ok_or_else(|| {
            "adapter smoke-report status requires --smoke-report or --latest".to_string()
        })?;
        dashboard
            .adapter_smoke_report_status(&smoke_report_id)
            .ok_or_else(|| format!("missing adapter smoke report: {smoke_report_id}"))?
    };
    Ok(render_adapter_smoke_report_status(report))
}

pub(crate) fn adapter_smoke_report_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let smoke_report_id = optional_arg(args, "--smoke-report");
    let adapter = optional_arg(args, "--adapter")
        .map(|adapter| adapter_label(&adapter).to_string())
        .filter(|adapter| adapter != "unknown");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--smoke-report" | "--latest" | "--adapter" | "--out"
            )
    }) {
        return Err(format!(
            "unknown adapter smoke-report evidence option: {unknown}"
        ));
    }
    if optional_arg(args, "--adapter").is_some() && adapter.is_none() {
        return Err("--adapter must be codex or claude".to_string());
    }
    if latest && smoke_report_id.is_some() {
        return Err(
            "adapter smoke-report evidence accepts either --smoke-report or --latest".to_string(),
        );
    }
    if !latest && adapter.is_some() {
        return Err("adapter smoke-report evidence --adapter filter requires --latest".to_string());
    }
    let out = PathBuf::from(required_arg(args, "--out")?);
    let project_id = project_id();
    let command = envelope(
        "adapter-smoke-report-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        smoke_report_id
            .clone()
            .or_else(|| adapter.clone())
            .or_else(|| Some("latest".to_string())),
    );
    let state = state(parsed)?;
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
    let report = if latest {
        dashboard
            .latest_adapter_smoke_report(adapter.as_deref())
            .ok_or_else(|| {
                adapter
                    .as_ref()
                    .map(|adapter| {
                        format!("no recorded adapter smoke reports matching adapter={adapter}")
                    })
                    .unwrap_or_else(|| "no recorded adapter smoke reports".to_string())
            })?
            .clone()
    } else {
        let smoke_report_id = smoke_report_id.ok_or_else(|| {
            "adapter smoke-report evidence requires --smoke-report or --latest".to_string()
        })?;
        dashboard
            .adapter_smoke_report_status(&smoke_report_id)
            .ok_or_else(|| format!("missing adapter smoke report: {smoke_report_id}"))?
            .clone()
    };
    let markdown = render_adapter_smoke_report_evidence(&project_id, &report);
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-adapter-smoke-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_adapter_smoke_report_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "adapter_smoke_evidence".to_string(),
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
                    "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"smoke_report_id\":\"{}\",\"adapter\":\"{}\",\"smoke_status\":\"{}\"}}",
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&report.smoke_report_id),
                    escape_json(&report.adapter_kind),
                    escape_json(&report.smoke_status)
                ),
                idempotency_key: Some(format!(
                    "adapter-smoke-report-evidence:{}:{}:{content_hash}",
                    project_id, report.smoke_report_id
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "adapter_smoke_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: adapter_smoke_report_evidence_confidence(&report),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_smoke_report_evidence_exported=true\nsmoke_report_id={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        report.smoke_report_id,
        path.display(),
        command.command_id
    ))
}

pub(crate) fn scan_adapter_smoke_artifacts(args: &[String]) -> Result<String, String> {
    let artifact_root = required_arg(args, "--artifact-root")?;
    let scan = scan_artifact_root(Path::new(&artifact_root))?;
    Ok(format!(
        "adapter_smoke_artifact_scan=true\ncredential_scan_status=clean\nartifact_root={artifact_root}\nfiles_scanned={}\n",
        scan.files_scanned
    ))
}

pub(crate) fn adapter_smoke_report_evidence_confidence(
    report: &AdapterSmokeReportProjection,
) -> i64 {
    if report.smoke_status == "passed"
        && report.credential_scan_status == "clean"
        && report.marker_found
    {
        85
    } else if report.smoke_status == "failed" || report.credential_scan_status == "blocked" {
        75
    } else {
        65
    }
}

fn render_adapter_smoke_report_evidence(
    project_id: &ProjectId,
    report: &AdapterSmokeReportProjection,
) -> String {
    format!(
        "<!-- capo:adapter-smoke-evidence -->\n# Capo Adapter Smoke Evidence - {}\n\n## Objective\n\nReview a recorded local adapter smoke report without launching subscription-backed provider CLIs or inspecting credential material.\n\n## Smoke Report\n\n- Project: `{}`\n- Smoke report: `{}`\n- Adapter: `{}`\n- Smoke status: `{}`\n- Credential scan status: `{}`\n- Marker found: `{}`\n- Artifact root: `{}`\n- Dogfood readiness effect: `{}`\n- Reason: `{}`\n- Updated sequence: `{}`\n\n## Review Notes\n\n- A passed report only counts toward first real-agent dogfood readiness when the credential scan is clean and the expected marker is present.\n- Skipped and failed reports remain useful evidence because they explain why connector readiness is still blocked.\n- Artifact roots are references only; this report does not render stdout, stderr, prompts, provider output, tokens, cookies, or subscription session material.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo adapter smoke report read models only.\n- It does not launch provider CLIs, materialize prompts, open tunnels, inspect credentials, request approvals, activate grants, or mutate connector state.\n- Credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are not rendered.\n",
        report.smoke_report_id,
        project_id,
        report.smoke_report_id,
        report.adapter_kind,
        report.smoke_status,
        report.credential_scan_status,
        report.marker_found,
        report.artifact_root.as_deref().unwrap_or("none"),
        report.dogfood_readiness_effect,
        report.reason,
        report.updated_sequence
    )
}

fn render_adapter_smoke_report_status(report: &AdapterSmokeReportProjection) -> String {
    format!(
        "adapter_smoke_report_status=true\nsmoke_report_id={}\nadapter={}\nsmoke_status={}\ncredential_scan_status={}\nmarker_found={}\ndogfood_readiness_effect={}\nartifact_root={}\nreason={}\nupdated_sequence={}\nprovider_cli_executed=false credential_material_rendered=false state_mutated=false\n",
        report.smoke_report_id,
        report.adapter_kind,
        report.smoke_status,
        report.credential_scan_status,
        report.marker_found,
        report.dogfood_readiness_effect,
        report.artifact_root.as_deref().unwrap_or("none"),
        report.reason,
        report.updated_sequence
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactScanSummary {
    files_scanned: usize,
}

fn scan_artifact_root(root: &Path) -> Result<ArtifactScanSummary, String> {
    if !root.is_dir() {
        return Err(format!(
            "artifact root does not exist or is not a directory: {}",
            root.display()
        ));
    }
    let files = collect_regular_files(root)?;
    if files.is_empty() {
        return Err(format!(
            "artifact root contains no files: {}",
            root.display()
        ));
    }
    scan_artifacts_for_sensitive_markers(files.iter()).map_err(format_smoke_scan_error)?;
    Ok(ArtifactScanSummary {
        files_scanned: files.len(),
    })
}

fn collect_regular_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to read artifact path {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "artifact scan refuses symlink path: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|error| {
                format!(
                    "failed to read artifact directory {}: {error}",
                    path.display()
                )
            })? {
                let entry = entry.map_err(|error| {
                    format!(
                        "failed to read artifact directory entry {}: {error}",
                        path.display()
                    )
                })?;
                pending.push(entry.path());
            }
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn format_smoke_scan_error(error: LocalAdapterSmokeError) -> String {
    match error {
        LocalAdapterSmokeError::SensitiveArtifact { path, marker } => format!(
            "credential scan blocked artifact {} because marker `{marker}` was not redacted",
            path.display()
        ),
        LocalAdapterSmokeError::Io(error) => {
            format!("credential scan failed to read artifact: {error}")
        }
        LocalAdapterSmokeError::Runtime(error) => {
            format!("credential scan runtime error: {error:?}")
        }
        LocalAdapterSmokeError::NotOptedIn(env) => {
            format!("credential scan unexpectedly hit opt-in gate: {env}")
        }
        LocalAdapterSmokeError::MarkerMissing { marker } => {
            format!("credential scan unexpectedly checked marker: {marker}")
        }
    }
}

fn write_adapter_smoke_report_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:adapter-smoke-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo adapter smoke evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo adapter smoke evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
