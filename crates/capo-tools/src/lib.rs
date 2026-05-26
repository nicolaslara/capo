//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 adds the first Capo-owned tool registry and an auditable invocation
//! lifecycle. Permission policy remains a separate boundary even when the
//! trusted local prototype allows broadly.

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, ToolCallId};
mod permission;
mod runtime_wrapper_paths;
mod runtime_wrapper_types;
mod runtime_wrappers;
pub use permission::*;
pub use runtime_wrapper_types::*;
pub use runtime_wrappers::*;

/// First Capo-owned tools selected by the architecture.
pub const CAPO_OWNED_TOOLS: &[&str] = &[
    "capo.task_status",
    "capo.agent_status",
    "capo.session_summary",
    "capo.workpad_read",
    "capo.evidence_record",
    "capo.capability_request",
];

/// First Capo-governed wrapper tools for local execution and workspace access.
pub const CAPO_WRAPPER_TOOLS: &[&str] = &[
    "capo.shell_run",
    "capo.git_status",
    "capo.git_diff",
    "capo.git_commit",
    "capo.file_read",
    "capo.file_write",
    "capo.workpad_read",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Capo(CapoToolRegistry),
    Runtime(RuntimeToolWrappers),
    Fake(FakeToolExposure),
}

impl ToolExposure {
    pub fn capo() -> Self {
        Self::Capo(CapoToolRegistry)
    }

    pub fn fake() -> Self {
        Self::Fake(FakeToolExposure)
    }

    pub fn runtime_wrappers(config: RuntimeToolConfig) -> Self {
        Self::Runtime(RuntimeToolWrappers::new(config))
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Capo(exposure) => exposure.binding(),
            Self::Runtime(exposure) => exposure.binding(),
            Self::Fake(exposure) => exposure.binding(),
        }
    }

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        match self {
            Self::Capo(_) => FakeToolExposure.invoke(request),
            Self::Runtime(_) => FakeToolExposure.invoke(request),
            Self::Fake(exposure) => exposure.invoke(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapoToolRegistry;

impl CapoToolRegistry {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ToolExposure,
            variant: "capo-registry",
            fake: false,
        }
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        CAPO_OWNED_TOOLS
            .iter()
            .map(|tool_id| self.describe_tool(tool_id).expect("known tool"))
            .collect()
    }

    pub fn describe_tool(&self, tool_id: &str) -> Option<ToolDefinition> {
        let (display_name, mutates_state, required_scopes, schema_json) = match tool_id {
            "capo.task_status" => (
                "Task Status",
                false,
                vec![
                    "tool:invoke:capo.task_status",
                    "state:read:task",
                    "state:read:session",
                    "state:read:evidence",
                ],
                "{\"input\":{\"task_id\":\"string\"}}",
            ),
            "capo.agent_status" => (
                "Agent Status",
                false,
                vec![
                    "tool:invoke:capo.agent_status",
                    "state:read:agent",
                    "state:read:session",
                    "state:read:runtime",
                    "state:read:provider",
                ],
                "{\"input\":{\"agent_id\":\"string\"}}",
            ),
            "capo.session_summary" => (
                "Session Summary",
                false,
                vec![
                    "tool:invoke:capo.session_summary",
                    "state:read:session",
                    "state:read:tool",
                    "state:read:permission_queue",
                ],
                "{\"input\":{\"session_id\":\"string\"}}",
            ),
            "capo.workpad_read" => (
                "Workpad Read",
                false,
                vec![
                    "tool:invoke:capo.workpad_read",
                    "filesystem:read:workspace",
                    "state:read:task",
                ],
                "{\"input\":{\"path\":\"string\",\"heading\":\"string?\"}}",
            ),
            "capo.evidence_record" => (
                "Evidence Record",
                true,
                vec![
                    "tool:invoke:capo.evidence_record",
                    "state:write:evidence",
                    "state:read:task",
                ],
                "{\"input\":{\"evidence\":\"string\",\"confidence\":\"integer\"}}",
            ),
            "capo.capability_request" => (
                "Capability Request",
                true,
                vec![
                    "tool:invoke:capo.capability_request",
                    "state:read:capability",
                    "state:write:capability_request",
                ],
                "{\"input\":{\"scope\":\"string\",\"reason\":\"string\"}}",
            ),
            _ => return None,
        };

        Some(ToolDefinition {
            tool_id: tool_id.to_string(),
            display_name: display_name.to_string(),
            origin: "capo".to_string(),
            handler_kind: "capo_registry".to_string(),
            schema_json: schema_json.to_string(),
            required_scopes_json: json_array(required_scopes),
            risk: if mutates_state { "medium" } else { "low" }.to_string(),
            exposure: "agent_visible".to_string(),
            instrumentation_level: "full".to_string(),
            status: "available".to_string(),
            mutates_state,
        })
    }

    pub fn authorize_tool_call(
        &self,
        request: &CapoToolRequest,
        policy: &PermissionPolicy,
    ) -> ToolAuthorization {
        let definition = self
            .describe_tool(&request.tool_id)
            .unwrap_or_else(|| unknown_tool_definition(&request.tool_id));
        let permission = policy.decide(PermissionRequest {
            session_id: request.session_id.clone(),
            capability_profile_id: request.capability_profile_id.clone(),
            scope_json: definition.required_scopes_json.clone(),
        });
        ToolAuthorization {
            definition,
            events: vec![
                ToolAuditEvent::new("tool.call_requested", "requested"),
                ToolAuditEvent::new("permission.requested", "pending"),
                ToolAuditEvent::new("permission.decided", permission.effect.clone()),
            ],
            permission,
            session_id: request.session_id.clone(),
            run_id: RunId::new("capo-registry-no-run"),
            tool_call_id: request.tool_call_id.clone(),
            capability_profile_id: request.capability_profile_id.clone(),
            input_hash: capo_tool_context_hash(&request.context),
        }
    }

    pub fn invoke_authorized(
        &self,
        request: CapoToolRequest,
        authorization: ToolAuthorization,
    ) -> CapoToolResult {
        if let Err(error) = verify_capo_authorization_matches_request(&request, &authorization) {
            let mut events = authorization.events;
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "authorization_mismatch",
            ));
            return CapoToolResult {
                tool_call_id: request.tool_call_id,
                tool_id: request.tool_id,
                output: error,
                output_artifact_id: "none".to_string(),
                mutates_state: authorization.definition.mutates_state,
                permission_decision: authorization.permission,
                events,
            };
        }
        if authorization.permission.effect != "allow" {
            let mut events = authorization.events;
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "permission_denied",
            ));
            return CapoToolResult {
                tool_call_id: request.tool_call_id,
                tool_id: request.tool_id,
                output: authorization.permission.explanation.clone(),
                output_artifact_id: "none".to_string(),
                mutates_state: authorization.definition.mutates_state,
                permission_decision: authorization.permission,
                events,
            };
        }

        let output = render_tool_output(&request.tool_id, &request.context);
        let output_artifact_id = format!(
            "artifact-{}-{}",
            request.tool_call_id,
            request.tool_id.replace('.', "-")
        );
        let mut events = authorization.events;
        events.extend([
            ToolAuditEvent::new("capability.grant_used", "used"),
            ToolAuditEvent::new("tool.invocation_started", "running"),
            ToolAuditEvent::new("tool.output_artifact_recorded", "safe"),
            ToolAuditEvent::new("tool.output_observed", "observed"),
            ToolAuditEvent::new("tool.call_completed", "completed"),
            ToolAuditEvent::new("tool.result_delivered", "delivered"),
        ]);

        CapoToolResult {
            tool_call_id: request.tool_call_id,
            tool_id: request.tool_id,
            output,
            output_artifact_id,
            mutates_state: authorization.definition.mutates_state,
            permission_decision: authorization.permission,
            events,
        }
    }

    pub fn authorize_and_invoke(
        &self,
        request: CapoToolRequest,
        policy: &PermissionPolicy,
    ) -> CapoToolResult {
        let authorization = self.authorize_tool_call(&request, policy);
        self.invoke_authorized(request, authorization)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDefinition {
    pub tool_id: String,
    pub display_name: String,
    pub origin: String,
    pub handler_kind: String,
    pub schema_json: String,
    pub required_scopes_json: String,
    pub risk: String,
    pub exposure: String,
    pub instrumentation_level: String,
    pub status: String,
    pub mutates_state: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpClientCapabilityPlan {
    pub filesystem_read: AcpClientCapabilityDecision,
    pub filesystem_write: AcpClientCapabilityDecision,
    pub terminal: AcpClientCapabilityDecision,
}

impl AcpClientCapabilityPlan {
    pub fn from_tool_definitions(
        tool_definitions: &[ToolDefinition],
        policy: &PermissionPolicy,
        session_id: SessionId,
    ) -> Self {
        Self {
            filesystem_read: acp_capability_decision(
                tool_definitions,
                policy,
                &session_id,
                "filesystem.read_text_file",
                "capo.file_read",
            ),
            filesystem_write: acp_capability_decision(
                tool_definitions,
                policy,
                &session_id,
                "filesystem.write_text_file",
                "capo.file_write",
            ),
            terminal: acp_capability_decision(
                tool_definitions,
                policy,
                &session_id,
                "terminal",
                "capo.shell_run",
            ),
        }
    }

    pub fn from_runtime_wrappers(
        wrappers: &RuntimeToolWrappers,
        policy: &PermissionPolicy,
        session_id: SessionId,
    ) -> Self {
        Self::from_tool_definitions(&wrappers.list_tools(), policy, session_id)
    }

    pub fn advertised_capabilities(&self) -> Vec<&str> {
        [
            &self.filesystem_read,
            &self.filesystem_write,
            &self.terminal,
        ]
        .into_iter()
        .filter(|decision| decision.advertise)
        .map(|decision| decision.acp_capability.as_str())
        .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpClientCapabilityDecision {
    pub acp_capability: String,
    pub backing_tool_id: String,
    pub advertise: bool,
    pub reason: String,
    pub required_scopes_json: Option<String>,
    pub permission_effect: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapoToolContext {
    pub task_status: String,
    pub agent_status: String,
    pub session_summary: String,
    pub workpad_excerpt: String,
    pub evidence_note: String,
    pub capability_scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapoToolRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub tool_id: String,
    pub capability_profile_id: String,
    pub context: CapoToolContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolAuthorization {
    pub definition: ToolDefinition,
    pub permission: PermissionDecision,
    pub events: Vec<ToolAuditEvent>,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_call_id: ToolCallId,
    pub capability_profile_id: String,
    pub input_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapoToolResult {
    pub tool_call_id: ToolCallId,
    pub tool_id: String,
    pub output: String,
    pub output_artifact_id: String,
    pub mutates_state: bool,
    pub permission_decision: PermissionDecision,
    pub events: Vec<ToolAuditEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolAuditEvent {
    pub kind: String,
    pub status: String,
}

impl ToolAuditEvent {
    pub fn new(kind: impl Into<String>, status: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            status: status.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolExposure;

impl FakeToolExposure {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ToolExposure, "fake-tools")
    }

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        FakeToolResult {
            tool_call_id: request.tool_call_id,
            tool_name: request.tool_name,
            output_artifact_id: format!("artifact-tool-{}", request.session_id),
            summary: format!("Tool observed session goal: {}", request.input_summary),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub tool_name: String,
    pub input_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub output_artifact_id: String,
    pub summary: String,
}

fn acp_capability_decision(
    tool_definitions: &[ToolDefinition],
    policy: &PermissionPolicy,
    session_id: &SessionId,
    acp_capability: &str,
    backing_tool_id: &str,
) -> AcpClientCapabilityDecision {
    let Some(definition) = tool_definitions
        .iter()
        .find(|definition| definition.tool_id == backing_tool_id)
    else {
        return AcpClientCapabilityDecision {
            acp_capability: acp_capability.to_string(),
            backing_tool_id: backing_tool_id.to_string(),
            advertise: false,
            reason: "missing_backing_wrapper_tool".to_string(),
            required_scopes_json: None,
            permission_effect: None,
        };
    };
    let permission = policy.decide(PermissionRequest {
        session_id: session_id.clone(),
        capability_profile_id: policy.default_profile_id().to_string(),
        scope_json: definition.required_scopes_json.clone(),
    });
    let advertise = permission.effect == "allow";
    AcpClientCapabilityDecision {
        acp_capability: acp_capability.to_string(),
        backing_tool_id: backing_tool_id.to_string(),
        advertise,
        reason: if advertise {
            "backing_wrapper_tool_allowed".to_string()
        } else {
            format!("permission_policy_rejected:{}", permission.explanation)
        },
        required_scopes_json: Some(definition.required_scopes_json.clone()),
        permission_effect: Some(permission.effect),
    }
}

fn render_tool_output(tool_id: &str, context: &CapoToolContext) -> String {
    match tool_id {
        "capo.task_status" => context.task_status.clone(),
        "capo.agent_status" => context.agent_status.clone(),
        "capo.session_summary" => context.session_summary.clone(),
        "capo.workpad_read" => context.workpad_excerpt.clone(),
        "capo.evidence_record" => format!("evidence recorded: {}", context.evidence_note),
        "capo.capability_request" => {
            format!("capability requested: {}", context.capability_scope)
        }
        _ => "unsupported tool".to_string(),
    }
}

pub(crate) fn unknown_tool_definition(tool_id: &str) -> ToolDefinition {
    ToolDefinition {
        tool_id: tool_id.to_string(),
        display_name: tool_id.to_string(),
        origin: "capo".to_string(),
        handler_kind: "capo_registry".to_string(),
        schema_json: "{}".to_string(),
        required_scopes_json: json_array(vec!["tool:invoke:capo"]),
        risk: "medium".to_string(),
        exposure: "internal".to_string(),
        instrumentation_level: "none".to_string(),
        status: "unhealthy".to_string(),
        mutates_state: false,
    }
}

pub(crate) fn json_array(items: Vec<&str>) -> String {
    let quoted = items
        .into_iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{quoted}]")
}

fn verify_capo_authorization_matches_request(
    request: &CapoToolRequest,
    authorization: &ToolAuthorization,
) -> Result<(), String> {
    if authorization.definition.tool_id != request.tool_id {
        return Err(format!(
            "authorization tool mismatch: authorized {} but requested {}",
            authorization.definition.tool_id, request.tool_id
        ));
    }
    if authorization.session_id != request.session_id {
        return Err("authorization session mismatch".to_string());
    }
    if authorization.run_id != RunId::new("capo-registry-no-run") {
        return Err("authorization run mismatch".to_string());
    }
    if authorization.tool_call_id != request.tool_call_id {
        return Err("authorization tool call mismatch".to_string());
    }
    if authorization.capability_profile_id != request.capability_profile_id {
        return Err("authorization capability profile mismatch".to_string());
    }
    if authorization.permission.capability_profile_id != request.capability_profile_id {
        return Err("permission decision profile mismatch".to_string());
    }
    if authorization.permission.scope_json != authorization.definition.required_scopes_json {
        return Err("permission decision scope mismatch".to_string());
    }
    if authorization.input_hash != capo_tool_context_hash(&request.context) {
        return Err("authorization input mismatch".to_string());
    }
    Ok(())
}

fn capo_tool_context_hash(context: &CapoToolContext) -> String {
    let fields = [
        context.task_status.as_str(),
        context.agent_status.as_str(),
        context.session_summary.as_str(),
        context.workpad_excerpt.as_str(),
        context.evidence_note.as_str(),
        context.capability_scope.as_str(),
    ];
    let mut encoded = Vec::new();
    for field in fields {
        encoded.extend_from_slice(field.len().to_string().as_bytes());
        encoded.push(0);
        encoded.extend_from_slice(field.as_bytes());
    }
    content_hash(&encoded)
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests;
