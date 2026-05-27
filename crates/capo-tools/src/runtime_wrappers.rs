use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use capo_core::{BoundaryBinding, BoundaryKind};
use capo_runtime::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, RuntimeError,
    RuntimeOutputArtifact,
};
use serde_json::Value;

use crate::runtime_wrapper_paths::{
    ensure_under_workspace, is_workpad_path, nearest_existing_ancestor, sanitize_path_component,
    sanitized_run_id, workspace_path, workspace_relative_path,
};
use crate::{
    CAPO_WRAPPER_TOOLS, PermissionPolicy, PermissionRequest, ToolAuditEvent, ToolAuthorization,
    ToolDefinition, content_hash, json_array, unknown_tool_definition,
};
use crate::{RuntimeToolConfig, WrapperArtifact, WrapperToolRequest, WrapperToolResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeToolWrappers {
    config: RuntimeToolConfig,
}

impl RuntimeToolWrappers {
    pub fn new(config: RuntimeToolConfig) -> Self {
        Self { config }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ToolExposure,
            variant: "runtime-wrappers",
            fake: false,
        }
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        CAPO_WRAPPER_TOOLS
            .iter()
            .map(|tool_id| self.describe_tool(tool_id).expect("known wrapper tool"))
            .collect()
    }

    pub fn describe_tool(&self, tool_id: &str) -> Option<ToolDefinition> {
        let (display_name, mutates_state, risk, required_scopes, schema_json) = match tool_id {
            "capo.shell_run" => (
                "Shell Run",
                false,
                "high",
                vec!["tool:invoke:capo.shell_run", "shell:execute:workspace"],
                "{\"input\":{\"program\":\"string\",\"argv\":\"string[]\",\"cwd\":\"string?\"}}",
            ),
            "capo.git_status" => (
                "Git Status",
                false,
                "low",
                vec!["tool:invoke:capo.git_status", "git:status:workspace"],
                "{\"input\":{\"path\":\"string?\"}}",
            ),
            "capo.git_diff" => (
                "Git Diff",
                false,
                "low",
                vec!["tool:invoke:capo.git_diff", "git:diff:workspace"],
                "{\"input\":{\"path\":\"string?\"}}",
            ),
            "capo.git_commit" => (
                "Git Commit",
                true,
                "high",
                vec!["tool:invoke:capo.git_commit", "git:commit:workspace"],
                "{\"input\":{\"message\":\"string\"}}",
            ),
            "capo.file_read" => (
                "File Read",
                false,
                "low",
                vec!["tool:invoke:capo.file_read", "filesystem:read:workspace"],
                "{\"input\":{\"path\":\"string\"}}",
            ),
            "capo.file_write" => (
                "File Write",
                true,
                "medium",
                vec!["tool:invoke:capo.file_write", "filesystem:write:workspace"],
                "{\"input\":{\"path\":\"string\",\"content\":\"string\"}}",
            ),
            "capo.workpad_read" => (
                "Workpad Read",
                false,
                "low",
                vec![
                    "tool:invoke:capo.workpad_read",
                    "filesystem:read:workspace",
                    "state:read:task",
                ],
                "{\"input\":{\"path\":\"string\"}}",
            ),
            "capo.project_memory_read" => (
                "Project Memory Read",
                false,
                "low",
                vec![
                    "tool:invoke:capo.project_memory_read",
                    "filesystem:read:workspace",
                    "state:read:task",
                ],
                "{\"input\":{\"path\":\"string\"}}",
            ),
            _ => return None,
        };

        Some(ToolDefinition {
            tool_id: tool_id.to_string(),
            display_name: display_name.to_string(),
            origin: "runtime".to_string(),
            handler_kind: "runtime_wrapper".to_string(),
            schema_json: schema_json.to_string(),
            required_scopes_json: json_array(required_scopes),
            risk: risk.to_string(),
            exposure: "agent_visible".to_string(),
            instrumentation_level: "full".to_string(),
            status: "available".to_string(),
            mutates_state,
        })
    }

    pub fn authorize_tool_call(
        &self,
        request: &WrapperToolRequest,
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
        let permission_effect = permission.effect.clone();
        ToolAuthorization {
            definition,
            permission,
            events: vec![
                ToolAuditEvent::new("tool.call_requested", "requested"),
                ToolAuditEvent::new("permission.requested", "pending"),
                ToolAuditEvent::new("permission.decided", permission_effect),
            ],
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            tool_call_id: request.tool_call_id.clone(),
            capability_profile_id: request.capability_profile_id.clone(),
            input_hash: wrapper_input_hash(&request.input),
        }
    }

    pub fn invoke_authorized(
        &self,
        request: WrapperToolRequest,
        authorization: ToolAuthorization,
    ) -> WrapperToolResult {
        if let Err(error) = verify_authorization_matches_request(&request, &authorization) {
            let mut events = authorization.events;
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "authorization_mismatch",
            ));
            return WrapperToolResult {
                tool_call_id: request.tool_call_id,
                tool_id: request.tool_id,
                status: "denied".to_string(),
                summary: error,
                input_artifact: None,
                output_artifacts: Vec::new(),
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
            return WrapperToolResult::denied(
                request,
                authorization.definition,
                authorization.permission,
                events,
            );
        }

        let mut events = authorization.events;
        events.extend([
            ToolAuditEvent::new("capability.grant_used", "used"),
            ToolAuditEvent::new("tool.invocation_started", "running"),
        ]);
        let input_artifact = self.record_input_artifact(&request);
        let execution = self.execute(&request);
        match execution {
            Ok(execution) => {
                events.extend([
                    ToolAuditEvent::new("tool.output_artifact_recorded", "safe"),
                    ToolAuditEvent::new("tool.output_observed", execution.status.clone()),
                    ToolAuditEvent::new("tool.call_completed", "completed"),
                    ToolAuditEvent::new("tool.result_delivered", "delivered"),
                ]);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: execution.status,
                    summary: execution.summary,
                    input_artifact: Some(input_artifact),
                    output_artifacts: execution.output_artifacts,
                    permission_decision: authorization.permission,
                    events,
                }
            }
            Err(error) => {
                events.extend([
                    ToolAuditEvent::new("tool.output_observed", "failed"),
                    ToolAuditEvent::new("tool.call_failed", "failed"),
                ]);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: "failed".to_string(),
                    summary: error,
                    input_artifact: Some(input_artifact),
                    output_artifacts: Vec::new(),
                    permission_decision: authorization.permission,
                    events,
                }
            }
        }
    }

    pub fn authorize_and_invoke(
        &self,
        request: WrapperToolRequest,
        policy: &PermissionPolicy,
    ) -> WrapperToolResult {
        let authorization = self.authorize_tool_call(&request, policy);
        self.invoke_authorized(request, authorization)
    }

    fn execute(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        match request.tool_id.as_str() {
            "capo.shell_run" => self.shell_run(request),
            "capo.git_status" => self.git_command(request, "status", &["status", "--short"]),
            "capo.git_diff" => self.git_command(request, "diff", &["diff", "--"]),
            "capo.git_commit" => self.git_commit(request),
            "capo.file_read" => self.file_read(request, "file_read"),
            "capo.file_write" => self.file_write(request),
            "capo.project_memory_read" => self.project_memory_read(request),
            "capo.workpad_read" => self.workpad_read(request),
            other => Err(format!("unsupported wrapper tool: {other}")),
        }
    }

    fn shell_run(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let program = required_input(request, "program")?;
        let argv = request
            .input
            .get("argv")
            .map(json_string_array)
            .transpose()?
            .unwrap_or_default();
        let cwd = request
            .input
            .get("cwd")
            .and_then(Value::as_str)
            .map(|path| self.resolve_workspace_path(path, true))
            .transpose()?
            .unwrap_or_else(|| self.config.workspace_root.clone());
        let outcome = self
            .runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                program,
                argv,
                cwd,
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!(
                "shell exited with {}",
                outcome
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ),
            output_artifacts: vec![
                wrapper_artifact("shell_stdout", outcome.stdout),
                wrapper_artifact("shell_stderr", outcome.stderr),
            ],
        })
    }

    fn git_command(
        &self,
        request: &WrapperToolRequest,
        label: &str,
        base_args: &[&str],
    ) -> Result<WrapperExecution, String> {
        let mut argv = base_args
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();
        if let Some(path) = request.input.get("path").and_then(Value::as_str) {
            let relative = workspace_relative_path(path)?;
            argv.push(relative);
        }
        let outcome = self
            .runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                program: "git".to_string(),
                argv,
                cwd: self.config.workspace_root.clone(),
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!("git {label} completed"),
            output_artifacts: vec![
                wrapper_artifact("git_stdout", outcome.stdout),
                wrapper_artifact("git_stderr", outcome.stderr),
            ],
        })
    }

    fn git_commit(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let message = required_input(request, "message")?;
        if message.trim().is_empty() {
            return Err("git_commit requires a non-empty message".to_string());
        }
        if message.chars().any(char::is_control) {
            return Err("git_commit message must not contain control characters".to_string());
        }
        let outcome = self
            .runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                program: "git".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "user.name=Capo Wrapper".to_string(),
                    "-c".to_string(),
                    "user.email=capo@example.invalid".to_string(),
                    "commit".to_string(),
                    "-m".to_string(),
                    message,
                ],
                cwd: self.config.workspace_root.clone(),
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!(
                "git commit completed with {}",
                outcome
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ),
            output_artifacts: vec![
                wrapper_artifact("git_commit_stdout", outcome.stdout),
                wrapper_artifact("git_commit_stderr", outcome.stderr),
            ],
        })
    }

    fn file_read(
        &self,
        request: &WrapperToolRequest,
        kind: &str,
    ) -> Result<WrapperExecution, String> {
        let path = self.resolve_workspace_path(&required_input(request, "path")?, false)?;
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let artifact = self.write_tool_artifact(
            request,
            kind,
            &format!("{} bytes read from {}", bytes.len(), path.display()),
            &bytes,
            "safe",
        )?;
        Ok(WrapperExecution {
            status: "completed".to_string(),
            summary: format!("{kind} read {}", path.display()),
            output_artifacts: vec![artifact],
        })
    }

    fn workpad_read(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let requested = required_input(request, "path")?;
        if !is_workpad_path(&requested) {
            return Err(format!(
                "workpad_read only supports TASKS.md, project.md, or workpads/*.md paths: {requested}"
            ));
        }
        self.file_read(request, "workpad_read")
    }

    fn project_memory_read(
        &self,
        request: &WrapperToolRequest,
    ) -> Result<WrapperExecution, String> {
        let requested = required_input(request, "path")?;
        if !is_workpad_path(&requested) {
            return Err(format!(
                "project_memory_read only supports TASKS.md, project.md, or workpads/*.md paths: {requested}"
            ));
        }
        self.file_read(request, "project_memory_read")
    }

    fn file_write(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let path = self.resolve_workspace_path(&required_input(request, "path")?, true)?;
        let content = required_input(request, "content")?;
        let before = fs::read(&path).unwrap_or_default();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, content.as_bytes()).map_err(|error| error.to_string())?;
        let after = fs::read(&path).map_err(|error| error.to_string())?;
        let before_hash = content_hash(&before);
        let after_hash = content_hash(&after);
        let diff_summary = format!(
            "file={} before={} after={}\n",
            path.display(),
            before_hash,
            after_hash
        );
        let artifact = self.write_tool_artifact(
            request,
            "file_write_diff",
            "before/after hash summary",
            diff_summary.as_bytes(),
            "safe",
        )?;
        Ok(WrapperExecution {
            status: "completed".to_string(),
            summary: format!("file_write wrote {}", path.display()),
            output_artifacts: vec![artifact],
        })
    }

    fn record_input_artifact(&self, request: &WrapperToolRequest) -> WrapperArtifact {
        let payload = format!(
            "{{\"tool_id\":\"{}\",\"input\":{}}}",
            request.tool_id, request.input
        );
        let redacted = self.redact_bytes(payload.as_bytes());
        let redaction_state = if redacted == payload.as_bytes() {
            "safe"
        } else {
            "redacted"
        };
        self.write_tool_artifact(
            request,
            "input",
            "wrapper input",
            &redacted,
            redaction_state,
        )
        .expect("write wrapper input artifact")
    }

    fn write_tool_artifact(
        &self,
        request: &WrapperToolRequest,
        kind: &str,
        summary: &str,
        bytes: &[u8],
        redaction_state: &str,
    ) -> Result<WrapperArtifact, String> {
        let dir = self
            .config
            .artifact_root
            .join(sanitize_path_component(request.tool_call_id.as_str()));
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let path = dir.join(format!("{kind}.txt"));
        fs::write(&path, bytes).map_err(|error| error.to_string())?;
        Ok(WrapperArtifact {
            artifact_id: format!("artifact-wrapper-{}-{kind}", request.tool_call_id),
            kind: kind.to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash(bytes),
            size_bytes: bytes.len() as i64,
            redaction_state: redaction_state.to_string(),
            summary: summary.to_string(),
        })
    }

    fn runtime_runner(&self) -> LocalProcessRunner {
        let mut config = LocalProcessConfig::for_test(
            self.config.workspace_root.clone(),
            self.config.artifact_root.clone(),
        );
        config.env_allowlist = self.config.env_allowlist.clone();
        config.redaction_rules = self.config.redaction_rules.clone();
        config.output_limit_bytes = self.config.output_limit_bytes;
        LocalProcessRunner::new(config)
    }

    fn redact_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let mut text = String::from_utf8_lossy(bytes).to_string();
        for rule in &self.config.redaction_rules {
            if text.contains(&rule.pattern) {
                text = text.replace(&rule.pattern, &rule.replacement);
            }
        }
        text.into_bytes()
    }

    fn resolve_workspace_path(&self, path: &str, allow_missing: bool) -> Result<PathBuf, String> {
        let candidate = workspace_path(&self.config.workspace_root, path);
        if candidate.exists() {
            let canonical = candidate
                .canonicalize()
                .map_err(|error| error.to_string())?;
            ensure_under_workspace(&canonical, &self.config.workspace_root)?;
            return Ok(canonical);
        }
        if !allow_missing {
            return Err(format!(
                "workspace path does not exist: {}",
                candidate.display()
            ));
        }
        let ancestor = nearest_existing_ancestor(&candidate).ok_or_else(|| {
            format!(
                "workspace path has no existing ancestor: {}",
                candidate.display()
            )
        })?;
        let ancestor = ancestor.canonicalize().map_err(|error| error.to_string())?;
        ensure_under_workspace(&ancestor, &self.config.workspace_root)?;
        Ok(candidate)
    }
}

struct WrapperExecution {
    status: String,
    summary: String,
    output_artifacts: Vec<WrapperArtifact>,
}

fn verify_authorization_matches_request(
    request: &WrapperToolRequest,
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
    if authorization.run_id != request.run_id {
        return Err("authorization run mismatch".to_string());
    }
    if authorization.tool_call_id != request.tool_call_id {
        return Err("authorization tool call mismatch".to_string());
    }
    if authorization.capability_profile_id != request.capability_profile_id {
        return Err("authorization capability profile mismatch".to_string());
    }
    if authorization.input_hash != wrapper_input_hash(&request.input) {
        return Err("authorization input mismatch".to_string());
    }
    if authorization.permission.capability_profile_id != request.capability_profile_id {
        return Err("permission decision profile mismatch".to_string());
    }
    if authorization.permission.scope_json != authorization.definition.required_scopes_json {
        return Err("permission decision scope mismatch".to_string());
    }
    Ok(())
}

fn wrapper_input_hash(input: &Value) -> String {
    content_hash(input.to_string().as_bytes())
}

fn required_input(request: &WrapperToolRequest, key: &str) -> Result<String, String> {
    request
        .input
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("{} input requires string field `{key}`", request.tool_id))
}

fn json_string_array(value: &Value) -> Result<Vec<String>, String> {
    let Value::Array(items) = value else {
        return Err("argv must be an array of strings".to_string());
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToString::to_string)
                .ok_or_else(|| "argv must contain only strings".to_string())
        })
        .collect()
}

fn wrapper_artifact(kind: &str, artifact: RuntimeOutputArtifact) -> WrapperArtifact {
    WrapperArtifact {
        artifact_id: artifact.artifact_id,
        kind: kind.to_string(),
        uri: artifact.path.display().to_string(),
        content_hash: artifact.content_hash,
        size_bytes: artifact.size_bytes,
        redaction_state: artifact.redaction_state,
        summary: format!("{kind} runtime artifact"),
    }
}

fn runtime_error(error: RuntimeError) -> String {
    format!("{error:?}")
}
