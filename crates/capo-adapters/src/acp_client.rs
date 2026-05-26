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
        let capability_plan =
            AcpClientCapabilityPlan::from_tool_definitions(tool_definitions, policy, session_id);
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
