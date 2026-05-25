//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 adds the first Capo-owned tool registry and an auditable invocation
//! lifecycle. Permission policy remains a separate boundary even when the
//! trusted local prototype allows broadly.

use capo_core::{BoundaryBinding, BoundaryKind, SessionId, ToolCallId};

/// First Capo-owned tools selected by the architecture.
pub const CAPO_OWNED_TOOLS: &[&str] = &[
    "capo.task_status",
    "capo.agent_status",
    "capo.session_summary",
    "capo.workpad_read",
    "capo.evidence_record",
    "capo.capability_request",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Capo(CapoToolRegistry),
    Fake(FakeToolExposure),
}

impl ToolExposure {
    pub fn capo() -> Self {
        Self::Capo(CapoToolRegistry)
    }

    pub fn fake() -> Self {
        Self::Fake(FakeToolExposure)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Capo(exposure) => exposure.binding(),
            Self::Fake(exposure) => exposure.binding(),
        }
    }

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        match self {
            Self::Capo(_) => FakeToolExposure.invoke(request),
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
            permission,
            events: vec![
                ToolAuditEvent::new("tool.call_requested", "requested"),
                ToolAuditEvent::new("permission.requested", "pending"),
                ToolAuditEvent::new("permission.decided", "allow"),
            ],
        }
    }

    pub fn invoke_authorized(
        &self,
        request: CapoToolRequest,
        authorization: ToolAuthorization,
    ) -> CapoToolResult {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionPolicy {
    Fake(FakePermissionPolicy),
    TrustedLocal(AllowTrustedLocalProfilePolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn allow_trusted_local() -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
            Self::TrustedLocal(policy) => policy.binding(),
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        match self {
            Self::Fake(policy) => policy.decide(request),
            Self::TrustedLocal(policy) => policy.decide(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakePermissionPolicy;

impl FakePermissionPolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::PermissionPolicy, "fake-permission")
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        PermissionDecision {
            capability_grant_id: format!("grant-{}", request.session_id),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowTrustedLocalProfilePolicy;

impl AllowTrustedLocalProfilePolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "trusted-local",
            fake: false,
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        PermissionDecision {
            capability_grant_id: format!("grant-{}", request.session_id),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub session_id: SessionId,
    pub capability_profile_id: String,
    pub scope_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDecision {
    pub capability_grant_id: String,
    pub capability_profile_id: String,
    pub effect: String,
    pub scope_json: String,
    pub subject_json: String,
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

fn unknown_tool_definition(tool_id: &str) -> ToolDefinition {
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

fn json_array(items: Vec<&str>) -> String {
    let quoted = items
        .into_iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{quoted}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tool_set_supports_status_and_evidence() {
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.task_status"));
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.evidence_record"));
    }

    #[test]
    fn fake_tool_and_permission_are_separate_boundaries() {
        assert_eq!(
            ToolExposure::fake().binding().kind,
            BoundaryKind::ToolExposure
        );
        assert_eq!(
            ToolExposure::capo().binding(),
            BoundaryBinding {
                kind: BoundaryKind::ToolExposure,
                variant: "capo-registry",
                fake: false,
            }
        );
        assert_eq!(
            PermissionPolicy::fake().binding().kind,
            BoundaryKind::PermissionPolicy
        );
    }

    #[test]
    fn trusted_local_policy_is_explicitly_not_fake() {
        let binding = PermissionPolicy::allow_trusted_local().binding();
        assert_eq!(binding.kind, BoundaryKind::PermissionPolicy);
        assert_eq!(binding.variant, "trusted-local");
        assert!(!binding.fake);
    }

    #[test]
    fn capo_registry_defines_first_six_tools() {
        let registry = CapoToolRegistry;
        let tools = registry.list_tools();

        assert_eq!(tools.len(), 6);
        for tool_id in CAPO_OWNED_TOOLS {
            let definition = registry.describe_tool(tool_id).expect("tool definition");
            assert_eq!(definition.origin, "capo");
            assert_eq!(definition.handler_kind, "capo_registry");
            assert_eq!(definition.instrumentation_level, "full");
            assert!(
                definition
                    .required_scopes_json
                    .contains(&format!("tool:invoke:{tool_id}"))
            );
        }
    }

    #[test]
    fn capo_tools_render_expected_context_outputs() {
        let registry = CapoToolRegistry;
        let policy = PermissionPolicy::allow_trusted_local();
        let context = tool_context();

        let cases = [
            ("capo.task_status", "task active"),
            ("capo.agent_status", "agent running"),
            ("capo.session_summary", "summary text"),
            ("capo.workpad_read", "workpad section"),
            ("capo.evidence_record", "evidence recorded: tests passed"),
            (
                "capo.capability_request",
                "capability requested: shell:execute:workspace",
            ),
        ];

        for (tool_id, expected) in cases {
            let result = registry.authorize_and_invoke(
                CapoToolRequest {
                    tool_call_id: ToolCallId::new(format!("call-{tool_id}")),
                    session_id: SessionId::new("session-tools"),
                    tool_id: tool_id.to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    context: context.clone(),
                },
                &policy,
            );

            assert_eq!(result.output, expected);
        }
    }

    #[test]
    fn trusted_local_tool_invocation_still_emits_audit_lifecycle() {
        let registry = CapoToolRegistry;
        let result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new("call-session-summary"),
                session_id: SessionId::new("session-tools"),
                tool_id: "capo.session_summary".to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: tool_context(),
            },
            &PermissionPolicy::allow_trusted_local(),
        );

        let event_kinds = result
            .events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            event_kinds,
            [
                "tool.call_requested",
                "permission.requested",
                "permission.decided",
                "capability.grant_used",
                "tool.invocation_started",
                "tool.output_artifact_recorded",
                "tool.output_observed",
                "tool.call_completed",
                "tool.result_delivered",
            ]
        );
        assert_eq!(result.permission_decision.effect, "allow");
        assert!(
            result
                .permission_decision
                .scope_json
                .contains("state:read:session")
        );
        assert_eq!(
            result.output_artifact_id,
            "artifact-call-session-summary-capo-session_summary"
        );
    }

    fn tool_context() -> CapoToolContext {
        CapoToolContext {
            task_status: "task active".to_string(),
            agent_status: "agent running".to_string(),
            session_summary: "summary text".to_string(),
            workpad_excerpt: "workpad section".to_string(),
            evidence_note: "tests passed".to_string(),
            capability_scope: "shell:execute:workspace".to_string(),
        }
    }
}
