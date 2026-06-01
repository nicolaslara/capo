use capo_core::{RunId, SessionId, ToolCallId};
use capo_tools::{
    AcpClientCapabilityDecision, AcpClientCapabilityPlan, PermissionPolicy, ToolDefinition,
    WrapperToolRequest,
};
use serde_json::Value;

use crate::AcpAdapter;

impl AcpAdapter {
    pub fn session_setup_plan(
        tool_definitions: &[ToolDefinition],
        policy: &PermissionPolicy,
        session_id: SessionId,
    ) -> AcpSessionSetupPlan {
        let capability_plan = AcpClientCapabilityPlan::from_tool_definitions(
            tool_definitions,
            policy,
            session_id.clone(),
        );
        AcpSessionSetupPlan {
            protocol_version: 1,
            client_kind: "capo".to_string(),
            advertised_capabilities: capability_plan
                .advertised_capabilities()
                .into_iter()
                .map(str::to_string)
                .collect(),
            filesystem_read: capability_plan.filesystem_read,
            filesystem_write: capability_plan.filesystem_write,
            terminal: capability_plan.terminal,
            mcp_server_count: 0,
            credential_policy: "not_inspected".to_string(),
            runtime_started: false,
            provider_cli_executed: false,
            session_id,
            capability_profile_id: policy.default_profile_id().to_string(),
            permission_profile: AcpPermissionProfile::from_policy(policy),
        }
    }
}

/// Which permission-decision profile the live wire client applies when it answers
/// an inbound `session/request_permission` on the wire.
///
/// DP1 scopes the live permission round-trip + the `capability-permissions.md`
/// option-mapping table to the TrustedLocal prototype profile (see DP1 acceptance
/// and [`crate::map_acp_options_trusted_local`]). Carrying the profile here makes
/// the wire client HONEST about that scope: a session running under the
/// TrustedLocal profile uses the documented option mapping, while any OTHER
/// profile fails CLOSED (the wire client cancels the request) rather than silently
/// applying TrustedLocal allow semantics to a session whose policy never granted
/// them. Full per-scope `PermissionPolicy::decide` integration for non-trusted
/// profiles is owned by the controller seam that wires `AcpLiveAdapter` into the
/// loop (alongside the `safety-gates` grant lifecycle), not by the wire client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpPermissionProfile {
    /// The trusted-local-dev prototype profile: apply the documented ACP
    /// option-mapping table.
    TrustedLocal,
    /// Any other profile: the wire client is not the policy authority, so it fails
    /// closed (cancels) rather than self-authorizing on the wire.
    Other,
}

impl AcpPermissionProfile {
    fn from_policy(policy: &PermissionPolicy) -> Self {
        match policy {
            PermissionPolicy::TrustedLocal(_) => Self::TrustedLocal,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpSessionSetupPlan {
    pub protocol_version: i64,
    pub client_kind: String,
    pub advertised_capabilities: Vec<String>,
    pub filesystem_read: AcpClientCapabilityDecision,
    pub filesystem_write: AcpClientCapabilityDecision,
    pub terminal: AcpClientCapabilityDecision,
    pub mcp_server_count: usize,
    pub credential_policy: String,
    pub runtime_started: bool,
    pub provider_cli_executed: bool,
    /// The Capo session this plan was built for. The live wire client uses it to
    /// construct the [`AcpClientCall`] envelope when it routes an inbound
    /// `fs/*` / `terminal/*` request through [`Self::wrapper_request_for_client_call`].
    pub session_id: SessionId,
    /// The capability profile id the session runs under (from the
    /// [`PermissionPolicy`] default profile), carried so the wire client can build
    /// a confined [`AcpClientCall`] without re-deriving it.
    pub capability_profile_id: String,
    /// The permission-decision profile the wire client applies to inbound
    /// `session/request_permission` requests. See [`AcpPermissionProfile`].
    pub permission_profile: AcpPermissionProfile,
}

impl AcpSessionSetupPlan {
    pub fn wrapper_request_for_client_call(
        &self,
        call: AcpClientCall,
    ) -> Result<WrapperToolRequest, String> {
        let (decision, tool_id, input) = match call.method.as_str() {
            "fs/read_text_file" => (
                &self.filesystem_read,
                "capo.file_read",
                serde_json::json!({
                    "path": required_param(&call.params, "path")?,
                }),
            ),
            "fs/write_text_file" => (
                &self.filesystem_write,
                "capo.file_write",
                serde_json::json!({
                    "path": required_param(&call.params, "path")?,
                    "content": required_param(&call.params, "content")?,
                }),
            ),
            "terminal/run" => (
                &self.terminal,
                "capo.shell_run",
                serde_json::json!({
                    "program": required_param(&call.params, "program")?,
                    "argv": string_array_param(&call.params, "argv")?,
                    "cwd": call.params.get("cwd").and_then(Value::as_str),
                }),
            ),
            other => return Err(format!("unsupported ACP client call: {other}")),
        };
        if !decision.advertise {
            return Err(format!(
                "ACP client capability `{}` is not advertised: {}",
                decision.acp_capability, decision.reason
            ));
        }
        Ok(WrapperToolRequest {
            tool_call_id: call.tool_call_id,
            session_id: call.session_id,
            run_id: call.run_id,
            tool_id: tool_id.to_string(),
            capability_profile_id: call.capability_profile_id,
            input,
        })
    }

    /// Whether `method` is a client-call the live wire client is expected to
    /// service (one of the `fs/*` / `terminal/*` ACP client methods).
    pub fn is_client_call_method(method: &str) -> bool {
        matches!(
            method,
            "fs/read_text_file" | "fs/write_text_file" | "terminal/run"
        )
    }

    /// Route one inbound ACP client-call request (`fs/*` / `terminal/*`) through
    /// the confinement seam.
    ///
    /// This builds the [`AcpClientCall`] envelope from the plan's session/profile
    /// context and the request params, then runs it through
    /// [`Self::wrapper_request_for_client_call`], which (a) maps the method to the
    /// backing wrapper tool, (b) extracts/validates the required params (path /
    /// program / argv), and (c) REJECTS any capability the plan did not advertise.
    /// The wire client turns `Ok` into a JSON-RPC result and `Err` into a JSON-RPC
    /// error addressed to the agent's request id, so an inbound client-call is
    /// never silently ingested without a confinement decision and a reply.
    pub fn route_inbound_client_call(
        &self,
        method: &str,
        params: &Value,
        run_id: RunId,
    ) -> Result<WrapperToolRequest, String> {
        let tool_call_id = params
            .get("toolCallId")
            .and_then(Value::as_str)
            .or_else(|| {
                params
                    .get("toolCall")
                    .and_then(|tc| tc.get("toolCallId"))
                    .and_then(Value::as_str)
            })
            .map(ToolCallId::new)
            .unwrap_or_else(|| ToolCallId::new(format!("acp-client-call-{method}")));
        let call = AcpClientCall {
            method: method.to_string(),
            params: params.clone(),
            tool_call_id,
            session_id: self.session_id.clone(),
            run_id,
            capability_profile_id: self.capability_profile_id.clone(),
        };
        self.wrapper_request_for_client_call(call)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpClientCall {
    pub method: String,
    pub params: Value,
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub capability_profile_id: String,
}

fn required_param(params: &Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing ACP client call string param: {key}"))
}

fn string_array_param(params: &Value, key: &str) -> Result<Vec<String>, String> {
    match params.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("ACP client call param `{key}` must be string[]"))
            })
            .collect(),
        Some(_) => Err(format!("ACP client call param `{key}` must be string[]")),
        None => Ok(Vec::new()),
    }
}
