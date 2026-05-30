use std::path::PathBuf;

use capo_core::{RunId, SessionId, ToolCallId};
use capo_runtime::RedactionRule;
use serde_json::Value;

use crate::{PermissionDecision, ToolAuditEvent, ToolDefinition};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeToolConfig {
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
}

impl RuntimeToolConfig {
    pub fn local_workspace(workspace_root: PathBuf, artifact_root: PathBuf) -> Self {
        Self {
            workspace_root,
            artifact_root,
            env_allowlist: Vec::new(),
            redaction_rules: Vec::new(),
            output_limit_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrapperToolRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_id: String,
    pub capability_profile_id: String,
    pub input: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrapperToolResult {
    pub tool_call_id: ToolCallId,
    pub tool_id: String,
    pub status: String,
    pub summary: String,
    pub input_artifact: Option<WrapperArtifact>,
    pub output_artifacts: Vec<WrapperArtifact>,
    pub permission_decision: PermissionDecision,
    pub events: Vec<ToolAuditEvent>,
}

impl WrapperToolResult {
    /// The narrow typed output object validatable against a wrapper tool's
    /// declared [`ToolDefinition::output_schema`] (ACI2): the observed status,
    /// a human summary, and the recorded output artifacts (full payloads live
    /// in the artifacts, never inline).
    pub fn narrow_output(&self) -> Value {
        Value::Object(
            [
                ("status".to_string(), Value::String(self.status.clone())),
                ("summary".to_string(), Value::String(self.summary.clone())),
                (
                    "output_artifacts".to_string(),
                    Value::Array(
                        self.output_artifacts
                            .iter()
                            .map(|artifact| Value::String(artifact.artifact_id.clone()))
                            .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    pub(crate) fn denied(
        request: WrapperToolRequest,
        definition: ToolDefinition,
        permission_decision: PermissionDecision,
        events: Vec<ToolAuditEvent>,
    ) -> Self {
        Self {
            tool_call_id: request.tool_call_id,
            tool_id: definition.tool_id,
            status: "denied".to_string(),
            summary: permission_decision.explanation.clone(),
            input_artifact: None,
            output_artifacts: Vec::new(),
            permission_decision,
            events,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrapperArtifact {
    pub artifact_id: String,
    pub kind: String,
    pub uri: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub redaction_state: String,
    pub summary: String,
}
