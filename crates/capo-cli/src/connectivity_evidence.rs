use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, ProjectId};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{
    ArtifactRecord, ConnectivityExposureProjection, EventKind, EvidenceProjection, NewEvent,
    ProjectionRecord, RedactionState,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::connectivity::{endpoint_owner, parse_channel_kind};
use crate::{debug_error, envelope, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn connectivity_exposure_evidence(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let exposure_id = optional_arg(args, "--exposure");
    let owner_kind = optional_arg(args, "--owner-kind");
    let owner_id = optional_arg(args, "--owner-id");
    let channel = optional_arg(args, "--channel");
    let out = PathBuf::from(required_arg(args, "--out")?);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--exposure" | "--latest" | "--owner-kind" | "--owner-id" | "--channel" | "--out"
            )
    }) {
        return Err(format!(
            "unknown connectivity exposure-evidence option: {unknown}"
        ));
    }
    if latest && exposure_id.is_some() {
        return Err(
            "connectivity exposure-evidence accepts either --exposure or --latest".to_string(),
        );
    }
    if !latest && (owner_kind.is_some() || owner_id.is_some() || channel.is_some()) {
        return Err("connectivity exposure-evidence filters require --latest".to_string());
    }
    if let Some(kind) = owner_kind.as_deref() {
        endpoint_owner(kind, owner_id.as_deref().unwrap_or("filter-validation"))?;
    }
    if let Some(channel) = channel.as_deref() {
        parse_channel_kind(channel)?;
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let exposure = if latest {
        dashboard
            .latest_connectivity_exposure(
                owner_kind.as_deref(),
                owner_id.as_deref(),
                channel.as_deref(),
            )
            .ok_or_else(|| {
                let mut filters = Vec::new();
                if let Some(owner_kind) = owner_kind.as_deref() {
                    filters.push(format!("owner_kind={owner_kind}"));
                }
                if let Some(owner_id) = owner_id.as_deref() {
                    filters.push(format!("owner_id={owner_id}"));
                }
                if let Some(channel) = channel.as_deref() {
                    filters.push(format!("channel={channel}"));
                }
                if filters.is_empty() {
                    "no recorded connectivity exposures".to_string()
                } else {
                    format!(
                        "no recorded connectivity exposures matching {}",
                        filters.join(",")
                    )
                }
            })?
            .clone()
    } else {
        let exposure_id = exposure_id.ok_or_else(|| {
            "connectivity exposure-evidence requires --exposure or --latest".to_string()
        })?;
        dashboard
            .connectivity_exposure_status(&exposure_id)
            .ok_or_else(|| format!("missing connectivity exposure: {exposure_id}"))?
            .clone()
    };
    let project_id = project_id();
    let command = envelope(
        "connectivity-exposure-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        Some(exposure.exposure_id.clone()),
    );
    let markdown = render_connectivity_exposure_evidence(&project_id, &exposure);
    // CT2 emitted-surface guard: the evidence artifact is one of the surfaces the
    // redaction guard covers. Before retaining it, scan for any leaked credential
    // pattern so the `RedactionState::Safe` marker on the artifact + event is
    // earned, not asserted. A handle field that ever carried a raw credential
    // already failed closed at expose-stub time; this is the defense-in-depth net.
    if let Err(pattern) = capo_state::assert_connectivity_event_safe(&markdown) {
        return Err(format!(
            "connectivity exposure-evidence artifact leaked a `{pattern}` credential pattern; refusing to retain"
        ));
    }
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-connectivity-exposure-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_connectivity_exposure_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "connectivity_exposure_evidence".to_string(),
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
                    "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"exposure_id\":\"{}\",\"status\":\"{}\"}}",
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&exposure.exposure_id),
                    escape_json(&exposure.status)
                ),
                idempotency_key: Some(format!(
                    "connectivity-exposure-evidence:{}:{}:{content_hash}",
                    project_id, exposure.exposure_id
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "connectivity_exposure_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: connectivity_exposure_evidence_confidence(&exposure),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "connectivity_exposure_evidence_exported=true\nexposure={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        exposure.exposure_id,
        path.display(),
        command.command_id
    ))
}

pub(crate) fn connectivity_exposure_evidence_confidence(
    exposure: &ConnectivityExposureProjection,
) -> i64 {
    if exposure.status == "active" && exposure.capability_grant_id.is_some() {
        85
    } else if exposure.status == "revoked" {
        80
    } else {
        65
    }
}

fn render_connectivity_exposure_evidence(
    project_id: &ProjectId,
    exposure: &ConnectivityExposureProjection,
) -> String {
    format!(
        "<!-- capo:connectivity-exposure-evidence -->\n# Capo Connectivity Exposure Evidence - {}\n\n## Objective\n\nReview a recorded connectivity exposure without opening tunnels or touching runtime/provider processes.\n\n## Exposure\n\n- Project: `{}`\n- Exposure: `{}`\n- Endpoint: `{}`\n- Owner: `{}:{}`\n- Channel: `{}`\n- Exposure scope: `{}`\n- Permission scope: `{}`\n- Status: `{}`\n- Health: `{}`\n- Reachable: `{}`\n- Linked grant: `{}`\n- Revoked at: `{}`\n- Auth mode: `{}`\n- Auth handle ref: `{}`\n- Identity handle ref: `{}`\n- Identity fingerprint: `{}`\n- Expires at: `{}`\n- Updated sequence: `{}`\n\n## Review Notes\n\n- Active exposure requires a matching durable allow grant before the exposure state can become active.\n- Revocation disables the exposure read model and marks it unreachable while preserving historical grant evidence.\n- This artifact records Capo connectivity metadata only; it is not proof of real tunnel reachability.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo connectivity read models only.\n- It does not open tunnels, launch runtimes, launch provider CLIs, inspect credentials, materialize prompts, or mutate exposure state.\n- The auth/identity values above are OPAQUE HANDLES and a derived fingerprint only (CT2); credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are never rendered.\n",
        exposure.exposure_id,
        project_id,
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
        exposure.revoked_at.as_deref().unwrap_or("none"),
        if exposure.auth_ref.is_some() {
            "auth_ref_handle"
        } else {
            "none"
        },
        exposure.auth_ref.as_deref().unwrap_or("none"),
        exposure.identity_ref.as_deref().unwrap_or("none"),
        exposure.identity_fingerprint.as_deref().unwrap_or("none"),
        exposure.expires_at.as_deref().unwrap_or("none"),
        exposure.updated_sequence
    )
}

fn write_connectivity_exposure_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:connectivity-exposure-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo connectivity exposure evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo connectivity exposure evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
