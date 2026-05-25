//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 adds the first Capo-owned tool registry and an auditable invocation
//! lifecycle. Permission policy remains a separate boundary even when the
//! trusted local prototype allows broadly.

use capo_core::{BoundaryBinding, BoundaryKind, SessionId, ToolCallId};
use serde_json::Value;

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
            events: vec![
                ToolAuditEvent::new("tool.call_requested", "requested"),
                ToolAuditEvent::new("permission.requested", "pending"),
                ToolAuditEvent::new("permission.decided", permission.effect.clone()),
            ],
            permission,
        }
    }

    pub fn invoke_authorized(
        &self,
        request: CapoToolRequest,
        authorization: ToolAuthorization,
    ) -> CapoToolResult {
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
    Static(StaticPolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn allow_trusted_local() -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy)
    }

    pub fn static_read_only_local() -> Self {
        Self::Static(StaticPolicy::read_only_local())
    }

    pub fn static_reviewer() -> Self {
        Self::Static(StaticPolicy::reviewer())
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
            Self::TrustedLocal(policy) => policy.binding(),
            Self::Static(policy) => policy.binding(),
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        match self {
            Self::Fake(policy) => policy.decide(request),
            Self::TrustedLocal(policy) => policy.decide(request),
            Self::Static(policy) => policy.decide(request),
        }
    }

    pub fn default_profile_id(&self) -> &'static str {
        match self {
            Self::Fake(_) => "fake",
            Self::TrustedLocal(_) => "trusted-local-dev",
            Self::Static(policy) => policy.profile_id(),
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
            capability_grant_id: scoped_grant_id(&request, "allow"),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: "fake".to_string(),
            persistence: "once".to_string(),
            explanation: "fake policy allows all requests".to_string(),
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
            capability_grant_id: scoped_grant_id(&request, "allow"),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: "allow_trusted_local_profile".to_string(),
            persistence: "until_session_end".to_string(),
            explanation: "trusted local profile allows audited local prototype request".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticPolicy {
    profile_id: String,
    allowed_scopes: Vec<String>,
}

impl StaticPolicy {
    pub fn read_only_local() -> Self {
        Self {
            profile_id: "read-only-local".to_string(),
            allowed_scopes: [
                "tool:invoke:capo.task_status",
                "tool:invoke:capo.agent_status",
                "tool:invoke:capo.session_summary",
                "tool:invoke:capo.workpad_read",
                "state:read:task",
                "state:read:agent",
                "state:read:session",
                "state:read:runtime",
                "state:read:provider",
                "state:read:tool",
                "state:read:evidence",
                "state:read:permission_queue",
                "filesystem:read:workspace",
                "git:status:workspace",
                "git:diff:workspace",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn reviewer() -> Self {
        Self {
            profile_id: "reviewer".to_string(),
            allowed_scopes: [
                "tool:invoke:capo.task_status",
                "tool:invoke:capo.agent_status",
                "tool:invoke:capo.session_summary",
                "tool:invoke:capo.workpad_read",
                "state:read:task",
                "state:read:agent",
                "state:read:session",
                "state:read:runtime",
                "state:read:provider",
                "state:read:tool",
                "state:read:evidence",
                "state:read:permission_queue",
                "state:read:capability",
                "filesystem:read:workspace",
                "git:status:workspace",
                "git:diff:workspace",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "static",
            fake: false,
        }
    }

    pub fn profile_id(&self) -> &'static str {
        match self.profile_id.as_str() {
            "read-only-local" => "read-only-local",
            "reviewer" => "reviewer",
            _ => "static",
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        let requested_scopes = match scope_items(&request.scope_json) {
            Ok(scopes) => scopes,
            Err(explanation) => {
                return PermissionDecision {
                    capability_grant_id: scoped_grant_id(&request, "deny"),
                    capability_profile_id: request.capability_profile_id,
                    effect: "deny".to_string(),
                    scope_json: request.scope_json,
                    subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
                    decision_source: format!("static_policy:{}", self.profile_id),
                    persistence: "once".to_string(),
                    explanation,
                };
            }
        };
        let missing_scopes = requested_scopes
            .iter()
            .filter(|scope| !self.allowed_scopes.iter().any(|allowed| allowed == *scope))
            .cloned()
            .collect::<Vec<_>>();
        let allowed = !requested_scopes.is_empty() && missing_scopes.is_empty();
        PermissionDecision {
            capability_grant_id: scoped_grant_id(&request, if allowed { "allow" } else { "deny" }),
            capability_profile_id: request.capability_profile_id,
            effect: if allowed { "allow" } else { "deny" }.to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: format!("static_policy:{}", self.profile_id),
            persistence: "once".to_string(),
            explanation: if allowed {
                format!(
                    "static profile `{}` allows all requested scopes",
                    self.profile_id
                )
            } else if requested_scopes.is_empty() {
                "static policy rejected request with no parseable scopes".to_string()
            } else {
                format!(
                    "static profile `{}` rejects missing scopes: {}",
                    self.profile_id,
                    missing_scopes.join(",")
                )
            },
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
    pub decision_source: String,
    pub persistence: String,
    pub explanation: String,
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

fn scope_items(scope_json: &str) -> Result<Vec<String>, String> {
    let value = serde_json::from_str::<Value>(scope_json)
        .map_err(|error| format!("static policy rejected malformed scope json: {error}"))?;
    let Value::Array(items) = value else {
        return Err("static policy rejected non-array scope json".to_string());
    };
    let mut scopes = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(scope) = item else {
            return Err("static policy rejected non-string scope item".to_string());
        };
        scopes.push(scope);
    }
    Ok(scopes)
}

fn scoped_grant_id(request: &PermissionRequest, effect: &str) -> String {
    format!(
        "grant-{}-{}-{}",
        request.session_id,
        effect,
        stable_hash(&format!(
            "{}:{}:{}",
            request.capability_profile_id, request.scope_json, effect
        ))
    )
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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

        let static_binding = PermissionPolicy::static_read_only_local().binding();
        assert_eq!(static_binding.kind, BoundaryKind::PermissionPolicy);
        assert_eq!(static_binding.variant, "static");
        assert!(!static_binding.fake);
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

    #[test]
    fn static_read_only_policy_allows_read_tools_and_denies_writes() {
        let registry = CapoToolRegistry;
        let policy = PermissionPolicy::static_read_only_local();

        let read_result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new("call-session-summary"),
                session_id: SessionId::new("session-tools"),
                tool_id: "capo.session_summary".to_string(),
                capability_profile_id: "read-only-local".to_string(),
                context: tool_context(),
            },
            &policy,
        );

        assert_eq!(read_result.permission_decision.effect, "allow");
        assert_eq!(
            read_result.permission_decision.decision_source,
            "static_policy:read-only-local"
        );
        assert!(
            read_result.events.iter().any(|event| {
                event.kind == "tool.invocation_started" && event.status == "running"
            })
        );

        let write_result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new("call-evidence-record"),
                session_id: SessionId::new("session-tools"),
                tool_id: "capo.evidence_record".to_string(),
                capability_profile_id: "read-only-local".to_string(),
                context: tool_context(),
            },
            &policy,
        );

        assert_eq!(write_result.permission_decision.effect, "deny");
        assert!(
            write_result
                .permission_decision
                .explanation
                .contains("state:write:evidence")
        );
        assert_eq!(write_result.output_artifact_id, "none");
        assert!(write_result.events.iter().any(|event| {
            event.kind == "tool.call_canceled" && event.status == "permission_denied"
        }));
        assert!(
            !write_result
                .events
                .iter()
                .any(|event| event.kind == "tool.invocation_started")
        );
    }

    #[test]
    fn static_reviewer_policy_keeps_decisions_scoped() {
        let policy = PermissionPolicy::static_reviewer();
        let allowed = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-review"),
            capability_profile_id: "reviewer".to_string(),
            scope_json: json_array(vec!["git:diff:workspace", "state:read:task"]),
        });
        assert_eq!(allowed.effect, "allow");
        assert_eq!(allowed.persistence, "once");
        assert!(allowed.scope_json.contains("git:diff:workspace"));

        let denied = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-review"),
            capability_profile_id: "reviewer".to_string(),
            scope_json: json_array(vec!["shell:execute:workspace"]),
        });
        assert_eq!(denied.effect, "deny");
        assert!(denied.explanation.contains("shell:execute:workspace"));
        assert_eq!(denied.subject_json, "{\"session_id\":\"session-review\"}");
    }

    #[test]
    fn static_policy_rejects_malformed_scope_payloads() {
        let policy = PermissionPolicy::static_read_only_local();
        let object_payload = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-static"),
            capability_profile_id: "read-only-local".to_string(),
            scope_json: "{\"tool:invoke:capo.workpad_read\":true}".to_string(),
        });
        assert_eq!(object_payload.effect, "deny");
        assert!(object_payload.explanation.contains("non-array scope json"));

        let non_string_payload = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-static"),
            capability_profile_id: "read-only-local".to_string(),
            scope_json: "[\"state:read:task\",true]".to_string(),
        });
        assert_eq!(non_string_payload.effect, "deny");
        assert!(
            non_string_payload
                .explanation
                .contains("non-string scope item")
        );
    }

    #[test]
    fn grant_ids_include_scope_identity() {
        let policy = PermissionPolicy::static_read_only_local();
        let status = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-static"),
            capability_profile_id: "read-only-local".to_string(),
            scope_json: json_array(vec!["state:read:task"]),
        });
        let summary = policy.decide(PermissionRequest {
            session_id: SessionId::new("session-static"),
            capability_profile_id: "read-only-local".to_string(),
            scope_json: json_array(vec!["state:read:session"]),
        });

        assert_ne!(status.capability_grant_id, summary.capability_grant_id);
        assert!(
            status
                .capability_grant_id
                .starts_with("grant-session-static-allow-")
        );
        assert!(
            summary
                .capability_grant_id
                .starts_with("grant-session-static-allow-")
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
