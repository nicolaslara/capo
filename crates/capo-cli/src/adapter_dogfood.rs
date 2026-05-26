use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, ProjectId};
use capo_query::{AdapterDogfoodGate, ProjectDashboardQuery, project_dashboard};
use capo_state::{
    AdapterSmokeReportProjection, ArtifactRecord, EventKind, EvidenceProjection, NewEvent,
    ProjectionRecord, RedactionState,
};

use crate::cli_surface::{ParsedArgs, required_arg};
use crate::{
    comma_or_none, debug_error, envelope, escape_json, project_id, stable_cli_hash, state,
};

pub(crate) fn adapter_dogfood_gate(parsed: &ParsedArgs) -> Result<String, String> {
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    Ok(render_adapter_dogfood_gate(&dashboard.adapter_dogfood_gate))
}

pub(crate) fn adapter_dogfood_gate_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--out"))
    {
        return Err(format!(
            "unknown adapter dogfood-gate evidence option: {unknown}"
        ));
    }
    let out = PathBuf::from(required_arg(args, "--out")?);
    let state = state(parsed)?;
    let project_id = project_id();
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
    let gate = &dashboard.adapter_dogfood_gate;
    let command = envelope(
        "adapter-dogfood-gate-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        Some(gate.status.clone()),
    );
    let markdown =
        render_adapter_dogfood_gate_evidence(&project_id, gate, &dashboard.adapter_smoke_reports);
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-adapter-dogfood-gate-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_adapter_dogfood_gate_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "adapter_dogfood_gate_evidence".to_string(),
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
                    gate.ready,
                    escape_json(&gate.status)
                ),
                idempotency_key: Some(format!(
                    "adapter-dogfood-gate-evidence:{}:{}:{content_hash}",
                    project_id, gate.status
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "adapter_dogfood_gate_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: adapter_dogfood_gate_evidence_confidence(gate),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_dogfood_gate_evidence_exported=true\nready_for_first_real_agent_dogfood={}\nstatus={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\nprovider_cli_executed=false credential_material_rendered=false\n",
        gate.ready,
        gate.status,
        path.display(),
        command.command_id
    ))
}

pub(crate) fn render_adapter_dogfood_gate(gate: &AdapterDogfoodGate) -> String {
    format!(
        "adapter_dogfood_gate=true\nready_for_first_real_agent_dogfood={}\nstatus={}\nrequired_adapters={}\nproven_adapters={}\nblocked_adapters={}\nreasons={}\n",
        gate.ready,
        gate.status,
        comma_or_none(&gate.required_adapters),
        comma_or_none(&gate.proven_adapters),
        comma_or_none(&gate.blocked_adapters),
        comma_or_none(&gate.reasons)
    )
}

fn adapter_dogfood_gate_evidence_confidence(gate: &AdapterDogfoodGate) -> i64 {
    if gate.ready { 85 } else { 65 }
}

pub(crate) fn render_adapter_dogfood_gate_evidence(
    project_id: &ProjectId,
    gate: &AdapterDogfoodGate,
    smoke_reports: &[AdapterSmokeReportProjection],
) -> String {
    let smoke_report_refs = smoke_reports
        .iter()
        .map(|report| {
            format!(
                "{}:{}:{}:{}",
                report.smoke_report_id,
                report.adapter_kind,
                report.smoke_status,
                report.credential_scan_status
            )
        })
        .collect::<Vec<_>>();
    format!(
        "<!-- capo:adapter-dogfood-gate-evidence -->\n# Capo Adapter Dogfood Gate Evidence - {}\n\n## Objective\n\nReview whether recorded connector evidence is sufficient for first real-agent dogfood, without launching subscription-backed provider CLIs or inspecting credential material.\n\n## Gate\n\n- Project: `{}`\n- Ready for first real-agent dogfood: `{}`\n- Status: `{}`\n- Required adapters: `{}`\n- Proven adapters: `{}`\n- Blocked adapters: `{}`\n- Reasons: `{}`\n\n## Connector Evidence Refs\n\n- Smoke report refs: `{}`\n\n## Review Notes\n\n- The first dogfood gate requires a passed Codex smoke report with a clean credential scan, the expected marker present, and `real_agent_connector_proven` as its readiness effect.\n- A blocked gate is still useful evidence because it identifies which connector proof is missing before Capo can dogfood with real local agents.\n- Smoke report refs are metadata only; this report does not render stdout, stderr, prompts, provider output, tokens, cookies, or subscription session material.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo adapter smoke report read models only.\n- It does not launch provider CLIs, materialize prompts, open tunnels, inspect credentials, request approvals, activate grants, or mutate connector state.\n- Credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are not rendered.\n",
        gate.status,
        project_id,
        gate.ready,
        gate.status,
        comma_or_none(&gate.required_adapters),
        comma_or_none(&gate.proven_adapters),
        comma_or_none(&gate.blocked_adapters),
        comma_or_none(&gate.reasons),
        comma_or_none(&smoke_report_refs)
    )
}

fn write_adapter_dogfood_gate_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:adapter-dogfood-gate-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo adapter dogfood gate evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo adapter dogfood gate evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
