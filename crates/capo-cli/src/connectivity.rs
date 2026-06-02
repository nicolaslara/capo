use std::time::{SystemTime, UNIX_EPOCH};

use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_runtime::{
    ChannelKind, ConnectivityEndpointConfig, ConnectivityError, ConnectivityTunnel, EndpointOwner,
    ExposurePolicy, ExposureScope, FakeTunnelScript,
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
    // CT2: opaque credential/identity HANDLES (never raw credentials). Empty flags
    // normalize to absent so a blank `--auth-ref ""` does not masquerade as present.
    let auth_ref = optional_arg(args, "--auth-ref").filter(|value| !value.is_empty());
    let identity_ref = optional_arg(args, "--identity-ref").filter(|value| !value.is_empty());
    let record = has_flag(args, "--record");
    // CT4: a DETERMINISTIC test/seam flag. When present on a `private` exposure it
    // routes resolution through a scripted `FakeTunnel` whose OBSERVED device id is
    // this value, exercising the SAME identity-verification path the live
    // `TailscaleTunnel` uses (the fake carries parity at the enum surface). With no
    // `--identity-ref` the observed device is trusted by ACL; with one, a mismatch
    // yields a typed `IdentityMismatch` that is recorded as a BLOCKED exposure event
    // (never a silent connect). It has NO effect on loopback/public scopes.
    let fake_observed_device =
        optional_arg(args, "--fake-observed-device").filter(|value| !value.is_empty());
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
                    | "--auth-ref"
                    | "--identity-ref"
                    | "--record"
                    | "--fake-observed-device"
            )
    }) {
        return Err(format!(
            "unknown connectivity expose-stub option: {unknown}"
        ));
    }

    // CT2 redaction guard (SECONDARY net, fail-closed on handle fields): refuse to
    // proceed if a raw-credential-looking value was passed into a HANDLE field. A
    // raw value in a handle field is a BUG, not something to silently scrub.
    capo_state::guard_connectivity_handles(&capo_state::ConnectivityHandles {
        auth_ref: auth_ref.as_deref(),
        identity_ref: identity_ref.as_deref(),
        identity_fingerprint: None,
    })
    .map_err(|error| format!("connectivity handle redaction guard failed closed: {error}"))?;

    let owner = endpoint_owner(&owner_kind, &owner_id)?;
    let tunnel = match (exposure, fake_observed_device.as_deref()) {
        (ExposureScope::Loopback, _) => ConnectivityTunnel::local_loopback(),
        // CT4 deterministic seam: a `private` exposure with `--fake-observed-device`
        // resolves through a scripted FakeTunnel carrying the SAME identity-check
        // surface as the live Tailscale adapter, so the CLI exercises the
        // identity-match / identity-mismatch paths with no live tailnet.
        (ExposureScope::Private, Some(observed)) => {
            let script = FakeTunnelScript::private_matching(endpoint_id.clone(), observed)
                .with_expected_identity_ref(identity_ref.clone());
            ConnectivityTunnel::fake_scripted(script)
        }
        (ExposureScope::Private, None) => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_private(endpoint_id.clone(), address_ref)
                .with_handles(auth_ref.clone(), identity_ref.clone()),
        ),
        (ExposureScope::Public, _) => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_public(endpoint_id.clone(), address_ref)
                .with_handles(auth_ref.clone(), identity_ref.clone()),
        ),
    };
    let resolved = match tunnel.resolve_endpoint(owner.clone(), channel) {
        Ok(resolved) => resolved,
        // CT4: an identity mismatch is NOT a silent failure. It is an AUDITABLE
        // blocked exposure: when `--record`, append a `ConnectivityExposureRequested`
        // event (status `blocked_pending_permission`) carrying the FINGERPRINTS only
        // (never a raw credential), then surface the typed refusal to the caller.
        Err(error @ ConnectivityError::IdentityMismatch { .. }) => {
            let recorded_sequence = if record {
                Some(record_blocked_identity_mismatch(
                    parsed,
                    &endpoint_id,
                    &owner,
                    channel,
                    &error,
                )?)
            } else {
                None
            };
            return Err(format!(
                "connectivity endpoint resolution failed: {error}\nstatus=blocked_pending_permission\nrecorded={record}\nrecorded_sequence={}",
                recorded_sequence
                    .map(|sequence| sequence.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
        }
        Err(error) => {
            return Err(format!(
                "connectivity endpoint resolution failed: {error:?}"
            ));
        }
    };
    // CT1: route the exposure through the explicit `ExposurePolicy`. With no
    // opt-in promotion and no `auth_ref` handle (CT2 adds the handle), the
    // default loopback-only policy authorizes loopback (no permission required)
    // and fails CLOSED for `private`/`public` with a typed `AuthRequired`
    // refusal. That refusal is representable as a blocked exposure, not a silent
    // allow: a `private`/`public` stub therefore stays
    // `blocked_pending_permission` and cannot reach `active` until a grant is
    // present (the grant still gates activation independently). The policy gate
    // and the grant gate are two separate, both-required checks.
    let policy = ExposurePolicy::loopback_default();
    // A typed refusal (no auth handle / above ceiling) maps to "permission
    // required" so the exposure stays blocked-pending-permission rather than a
    // silent allow; loopback maps to false. Capture the TYPED reason so the
    // surfaced status is provably the POLICY gate (AuthRequired/ScopeExceedsCeiling)
    // and not merely the downstream grant gate — this is what distinguishes the
    // CT1 policy block from the pre-CT1 grant block.
    let policy_decision = policy.authorize(resolved.exposure, auth_ref.as_deref());
    let (permission_required, policy_block_reason) = match &policy_decision {
        Ok(required) => (*required, None),
        Err(error @ ConnectivityError::AuthRequired { .. }) => {
            (true, Some(format!("AuthRequired: {error}")))
        }
        Err(error @ ConnectivityError::ScopeExceedsCeiling { .. }) => {
            (true, Some(format!("ScopeExceedsCeiling: {error}")))
        }
        // Any other typed refusal still fails closed to permission-required.
        Err(error) => (true, Some(format!("{error}"))),
    };
    let status = if permission_required {
        "blocked_pending_permission"
    } else {
        "active"
    };
    let health = tunnel.check_reachability();
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
        // CT2: opaque handles + derived audit fields. The handles came from the
        // CLI flags (already guarded fail-closed above); the fingerprint/expiry
        // come off the resolved endpoint (None for the stub today, populated by
        // CT4/CT8). All are pointers/derived values, never raw credentials.
        auth_ref: auth_ref.clone(),
        identity_ref: identity_ref.clone(),
        identity_fingerprint: resolved.identity_fingerprint.clone(),
        expires_at: resolved.expires_at.clone(),
        updated_sequence: 0,
    };
    let sequence = if record {
        ensure_runtime_target_owner_exists(parsed, &exposure)?;
        let event_kind = if permission_required {
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
        // CT2: record auth MODE + the opaque handles only — never the resolved
        // credential. `auth_mode` mirrors the protocol-provider "record auth mode
        // only" rule; the handle/identity refs are opaque pointers (already guarded
        // fail-closed). `null` when absent so the payload stays replay-stable.
        let auth_mode = if exposure.auth_ref.is_some() {
            "auth_ref_handle"
        } else {
            "none"
        };
        event.payload_json = format!(
            "{{\"exposure_id\":\"{}\",\"resolved_endpoint_id\":\"{}\",\"endpoint_id\":\"{}\",\"owner_kind\":\"{}\",\"owner_id\":\"{}\",\"channel\":\"{}\",\"exposure\":\"{}\",\"permission_scope\":\"{}\",\"status\":\"{}\",\"auth_mode\":\"{}\",\"auth_ref\":{},\"identity_ref\":{},\"identity_fingerprint\":{},\"expires_at\":{}}}",
            escape_json(&exposure.exposure_id),
            escape_json(&resolved.resolved_endpoint_id),
            escape_json(&exposure.connectivity_endpoint_id),
            escape_json(&exposure.owner_kind),
            escape_json(&exposure.owner_id),
            escape_json(&exposure.channel_kind),
            escape_json(&exposure.exposure),
            escape_json(&exposure.permission_scope),
            escape_json(&exposure.status),
            auth_mode,
            json_opt_string(exposure.auth_ref.as_deref()),
            json_opt_string(exposure.identity_ref.as_deref()),
            json_opt_string(exposure.identity_fingerprint.as_deref()),
            json_opt_string(exposure.expires_at.as_deref()),
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
        // CT2 emitted-surface guard: make the `Safe` marker MEAN something by
        // scanning the payload for any leaked credential pattern before persisting.
        if let Err(pattern) = capo_state::assert_connectivity_event_safe(&event.payload_json) {
            return Err(format!(
                "connectivity event payload marked Safe but leaked a `{pattern}` credential pattern; refusing to persist"
            ));
        }
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
        "connectivity_exposure_planned=true\nexposure={}\nendpoint={}\nresolved_endpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_required={}\npermission_scope={}\nstatus={}\npolicy_block_reason={}\nhealth={}\nreachable={}\nrecorded={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        resolved.resolved_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        permission_required,
        exposure.permission_scope,
        exposure.status,
        policy_block_reason.as_deref().unwrap_or("none"),
        exposure.health_status,
        exposure.reachable,
        record,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    ))
}

/// CT4: record an identity-mismatch refusal as an AUDITABLE blocked exposure.
///
/// A `TailscaleTunnel`/scripted `FakeTunnel` `IdentityMismatch` must never be a
/// SILENT failure (invisible in the audit log). When `--record` is set, this
/// appends a `ConnectivityExposureRequested` event with `status =
/// blocked_pending_permission` and a payload carrying the EXPECTED/OBSERVED device
/// FINGERPRINTS only (the CT2 `tsnode:` contract — never a raw credential), so an
/// operator can reconstruct WHY the exposure was blocked. The payload is scanned by
/// the CT2 emitted-surface guard before persistence.
fn record_blocked_identity_mismatch(
    parsed: &ParsedArgs,
    endpoint_id: &str,
    owner: &EndpointOwner,
    channel: ChannelKind,
    error: &ConnectivityError,
) -> Result<i64, String> {
    let ConnectivityError::IdentityMismatch {
        expected, observed, ..
    } = error
    else {
        return Err(
            "record_blocked_identity_mismatch called with a non-mismatch error".to_string(),
        );
    };
    // The mismatch is a `private` tunnel refusal: scope + permission-scope are the
    // private-tunnel ones; the exposure stays blocked pending permission.
    let exposure_scope = ExposureScope::Private;
    let permission_scope = exposure_scope.permission_scope().to_string();
    let exposure = ConnectivityExposureProjection {
        exposure_id: format!(
            "connectivity-exposure-{}",
            stable_cli_hash(&format!(
                "{}:{}",
                endpoint_id,
                exposure_scope_str(exposure_scope)
            ))
        ),
        project_id: project_id(),
        connectivity_endpoint_id: endpoint_id.to_string(),
        owner_kind: owner.owner_kind.clone(),
        owner_id: owner.owner_id.clone(),
        channel_kind: channel_kind_str(channel).to_string(),
        exposure: exposure_scope_str(exposure_scope).to_string(),
        permission_scope: permission_scope.clone(),
        status: "blocked_pending_permission".to_string(),
        capability_grant_id: None,
        health_status: "unreachable".to_string(),
        reachable: false,
        revoked_at: None,
        // CT2/CT4: NO raw credential. The identity_fingerprint records the OBSERVED
        // device fingerprint so the audit trail shows which device was refused.
        auth_ref: None,
        identity_ref: None,
        identity_fingerprint: Some(observed.clone()),
        expires_at: None,
        updated_sequence: 0,
    };
    ensure_runtime_target_owner_exists(parsed, &exposure)?;
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-{}",
            stable_cli_hash(&exposure.exposure_id)
        ),
        EventKind::ConnectivityExposureRequested,
        "capo-cli",
    );
    event.project_id = Some(exposure.project_id.clone());
    event.item_id = Some(exposure.exposure_id.clone());
    event.payload_json = format!(
        "{{\"exposure_id\":\"{}\",\"endpoint_id\":\"{}\",\"owner_kind\":\"{}\",\"owner_id\":\"{}\",\"channel\":\"{}\",\"exposure\":\"{}\",\"permission_scope\":\"{}\",\"status\":\"blocked_pending_permission\",\"block_reason\":\"identity_mismatch\",\"expected_identity_fingerprint\":\"{}\",\"observed_identity_fingerprint\":\"{}\"}}",
        escape_json(&exposure.exposure_id),
        escape_json(&exposure.connectivity_endpoint_id),
        escape_json(&exposure.owner_kind),
        escape_json(&exposure.owner_id),
        escape_json(&exposure.channel_kind),
        escape_json(&exposure.exposure),
        escape_json(&exposure.permission_scope),
        escape_json(expected),
        escape_json(observed),
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-identity-mismatch:{}:{}:{}:{}:{}",
        exposure.project_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind
    ));
    // CT2 emitted-surface guard: the payload carries fingerprints only, but make the
    // `Safe` marker MEAN something by scanning for any leaked credential pattern.
    if let Err(pattern) = capo_state::assert_connectivity_event_safe(&event.payload_json) {
        return Err(format!(
            "connectivity event payload marked Safe but leaked a `{pattern}` credential pattern; refusing to persist"
        ));
    }
    event.redaction_state = RedactionState::Safe;
    state(parsed)?
        .append_event(
            event,
            &[ProjectionRecord::ConnectivityExposure(exposure.clone())],
        )
        .map_err(debug_error)
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
    let raw_reason =
        optional_arg(args, "--reason").unwrap_or_else(|| "operator_revoked".to_string());
    // CT2 FREE-TEXT rule: `--reason` is operator-supplied free text and the only
    // free-text vector on any connectivity event payload. Scrub any recognized
    // credential pattern out of it BEFORE it reaches the payload, the CLI render, or
    // persistence, so the `RedactionState::Safe` marker below is earned, not assumed.
    let (reason, _scrubbed) = capo_state::scrub_free_text(&raw_reason);
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
    // CT2 emitted-surface guard: the `reason` free text was scrubbed above, but make
    // the `Safe` marker MEAN something by scanning the fully-built payload for any
    // leaked credential pattern before persisting, mirroring the expose-stub path.
    if let Err(pattern) = capo_state::assert_connectivity_event_safe(&event.payload_json) {
        return Err(format!(
            "connectivity event payload marked Safe but leaked a `{pattern}` credential pattern; refusing to persist"
        ));
    }
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

/// CT2: render an optional opaque HANDLE as a JSON value — a quoted, escaped
/// string when present, the literal `null` when absent. The value is an opaque
/// pointer/derived field (auth_ref/identity_ref/fingerprint/expiry), never a raw
/// credential, and the emitted-surface guard scans the assembled payload before
/// persistence as a secondary net.
fn json_opt_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape_json(value)),
        None => "null".to_string(),
    }
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
