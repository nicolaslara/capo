use capo_core::{CommandIntent, CommandTarget, SessionId, ToolCallId};
use capo_state::{
    CapabilityGrantProjection, EventKind, NewEvent, PermissionApprovalProjection, ProjectionRecord,
    RedactionState,
};

use crate::cli_surface::{ParsedArgs, optional_arg, required_arg};
use crate::{debug_error, envelope, escape_json, project_id, stable_cli_hash, state};

pub(crate) fn request_permission_approval(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let scope_json = required_arg(args, "--scope-json")?;
    validate_scope_json(&scope_json)?;
    let subject_json = optional_arg(args, "--subject-json")
        .unwrap_or_else(|| "{\"actor\":\"local-user\"}".to_string());
    validate_json_object("subject-json", &subject_json)?;
    let reason = required_arg(args, "--reason")?;
    let capability_profile_id =
        optional_arg(args, "--profile").unwrap_or_else(|| "trusted-local-dev".to_string());
    let requested_by =
        optional_arg(args, "--requested-by").unwrap_or_else(|| "local-user".to_string());
    let project_id = project_id();
    let session_id = optional_arg(args, "--session").map(SessionId::new);
    let tool_call_id = optional_arg(args, "--tool-call").map(ToolCallId::new);
    let command = envelope(
        "permission-request",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::QueuePermissionApproval,
        Some(reason.clone()),
    );
    let state = state(parsed)?;
    if state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .is_some()
    {
        return Err(format!("approval already exists: {approval_id}"));
    }
    let approval = PermissionApprovalProjection {
        approval_id: approval_id.clone(),
        project_id: project_id.clone(),
        session_id,
        tool_call_id,
        capability_profile_id,
        scope_json,
        subject_json,
        status: "pending".to_string(),
        requested_by,
        reason,
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-approval-queued-{}",
            stable_cli_hash(&approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!("permission-approval-request:{approval_id}"));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_queued=true\napproval_id={}\nstatus=pending\nprofile={}\nsequence={sequence}\ncommand_id={}\n",
        approval.approval_id, approval.capability_profile_id, command.command_id
    ))
}

pub(crate) fn list_permission_approvals(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "permission-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let approvals = state(parsed)?
        .permission_approvals(&project_id())
        .map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\npermission_approvals={}\n",
        command.command_id,
        approvals.len()
    );
    for approval in approvals {
        output.push_str(&format!(
            "approval={} status={} profile={} session={} tool_call={} decision={} grant={} requested_by={} reason={}\n",
            approval.approval_id,
            approval.status,
            approval.capability_profile_id,
            approval
                .session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval
                .tool_call_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval.decision.as_deref().unwrap_or("none"),
            approval.capability_grant_id.as_deref().unwrap_or("none"),
            approval.requested_by,
            approval.reason
        ));
    }
    Ok(output)
}

pub(crate) fn decide_permission_approval(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let decision = required_arg(args, "--decision")?;
    let (effect, persistence) = approval_decision_effect(&decision)?;
    let project_id = project_id();
    let command = envelope(
        "permission-decide",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::DecidePermissionApproval,
        Some(decision.clone()),
    );
    let state = state(parsed)?;
    let approval = state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing approval: {approval_id}"))?;
    if approval.status != "pending" {
        return Err(format!(
            "approval is not pending: {approval_id} status={}",
            approval.status
        ));
    }
    validate_decision_scope(&decision, &approval)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = (decision != "reject_once").then(|| CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json: subject_json.clone(),
        decision_source: "user".to_string(),
        persistence: persistence.to_string(),
        explanation: format!("user approval decision {decision} for {approval_id}"),
        created_at: None,
        expires_at: None,
        revoked_at: None,
        updated_sequence: 0,
    });
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.clone()),
        capability_grant_id: grant
            .as_ref()
            .map(|grant| grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-decided-{}",
            stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
        ),
        EventKind::PermissionDecided,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\"}}",
        escape_json(&approval_id),
        escape_json(&decision),
        escape_json(&grant_id),
        effect,
        persistence
    );
    event.idempotency_key = None;
    event.redaction_state = RedactionState::Safe;
    let grant_event = grant.as_ref().map(|grant| {
        let mut event = NewEvent::new(
            format!(
                "event-capability-grant-{}",
                stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
            ),
            EventKind::CapabilityGrantCreated,
            "capo-cli",
        );
        event.project_id = Some(project_id.clone());
        event.session_id = approval.session_id.clone();
        event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
        event.payload_json = format!(
            "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\"}}",
            escape_json(&approval_id),
            escape_json(&grant.capability_grant_id),
            escape_json(&grant.effect),
            escape_json(&grant.decision_source),
            escape_json(&grant.persistence)
        );
        event.idempotency_key = None;
        event.redaction_state = RedactionState::Safe;
        event
    });
    let sequence = state
        .decide_permission_approval(&approval_id, event, grant_event, decided_approval, grant)
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_decided=true\napproval_id={approval_id}\ndecision={decision}\neffect={effect}\npersistence={persistence}\ncapability_grant_id={}\nsequence={sequence}\ncommand_id={}\n",
        if decision == "reject_once" {
            "none"
        } else {
            &grant_id
        },
        command.command_id
    ))
}

fn validate_scope_json(scope_json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values))
            if values.iter().all(|value| value.as_str().is_some()) =>
        {
            Ok(())
        }
        Ok(_) => Err("--scope-json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("--scope-json is not valid JSON: {error}")),
    }
}

fn validate_json_object(label: &str, json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(format!("--{label} must be a JSON object")),
        Err(error) => Err(format!("--{label} is not valid JSON: {error}")),
    }
}

pub(crate) fn approval_decision_effect(
    decision: &str,
) -> Result<(&'static str, &'static str), String> {
    match decision {
        "allow_once" => Ok(("allow", "once")),
        "allow_always" => Ok(("allow", "until_revoked")),
        "reject_once" => Ok(("deny", "once")),
        "reject_always" => Ok(("deny", "until_revoked")),
        other => Err(format!(
            "unknown approval decision: {other}; expected allow_once, allow_always, reject_once, or reject_always"
        )),
    }
}

fn validate_decision_scope(
    decision: &str,
    approval: &PermissionApprovalProjection,
) -> Result<(), String> {
    if decision != "allow_always" {
        return Ok(());
    }
    let scopes = scope_values(&approval.scope_json)?;
    let durable_allowed = scopes.iter().all(|scope| {
        matches!(
            scope.as_str(),
            "tool:invoke:capo.task_status"
                | "tool:invoke:capo.agent_status"
                | "tool:invoke:capo.session_summary"
                | "tool:invoke:capo.workpad_read"
        ) || scope.starts_with("state:read:")
    });
    if durable_allowed {
        Ok(())
    } else {
        Err(
            "allow_always is restricted to Capo-owned read/status scopes in the PT2 CLI path"
                .to_string(),
        )
    }
}

pub(crate) fn approval_subject_json(
    approval: &PermissionApprovalProjection,
) -> Result<String, String> {
    let mut subject = match serde_json::from_str::<serde_json::Value>(&approval.subject_json) {
        Ok(serde_json::Value::Object(subject)) => subject,
        Ok(_) => return Err("approval subject_json must be a JSON object".to_string()),
        Err(error) => return Err(format!("approval subject_json is not valid JSON: {error}")),
    };
    subject.insert(
        "approval_id".to_string(),
        serde_json::Value::String(approval.approval_id.clone()),
    );
    subject.insert(
        "persistence_scope".to_string(),
        serde_json::Value::String("permission_approval".to_string()),
    );
    if let Some(session_id) = &approval.session_id {
        subject.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
    }
    if let Some(tool_call_id) = &approval.tool_call_id {
        subject.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(tool_call_id.to_string()),
        );
    }
    Ok(serde_json::Value::Object(subject).to_string())
}

pub(crate) fn scope_values(scope_json: &str) -> Result<Vec<String>, String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values)) => values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| "scope_json must contain only strings".to_string())
            })
            .collect(),
        Ok(_) => Err("scope_json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("scope_json is not valid JSON: {error}")),
    }
}
