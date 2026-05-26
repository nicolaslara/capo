use std::time::{SystemTime, UNIX_EPOCH};

use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_runtime::{
    ChannelKind, ConnectivityEndpointConfig, ConnectivityTunnel, EndpointOwner, ExposureScope,
};
use capo_state::{
    CapabilityGrantProjection, ConnectivityExposureProjection, EventKind, NewEvent,
    PermissionApprovalProjection, ProjectionRecord, RedactionState, SqliteStateStore,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::permission::scope_values;
use crate::{debug_error, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn expose_connectivity_stub(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let endpoint_id = required_arg(args, "--endpoint")?;
    let owner_kind = required_arg(args, "--owner-kind")?;
    let owner_id = required_arg(args, "--owner-id")?;
    let channel = parse_channel_kind(&required_arg(args, "--channel")?)?;
    let exposure = parse_exposure_scope(&required_arg(args, "--exposure")?)?;
    let address_ref = optional_arg(args, "--address").unwrap_or_else(|| owner_id.clone());
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--endpoint"
                    | "--owner-kind"
                    | "--owner-id"
                    | "--channel"
                    | "--exposure"
                    | "--address"
                    | "--record"
            )
    }) {
        return Err(format!(
            "unknown connectivity expose-stub option: {unknown}"
        ));
    }

    let owner = endpoint_owner(&owner_kind, &owner_id)?;
    let tunnel = match exposure {
        ExposureScope::Loopback => ConnectivityTunnel::local_loopback(),
        ExposureScope::Private => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_private(endpoint_id.clone(), address_ref),
        ),
        ExposureScope::Public => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_public(endpoint_id.clone(), address_ref),
        ),
    };
    let resolved = tunnel
        .resolve_endpoint(owner, channel)
        .map_err(|error| format!("connectivity endpoint resolution failed: {error:?}"))?;
    let health = tunnel.check_reachability();
    let status = if resolved.permission_required {
        "blocked_pending_permission"
    } else {
        "active"
    };
    let exposure = ConnectivityExposureProjection {
        exposure_id: format!(
            "connectivity-exposure-{}",
            stable_cli_hash(&format!(
                "{}:{}",
                resolved.resolved_endpoint_id,
                exposure_scope_str(resolved.exposure)
            ))
        ),
        project_id: project_id(),
        connectivity_endpoint_id: resolved.connectivity_endpoint_id.clone(),
        owner_kind: resolved.owner.owner_kind.clone(),
        owner_id: resolved.owner.owner_id.clone(),
        channel_kind: channel_kind_str(resolved.channel_kind).to_string(),
        exposure: exposure_scope_str(resolved.exposure).to_string(),
        permission_scope: resolved.permission_scope.clone(),
        status: status.to_string(),
        capability_grant_id: None,
        health_status: health.status.clone(),
        reachable: health.reachable,
        revoked_at: None,
        updated_sequence: 0,
    };
    let sequence = if record {
        ensure_runtime_target_owner_exists(parsed, &exposure)?;
        let event_kind = if resolved.permission_required {
            EventKind::ConnectivityExposureRequested
        } else {
            EventKind::ConnectivityExposureChanged
        };
        let mut event = NewEvent::new(
            format!(
                "event-connectivity-exposure-{}",
                stable_cli_hash(&exposure.exposure_id)
            ),
            event_kind,
            "capo-cli",
        );
        event.project_id = Some(exposure.project_id.clone());
        event.item_id = Some(exposure.exposure_id.clone());
        event.payload_json = format!(
            "{{\"exposure_id\":\"{}\",\"resolved_endpoint_id\":\"{}\",\"endpoint_id\":\"{}\",\"owner_kind\":\"{}\",\"owner_id\":\"{}\",\"channel\":\"{}\",\"exposure\":\"{}\",\"permission_scope\":\"{}\",\"status\":\"{}\"}}",
            escape_json(&exposure.exposure_id),
            escape_json(&resolved.resolved_endpoint_id),
            escape_json(&exposure.connectivity_endpoint_id),
            escape_json(&exposure.owner_kind),
            escape_json(&exposure.owner_id),
            escape_json(&exposure.channel_kind),
            escape_json(&exposure.exposure),
            escape_json(&exposure.permission_scope),
            escape_json(&exposure.status)
        );
        event.idempotency_key = Some(format!(
            "connectivity-exposure:{}:{}:{}:{}:{}:{}",
            exposure.project_id,
            exposure.connectivity_endpoint_id,
            exposure.owner_kind,
            exposure.owner_id,
            exposure.channel_kind,
            exposure.exposure
        ));
        event.redaction_state = RedactionState::Safe;
        Some(
            state(parsed)?
                .append_event(
                    event,
                    &[ProjectionRecord::ConnectivityExposure(exposure.clone())],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };

    Ok(format!(
        "connectivity_exposure_planned=true\nexposure={}\nendpoint={}\nresolved_endpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_required={}\npermission_scope={}\nstatus={}\nhealth={}\nreachable={}\nrecorded={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        resolved.resolved_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        resolved.permission_required,
        exposure.permission_scope,
        exposure.status,
        exposure.health_status,
        exposure.reachable,
        record,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    ))
}

fn ensure_runtime_target_owner_exists(
    parsed: &ParsedArgs,
    exposure: &ConnectivityExposureProjection,
) -> Result<(), String> {
    if exposure.owner_kind != "runtime_target" {
        return Ok(());
    }
    let target = state(parsed)?
        .runtime_targets(&exposure.project_id)
        .map_err(debug_error)?
        .into_iter()
        .find(|target| target.runtime_target_id == exposure.owner_id);
    let Some(target) = target else {
        return Err(format!(
            "unknown runtime target for recorded connectivity exposure: {}; register it with `capo runtime target register` first",
            exposure.owner_id
        ));
    };
    if target.status != "available" {
        return Err(format!(
            "runtime target is not available for recorded connectivity exposure: target={} status={}",
            exposure.owner_id, target.status
        ));
    }
    if let Some(expected_endpoint) = &target.connectivity_endpoint_id
        && expected_endpoint != &exposure.connectivity_endpoint_id
    {
        Err(format!(
            "runtime target endpoint mismatch for recorded connectivity exposure: target={} registered_endpoint={} requested_endpoint={}",
            exposure.owner_id, expected_endpoint, exposure.connectivity_endpoint_id
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn request_connectivity_exposure_approval(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    let approval_id = optional_arg(args, "--approval").unwrap_or_else(|| {
        format!(
            "approval-connectivity-exposure-{}",
            stable_cli_hash(&exposure_id)
        )
    });
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure" | "--approval"))
    {
        return Err(format!(
            "unknown connectivity request-approval option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status != "blocked_pending_permission" {
        return Err(format!(
            "connectivity exposure is not awaiting permission: {} status={}",
            exposure.exposure_id, exposure.status
        ));
    }
    if state
        .permission_approval(&project_id(), &approval_id)
        .map_err(debug_error)?
        .is_some()
    {
        return Err(format!("approval already exists: {approval_id}"));
    }
    let scope_json = connectivity_exposure_scope_json(&exposure);
    let subject_json = connectivity_exposure_subject_json(&exposure);
    let approval = PermissionApprovalProjection {
        approval_id: approval_id.clone(),
        project_id: project_id(),
        session_id: None,
        tool_call_id: None,
        capability_profile_id: "remote-control-reviewed".to_string(),
        scope_json,
        subject_json,
        status: "pending".to_string(),
        requested_by: "local-user".to_string(),
        reason: format!("approve connectivity exposure {}", exposure.exposure_id),
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-approval-{}",
            stable_cli_hash(&approval.approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-cli",
    );
    event.project_id = Some(project_id());
    event.item_id = Some(exposure.exposure_id.clone());
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"exposure_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"reason\":\"{}\"}}",
        escape_json(&approval.approval_id),
        escape_json(&exposure.exposure_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-approval:{}:{}:{}",
        exposure.project_id, exposure.exposure_id, approval.approval_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "connectivity_exposure_approval_requested=true\nexposure={}\napproval={}\nstatus=pending\npermission_scope={}\nsequence={sequence}\n",
        exposure.exposure_id, approval.approval_id, exposure.permission_scope
    ))
}

pub(crate) fn activate_connectivity_exposure(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure"))
    {
        return Err(format!(
            "unknown connectivity activate-exposure option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status == "revoked" {
        return Err(format!("connectivity exposure is revoked: {exposure_id}"));
    }
    if exposure.status == "active" {
        return Ok(render_connectivity_exposure_activation(
            &exposure,
            exposure.capability_grant_id.as_deref().unwrap_or("none"),
            None,
        ));
    }
    if exposure.status != "blocked_pending_permission" {
        return Err(format!(
            "connectivity exposure is not activatable: {} status={}",
            exposure.exposure_id, exposure.status
        ));
    }
    let grant = matching_connectivity_exposure_grant(&state, &exposure)?;
    let active = ConnectivityExposureProjection {
        status: "active".to_string(),
        capability_grant_id: Some(grant.capability_grant_id.clone()),
        health_status: if exposure.health_status == "unknown" {
            "available".to_string()
        } else {
            exposure.health_status.clone()
        },
        reachable: exposure.reachable,
        revoked_at: None,
        updated_sequence: 0,
        ..exposure.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-activated-{}",
            stable_cli_hash(&format!(
                "{}:{}",
                active.exposure_id, grant.capability_grant_id
            ))
        ),
        EventKind::ConnectivityExposureChanged,
        "capo-cli",
    );
    event.project_id = Some(active.project_id.clone());
    event.item_id = Some(active.exposure_id.clone());
    event.payload_json = format!(
        "{{\"exposure_id\":\"{}\",\"capability_grant_id\":\"{}\",\"status\":\"active\",\"permission_scope\":\"{}\"}}",
        escape_json(&active.exposure_id),
        escape_json(&grant.capability_grant_id),
        escape_json(&active.permission_scope)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-activate:{}:{}:{}",
        active.project_id, active.exposure_id, grant.capability_grant_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::ConnectivityExposure(active.clone())],
        )
        .map_err(debug_error)?;
    Ok(render_connectivity_exposure_activation(
        &active,
        &grant.capability_grant_id,
        Some(sequence),
    ))
}

pub(crate) fn revoke_connectivity_exposure(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    let reason = optional_arg(args, "--reason").unwrap_or_else(|| "operator_revoked".to_string());
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure" | "--reason"))
    {
        return Err(format!(
            "unknown connectivity revoke-exposure option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status == "revoked" {
        return Ok(render_connectivity_exposure_revocation(
            &exposure, &reason, None,
        ));
    }
    let revoked_at = unix_timestamp_label()?;
    let revoked = ConnectivityExposureProjection {
        status: "revoked".to_string(),
        health_status: "disabled".to_string(),
        reachable: false,
        revoked_at: Some(revoked_at.clone()),
        updated_sequence: 0,
        ..exposure.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-revoked-{}",
            stable_cli_hash(&revoked.exposure_id)
        ),
        EventKind::ConnectivityExposureRevoked,
        "capo-cli",
    );
    event.project_id = Some(revoked.project_id.clone());
    event.item_id = Some(revoked.exposure_id.clone());
    event.payload_json = format!(
        "{{\"exposure_id\":\"{}\",\"status\":\"revoked\",\"reason\":\"{}\",\"revoked_at\":\"{}\"}}",
        escape_json(&revoked.exposure_id),
        escape_json(&reason),
        escape_json(&revoked_at)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-revoke:{}:{}",
        revoked.project_id, revoked.exposure_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::ConnectivityExposure(revoked.clone())],
        )
        .map_err(debug_error)?;
    Ok(render_connectivity_exposure_revocation(
        &revoked,
        &reason,
        Some(sequence),
    ))
}

pub(crate) fn connectivity_exposure_status(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let exposure_id = optional_arg(args, "--exposure");
    let owner_kind = optional_arg(args, "--owner-kind");
    let owner_id = optional_arg(args, "--owner-id");
    let channel = optional_arg(args, "--channel");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--exposure" | "--latest" | "--owner-kind" | "--owner-id" | "--channel"
            )
    }) {
        return Err(format!(
            "unknown connectivity exposure-status option: {unknown}"
        ));
    }
    if latest && exposure_id.is_some() {
        return Err(
            "connectivity exposure-status accepts either --exposure or --latest".to_string(),
        );
    }
    if !latest && (owner_kind.is_some() || owner_id.is_some() || channel.is_some()) {
        return Err("connectivity exposure-status filters require --latest".to_string());
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
    } else {
        let exposure_id = exposure_id.ok_or_else(|| {
            "connectivity exposure-status requires --exposure or --latest".to_string()
        })?;
        dashboard
            .connectivity_exposure_status(&exposure_id)
            .ok_or_else(|| format!("missing connectivity exposure: {exposure_id}"))?
    };

    Ok(render_connectivity_exposure_status(exposure))
}

fn render_connectivity_exposure_status(exposure: &ConnectivityExposureProjection) -> String {
    format!(
        "connectivity_exposure_status=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrevoked_at={}\nupdated_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.health_status,
        exposure.reachable,
        exposure.revoked_at.as_deref().unwrap_or("none"),
        exposure.updated_sequence
    )
}

fn render_connectivity_exposure_activation(
    exposure: &ConnectivityExposureProjection,
    grant_id: &str,
    sequence: Option<i64>,
) -> String {
    format!(
        "connectivity_exposure_activated=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        grant_id,
        exposure.health_status,
        exposure.reachable,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_connectivity_exposure_revocation(
    exposure: &ConnectivityExposureProjection,
    reason: &str,
    sequence: Option<i64>,
) -> String {
    format!(
        "connectivity_exposure_revoked=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrevoked_at={}\nreason={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.health_status,
        exposure.reachable,
        exposure.revoked_at.as_deref().unwrap_or("none"),
        reason,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn connectivity_exposure(
    state: &SqliteStateStore,
    exposure_id: &str,
) -> Result<ConnectivityExposureProjection, String> {
    state
        .connectivity_exposures(&project_id())
        .map_err(debug_error)?
        .into_iter()
        .rev()
        .find(|exposure| exposure.exposure_id == exposure_id)
        .ok_or_else(|| format!("missing connectivity exposure: {exposure_id}"))
}

fn matching_connectivity_exposure_grant(
    state: &SqliteStateStore,
    exposure: &ConnectivityExposureProjection,
) -> Result<CapabilityGrantProjection, String> {
    let expected_subject = connectivity_exposure_subject_value(exposure);
    state
        .capability_grants()
        .map_err(debug_error)?
        .into_iter()
        .rev()
        .find(|grant| {
            grant.effect == "allow"
                && scope_values(&grant.scope_json)
                    .map(|scopes| {
                        scopes
                            .iter()
                            .any(|scope| scope == &exposure.permission_scope)
                    })
                    .unwrap_or(false)
                && subject_contains(&grant.subject_json, &expected_subject)
        })
        .ok_or_else(|| {
            format!(
                "missing allow grant for connectivity exposure {} scope={}",
                exposure.exposure_id, exposure.permission_scope
            )
        })
}

fn connectivity_exposure_scope_json(exposure: &ConnectivityExposureProjection) -> String {
    format!("[\"{}\"]", escape_json(&exposure.permission_scope))
}

fn connectivity_exposure_subject_json(exposure: &ConnectivityExposureProjection) -> String {
    connectivity_exposure_subject_value(exposure).to_string()
}

fn connectivity_exposure_subject_value(
    exposure: &ConnectivityExposureProjection,
) -> serde_json::Value {
    serde_json::json!({
        "exposure_id": exposure.exposure_id,
        "endpoint_id": exposure.connectivity_endpoint_id,
        "owner_kind": exposure.owner_kind,
        "owner_id": exposure.owner_id,
        "channel": exposure.channel_kind,
        "exposure": exposure.exposure,
    })
}

fn subject_contains(subject_json: &str, expected: &serde_json::Value) -> bool {
    let Ok(serde_json::Value::Object(subject)) =
        serde_json::from_str::<serde_json::Value>(subject_json)
    else {
        return false;
    };
    let Some(expected) = expected.as_object() else {
        return false;
    };
    expected
        .iter()
        .all(|(key, value)| subject.get(key) == Some(value))
}

fn unix_timestamp_label() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system time before unix epoch: {error}"))?
        .as_secs();
    Ok(format!("unix:{seconds}"))
}

pub(crate) fn parse_channel_kind(value: &str) -> Result<ChannelKind, String> {
    match value {
        "control" => Ok(ChannelKind::Control),
        "stdio" => Ok(ChannelKind::Stdio),
        "logs" => Ok(ChannelKind::Logs),
        "dashboard" => Ok(ChannelKind::Dashboard),
        "artifact" => Ok(ChannelKind::Artifact),
        other => Err(format!("unsupported channel kind: {other}")),
    }
}

fn channel_kind_str(value: ChannelKind) -> &'static str {
    match value {
        ChannelKind::Control => "control",
        ChannelKind::Stdio => "stdio",
        ChannelKind::Logs => "logs",
        ChannelKind::Dashboard => "dashboard",
        ChannelKind::Artifact => "artifact",
    }
}

fn parse_exposure_scope(value: &str) -> Result<ExposureScope, String> {
    match value {
        "loopback" => Ok(ExposureScope::Loopback),
        "private" => Ok(ExposureScope::Private),
        "public" => Ok(ExposureScope::Public),
        other => Err(format!("unsupported exposure scope: {other}")),
    }
}

fn exposure_scope_str(value: ExposureScope) -> &'static str {
    match value {
        ExposureScope::Loopback => "loopback",
        ExposureScope::Private => "private",
        ExposureScope::Public => "public",
    }
}

pub(crate) fn endpoint_owner(owner_kind: &str, owner_id: &str) -> Result<EndpointOwner, String> {
    match owner_kind {
        "runtime_target" => Ok(EndpointOwner::runtime_target(owner_id)),
        "capo_server" => Ok(EndpointOwner::capo_server(owner_id)),
        other => Err(format!("unsupported endpoint owner kind: {other}")),
    }
}
