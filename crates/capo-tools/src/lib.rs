//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 adds the first Capo-owned tool registry and an auditable invocation
//! lifecycle. Permission policy remains a separate boundary even when the
//! trusted local prototype allows broadly.

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, ToolCallId};
mod agent_reports;
mod apply_patch;
mod lint;
mod permission;
mod runtime_wrapper_paths;
mod runtime_wrapper_types;
mod runtime_wrappers;
mod search;
mod test_run;
pub use agent_reports::*;
pub use permission::*;
pub use runtime_wrapper_paths::confine_write_path;
pub use runtime_wrapper_types::*;
pub use runtime_wrappers::*;

/// First Capo-owned tools selected by the architecture.
pub const CAPO_OWNED_TOOLS: &[&str] = &[
    "capo.task_status",
    "capo.agent_status",
    "capo.session_summary",
    "capo.project_memory_read",
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
    "capo.apply_patch",
    "capo.search",
    "capo.test_run",
    "capo.project_memory_read",
    "capo.workpad_read",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Capo(CapoToolRegistry),
    Runtime(RuntimeToolWrappers),
    /// ACI8: the `GO2` agent-reporting / evidence tool surface. A distinct
    /// exposure because every tool here emits an agent CLAIM tagged
    /// `agent_reported`, never observed evidence.
    AgentReports(AgentReportRegistry),
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

    /// ACI8: the `GO2` agent-reporting / evidence tool surface.
    pub fn agent_reports() -> Self {
        Self::AgentReports(AgentReportRegistry)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Capo(exposure) => exposure.binding(),
            Self::Runtime(exposure) => exposure.binding(),
            Self::AgentReports(exposure) => exposure.binding(),
            Self::Fake(exposure) => exposure.binding(),
        }
    }

    /// The legacy fake summary shim, kept for the scripted `send_task` e2e path.
    ///
    /// ACI1: this is now reachable ONLY through the explicit test-only
    /// [`Self::Fake`] variant. The `Capo`/`Runtime` variants no longer silently
    /// masquerade as the fake here -- they must go through the typed
    /// [`Self::authorize_and_invoke`] dispatch, which calls the real
    /// [`CapoToolRegistry::authorize_and_invoke`] /
    /// [`RuntimeToolWrappers::authorize_and_invoke`]. Routing a real variant
    /// through the fake summary shim is a wiring bug, so it panics rather than
    /// returning a fabricated fake observation.
    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        match self {
            Self::Fake(exposure) => exposure.invoke(request),
            Self::Capo(_) | Self::Runtime(_) | Self::AgentReports(_) => panic!(
                "ToolExposure::invoke is the fake-only summary shim; the real \
                 `{}` exposure must dispatch through ToolExposure::authorize_and_invoke",
                self.binding().variant
            ),
        }
    }

    /// Typed tool dispatch: route a real tool call through the registry/wrappers
    /// `authorize_and_invoke`, or the fake exposure for the test-only variant.
    ///
    /// ACI1: this replaces the dead fake-only routing. The `Capo` variant
    /// dispatches into [`CapoToolRegistry::authorize_and_invoke`] and the
    /// `Runtime` variant into [`RuntimeToolWrappers::authorize_and_invoke`], so a
    /// real loop turn that invokes `capo.file_read`/`capo.shell_run` flows
    /// through the real authorize+invoke path and the real audit event sequence.
    /// A request whose variant does not match the exposure variant is a wiring
    /// bug and is rejected as a mismatch rather than silently downgraded to the
    /// fake path.
    pub fn authorize_and_invoke(
        &self,
        request: ToolExposureRequest,
        policy: &PermissionPolicy,
    ) -> ToolExposureResult {
        match (self, request) {
            (Self::Capo(registry), ToolExposureRequest::Capo(request)) => {
                ToolExposureResult::Capo(registry.authorize_and_invoke(request, policy))
            }
            (Self::Runtime(wrappers), ToolExposureRequest::Runtime(request)) => {
                ToolExposureResult::Runtime(wrappers.authorize_and_invoke(request, policy))
            }
            (Self::AgentReports(registry), ToolExposureRequest::AgentReport(request)) => {
                ToolExposureResult::AgentReport(registry.authorize_and_invoke(request, policy))
            }
            (Self::Fake(exposure), ToolExposureRequest::Fake(request)) => {
                ToolExposureResult::Fake(exposure.invoke(request))
            }
            (exposure, request) => panic!(
                "ToolExposure::authorize_and_invoke variant mismatch: `{}` exposure \
                 cannot dispatch a `{}` request",
                exposure.binding().variant,
                request.variant_name()
            ),
        }
    }
}

/// A typed tool-dispatch request: a real Capo-registry call, a real
/// runtime-wrapper call, or the test-only fake summary observation.
///
/// ACI1: the typed envelope that lets [`ToolExposure::authorize_and_invoke`]
/// route to the real `authorize_and_invoke` instead of the fake shim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposureRequest {
    Capo(CapoToolRequest),
    Runtime(WrapperToolRequest),
    AgentReport(AgentReportRequest),
    Fake(FakeToolRequest),
}

impl ToolExposureRequest {
    fn variant_name(&self) -> &'static str {
        match self {
            Self::Capo(_) => "capo",
            Self::Runtime(_) => "runtime",
            Self::AgentReport(_) => "agent-report",
            Self::Fake(_) => "fake",
        }
    }
}

/// The typed result of [`ToolExposure::authorize_and_invoke`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposureResult {
    Capo(CapoToolResult),
    Runtime(WrapperToolResult),
    AgentReport(AgentReportRecord),
    Fake(FakeToolResult),
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
            "capo.project_memory_read" => (
                "Project Memory Read",
                false,
                vec![
                    "tool:invoke:capo.project_memory_read",
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
            output_schema: CAPO_REGISTRY_OUTPUT_SCHEMA.to_string(),
            required_scopes_json: json_array(required_scopes),
            risk: if mutates_state { "medium" } else { "low" }.to_string(),
            redaction_policy_json: capo_redaction_policy(tool_id),
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
    /// Input schema descriptor (`{"input":{...}}`).
    pub schema_json: String,
    /// Output schema descriptor (`{"output":{...}}`).
    ///
    /// ACI2: every registered tool declares a non-empty output schema so
    /// "narrow typed output" is a checkable contract rather than convention.
    /// [`ToolDefinition::validate_output`] checks an emitted result against it.
    pub output_schema: String,
    pub required_scopes_json: String,
    pub risk: String,
    /// Per-tool redaction policy descriptor (`{"strategy":...,"fields":[...]}`).
    ///
    /// ACI2: matches `tool-exposure.md`'s `redaction_policy_json` field. Every
    /// registered tool declares a non-empty policy so input/output redaction is
    /// a declared per-tool contract.
    pub redaction_policy_json: String,
    pub exposure: String,
    pub instrumentation_level: String,
    pub status: String,
    pub mutates_state: bool,
}

/// One of the `tool-exposure.md` risk levels.
pub const TOOL_RISK_LEVELS: &[&str] = &["low", "medium", "high", "critical"];

/// Captured runtime process streams that redaction scrubs but that are not
/// fields of a tool's JSON input/output schema.
///
/// ACI2: a wrapper's narrow output keeps stdout/stderr in artifacts, never
/// inline, so these names cannot appear in `schema_json`/`output_schema`. They
/// are still legitimate redaction targets (this is exactly where secrets leak),
/// so [`ToolDefinition::redaction_policy_fields_are_coherent`] accepts them
/// alongside declared schema fields rather than treating the policy as free text.
pub const RUNTIME_CAPTURE_FIELDS: &[&str] = &["stdout", "stderr"];

impl ToolDefinition {
    /// Whether `risk` is one of the `tool-exposure.md` levels.
    pub fn risk_is_valid(&self) -> bool {
        TOOL_RISK_LEVELS.contains(&self.risk.as_str())
    }

    /// Validate an emitted result object against the declared `output_schema`.
    ///
    /// ACI2: the output schema follows the same lightweight descriptor shape as
    /// `schema_json` (`{"output":{"field":"type"}}`). Every declared field must
    /// be present in `result` and match its declared scalar/array type, so a
    /// tool's narrow typed output is checkable rather than convention. A `?`
    /// suffix marks an optional field. Returns the list of validation errors;
    /// an empty list means the result conforms.
    pub fn validate_output(&self, result: &serde_json::Value) -> Vec<String> {
        validate_against_schema(&self.output_schema, "output", result)
    }

    /// Field names declared by the input `schema_json` (`{"input":{...}}`).
    pub fn declared_input_fields(&self) -> Vec<String> {
        descriptor_field_names(&self.schema_json, "input")
    }

    /// Field names declared by the `output_schema` (`{"output":{...}}`).
    pub fn declared_output_fields(&self) -> Vec<String> {
        descriptor_field_names(&self.output_schema, "output")
    }

    /// The `fields` the declared `redaction_policy_json` scrubs.
    pub fn redaction_policy_fields(&self) -> Vec<String> {
        let Ok(policy) = serde_json::from_str::<serde_json::Value>(&self.redaction_policy_json)
        else {
            return Vec::new();
        };
        policy
            .get("fields")
            .and_then(serde_json::Value::as_array)
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| field.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether every field the redaction policy names is a real target: a
    /// declared input field, a declared output field, or a recognized runtime
    /// capture stream ([`RUNTIME_CAPTURE_FIELDS`]). ACI2: keeps the policy
    /// coherent with the tool's actual surface rather than free text. Returns
    /// the list of policy fields that reference nothing real (empty == coherent).
    pub fn incoherent_redaction_fields(&self) -> Vec<String> {
        let inputs = self.declared_input_fields();
        let outputs = self.declared_output_fields();
        self.redaction_policy_fields()
            .into_iter()
            .filter(|field| {
                !inputs.contains(field)
                    && !outputs.contains(field)
                    && !RUNTIME_CAPTURE_FIELDS.contains(&field.as_str())
            })
            .collect()
    }
}

/// Field names declared by a `{"<root>":{"field":"type"}}` descriptor.
fn descriptor_field_names(schema_json: &str, root_key: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(schema_json)
        .ok()
        .and_then(|schema| {
            schema
                .get(root_key)
                .and_then(serde_json::Value::as_object)
                .map(|fields| fields.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// Validate a value object against a `{"<root>":{"field":"type"}}` descriptor.
///
/// Shared by the input and output schema descriptors. `type` is one of
/// `string`, `integer`, `number`, `boolean`, `string[]`, `object`, `array`,
/// optionally suffixed with `?` to mark the field optional.
fn validate_against_schema(
    schema_json: &str,
    root_key: &str,
    value: &serde_json::Value,
) -> Vec<String> {
    let mut errors = Vec::new();
    let schema: serde_json::Value = match serde_json::from_str(schema_json) {
        Ok(schema) => schema,
        Err(error) => {
            errors.push(format!("schema is not valid json: {error}"));
            return errors;
        }
    };
    let Some(fields) = schema.get(root_key).and_then(|root| root.as_object()) else {
        errors.push(format!("schema has no `{root_key}` object"));
        return errors;
    };
    let Some(object) = value.as_object() else {
        errors.push("result is not a json object".to_string());
        return errors;
    };
    for (field, declared_type) in fields {
        let Some(declared_type) = declared_type.as_str() else {
            errors.push(format!("schema field `{field}` type must be a string"));
            continue;
        };
        let (base_type, optional) = match declared_type.strip_suffix('?') {
            Some(base) => (base, true),
            None => (declared_type, false),
        };
        match object.get(field) {
            None => {
                if !optional {
                    errors.push(format!("missing required field `{field}`"));
                }
            }
            Some(actual) => {
                if !scalar_type_matches(base_type, actual) {
                    errors.push(format!(
                        "field `{field}` expected `{base_type}` but found `{}`",
                        json_type_name(actual)
                    ));
                }
            }
        }
    }
    errors
}

fn scalar_type_matches(declared: &str, value: &serde_json::Value) -> bool {
    match declared {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "string[]" => value
            .as_array()
            .is_some_and(|items| items.iter().all(serde_json::Value::is_string)),
        "array" => value.is_array(),
        "object" => value.is_object(),
        // An unknown declared type accepts any present value rather than
        // silently failing closed; the schema-shape test guards declared types.
        _ => true,
    }
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
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

impl CapoToolResult {
    /// The narrow typed output object validatable against a Capo tool's
    /// declared [`ToolDefinition::output_schema`] (ACI2).
    pub fn narrow_output(&self) -> serde_json::Value {
        serde_json::json!({
            "output": self.output,
            "output_artifact_id": self.output_artifact_id,
        })
    }
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
        "capo.project_memory_read" => context.workpad_excerpt.clone(),
        "capo.workpad_read" => context.workpad_excerpt.clone(),
        "capo.evidence_record" => format!("evidence recorded: {}", context.evidence_note),
        "capo.capability_request" => {
            format!("capability requested: {}", context.capability_scope)
        }
        _ => "unsupported tool".to_string(),
    }
}

/// Narrow typed output shape every Capo-registry tool emits: a rendered
/// `output` string plus the `output_artifact_id` that carries the full payload.
pub(crate) const CAPO_REGISTRY_OUTPUT_SCHEMA: &str =
    "{\"output\":{\"output\":\"string\",\"output_artifact_id\":\"string\"}}";

/// Per-tool redaction policy descriptor for a Capo-registry tool.
///
/// Read-only status tools default to the credential-shape scan; tools that
/// carry free-text evidence/scope add those fields to the scrub set.
pub(crate) fn capo_redaction_policy(tool_id: &str) -> String {
    match tool_id {
        "capo.evidence_record" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"evidence\"]}".to_string()
        }
        "capo.capability_request" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"scope\",\"reason\"]}".to_string()
        }
        _ => "{\"strategy\":\"credential_scan\",\"fields\":[\"output\"]}".to_string(),
    }
}

pub(crate) fn unknown_tool_definition(tool_id: &str) -> ToolDefinition {
    ToolDefinition {
        tool_id: tool_id.to_string(),
        display_name: tool_id.to_string(),
        origin: "capo".to_string(),
        handler_kind: "capo_registry".to_string(),
        schema_json: "{}".to_string(),
        output_schema: CAPO_REGISTRY_OUTPUT_SCHEMA.to_string(),
        required_scopes_json: json_array(vec!["tool:invoke:capo"]),
        risk: "medium".to_string(),
        redaction_policy_json: "{\"strategy\":\"credential_scan\",\"fields\":[\"output\"]}"
            .to_string(),
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
