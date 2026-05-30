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
    /// Inline output cap (ACI3). Output beyond this is NOT inlined: the full
    /// payload lives in the artifact and the typed result records `truncated`.
    pub output_limit_bytes: usize,
    /// The hard ceiling the runtime runner enforces for a single wrapper
    /// execution's stdout/stderr (ACI3). This is a real, bounded resource cap:
    /// output beyond it fails the call with `OutputLimitExceeded` rather than
    /// being buffered/persisted, so a runaway command (`yes`) cannot fill memory
    /// or disk. It is deliberately much larger than `output_limit_bytes` so the
    /// normal "over inline cap, under ceiling = truncated inline, full artifact"
    /// path keeps working, but it is never unbounded.
    pub artifact_limit_bytes: usize,
}

/// Default hard ceiling for a single wrapper execution's captured output (ACI3).
/// `start_process` buffers the child's entire stdout/stderr in memory via
/// `command.output()`, so this bound is what keeps that buffer (and the
/// subsequent on-disk artifact) finite for a runaway command.
pub(crate) const DEFAULT_ARTIFACT_LIMIT_BYTES: usize = 16 * 1024 * 1024;

impl RuntimeToolConfig {
    pub fn local_workspace(workspace_root: PathBuf, artifact_root: PathBuf) -> Self {
        Self {
            workspace_root,
            artifact_root,
            env_allowlist: Vec::new(),
            redaction_rules: Vec::new(),
            output_limit_bytes: 64 * 1024,
            artifact_limit_bytes: DEFAULT_ARTIFACT_LIMIT_BYTES,
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
    /// The narrow typed, schema-validated output object for this tool (ACI3).
    ///
    /// Each wrapper emits a per-tool typed result (exit status / `passed` /
    /// duration / `output_artifact_id` for executions, before/after hash +
    /// diff for `file_write`, byte counts for reads) rather than only the
    /// generic status/summary/artifact-id blob. Validatable against the tool's
    /// declared [`ToolDefinition::output_schema`].
    pub typed_output: Value,
    pub input_artifact: Option<WrapperArtifact>,
    pub output_artifacts: Vec<WrapperArtifact>,
    pub permission_decision: PermissionDecision,
    pub events: Vec<ToolAuditEvent>,
}

impl WrapperToolResult {
    /// The narrow typed output object validatable against a wrapper tool's
    /// declared [`ToolDefinition::output_schema`] (ACI2/ACI3).
    ///
    /// ACI3: this is now the per-tool [`Self::typed_output`] the handler built,
    /// not a generic status/summary/artifact blob, so each wrapper returns a
    /// narrow typed result (exit status, `passed`, duration, diff, hashes) that
    /// the loop can act on directly.
    pub fn narrow_output(&self) -> Value {
        self.typed_output.clone()
    }

    pub(crate) fn denied(
        request: WrapperToolRequest,
        definition: ToolDefinition,
        permission_decision: PermissionDecision,
        events: Vec<ToolAuditEvent>,
    ) -> Self {
        let typed_output = denied_typed_output(&definition.output_schema, &permission_decision);
        Self {
            tool_call_id: request.tool_call_id,
            tool_id: definition.tool_id,
            status: "denied".to_string(),
            summary: permission_decision.explanation.clone(),
            typed_output,
            input_artifact: None,
            output_artifacts: Vec::new(),
            permission_decision,
            events,
        }
    }
}

/// A schema-shaped typed output for a denied/canceled call.
///
/// ACI3: every wrapper declares a typed `output_schema`, and denied/failed
/// calls do not run the handler, so we synthesize a minimal object that still
/// validates against the declared schema (each declared field defaulted by its
/// declared scalar/array type) carrying the terminal `status`. This keeps the
/// "every emitted result validates against output_schema" contract true on the
/// deny/fail paths too, without inventing tool-specific values.
pub(crate) fn denied_typed_output(output_schema: &str, decision: &PermissionDecision) -> Value {
    terminal_typed_output(output_schema, "denied", &decision.explanation)
}

/// A schema-shaped typed output for a failed (handler-error) call (ACI3).
///
/// Mirrors [`denied_typed_output`]: the handler did not produce a typed result,
/// so synthesize a schema-valid object carrying `status:"failed"` and the
/// error in `summary` (when those fields are declared).
pub(crate) fn failed_typed_output(output_schema: &str, error: &str) -> Value {
    terminal_typed_output(output_schema, "failed", error)
}

fn terminal_typed_output(output_schema: &str, status: &str, detail: &str) -> Value {
    let mut object = schema_default_object(output_schema);
    if object.contains_key("status") {
        object.insert("status".to_string(), Value::String(status.to_string()));
    }
    if object.contains_key("summary") {
        object.insert("summary".to_string(), Value::String(detail.to_string()));
    }
    Value::Object(object)
}

/// Build a default object for a `{"output":{field:type}}` descriptor: each
/// declared field gets a zero value of its declared scalar/array type (`?`
/// optional fields are omitted). Used to synthesize a schema-valid typed
/// output for deny/fail paths that never ran the handler (ACI3).
fn schema_default_object(output_schema: &str) -> serde_json::Map<String, Value> {
    let mut object = serde_json::Map::new();
    let Ok(schema) = serde_json::from_str::<Value>(output_schema) else {
        return object;
    };
    let Some(fields) = schema.get("output").and_then(Value::as_object) else {
        return object;
    };
    for (field, declared_type) in fields {
        let Some(declared_type) = declared_type.as_str() else {
            continue;
        };
        let (base_type, optional) = match declared_type.strip_suffix('?') {
            Some(base) => (base, true),
            None => (declared_type, false),
        };
        if optional {
            continue;
        }
        let default = match base_type {
            "string" => Value::String(String::new()),
            "integer" | "number" => Value::Number(0.into()),
            "boolean" => Value::Bool(false),
            "string[]" | "array" => Value::Array(Vec::new()),
            "object" => Value::Object(serde_json::Map::new()),
            _ => Value::Null,
        };
        object.insert(field.clone(), default);
    }
    object
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
