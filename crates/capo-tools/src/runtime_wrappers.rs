use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use capo_core::{BoundaryBinding, BoundaryKind};
use capo_runtime::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, RuntimeError,
    RuntimeOutputArtifact,
};
use serde_json::Value;
use similar::TextDiff;

use crate::runtime_wrapper_paths::{
    ensure_under_workspace, is_workpad_path, lexically_normalize, nearest_existing_ancestor,
    sanitize_path_component, sanitized_run_id, workspace_path, workspace_relative_path,
};
use crate::runtime_wrapper_types::{denied_typed_output, failed_typed_output};
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
                "{\"input\":{\"path\":\"string\",\"content\":\"string?\",\"expected_hash\":\"string?\",\"replace\":\"string?\",\"with\":\"string?\"}}",
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
            output_schema: wrapper_output_schema(tool_id).to_string(),
            required_scopes_json: json_array(required_scopes),
            risk: risk.to_string(),
            redaction_policy_json: wrapper_redaction_policy(tool_id),
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
            let typed_output = denied_typed_output(
                &authorization.definition.output_schema,
                &authorization.permission,
            );
            return WrapperToolResult {
                tool_call_id: request.tool_call_id,
                tool_id: request.tool_id,
                status: "denied".to_string(),
                summary: error,
                typed_output,
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
                    typed_output: execution.typed_output,
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
                let typed_output =
                    failed_typed_output(&authorization.definition.output_schema, &error);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: "failed".to_string(),
                    summary: error,
                    typed_output,
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
        // ACI3: run with an UNBOUNDED runner output limit so a successful run
        // that exceeds the inline cap is not turned into a hard
        // `OutputLimitExceeded` failure (which would discard the artifacts). The
        // full output is preserved in the artifact; we record `truncated` in the
        // typed result by comparing the artifact size against the configured
        // inline `output_limit_bytes` cap.
        let started = Instant::now();
        let outcome = self
            .uncapped_runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                turn_id: None,
                program,
                argv,
                cwd,
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let stdout = wrapper_artifact("shell_stdout", outcome.stdout);
        let stderr = wrapper_artifact("shell_stderr", outcome.stderr);
        let cap = self.config.output_limit_bytes as i64;
        let truncated = stdout.size_bytes > cap || stderr.size_bytes > cap;
        let passed = outcome.exit_code == Some(0);
        let exit_label = outcome
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let typed_output = exec_typed_output(
            &outcome.process.status,
            outcome.exit_code,
            passed,
            duration_ms,
            &stdout.artifact_id,
            truncated,
        );
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!("shell exited with {exit_label}"),
            typed_output,
            output_artifacts: vec![stdout, stderr],
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
        let started = Instant::now();
        let outcome = self
            .uncapped_runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                turn_id: None,
                program: "git".to_string(),
                argv,
                cwd: self.config.workspace_root.clone(),
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let stdout = wrapper_artifact("git_stdout", outcome.stdout);
        let stderr = wrapper_artifact("git_stderr", outcome.stderr);
        let cap = self.config.output_limit_bytes as i64;
        let truncated = stdout.size_bytes > cap || stderr.size_bytes > cap;
        let passed = outcome.exit_code == Some(0);
        let typed_output = exec_typed_output(
            &outcome.process.status,
            outcome.exit_code,
            passed,
            duration_ms,
            &stdout.artifact_id,
            truncated,
        );
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!("git {label} completed"),
            typed_output,
            output_artifacts: vec![stdout, stderr],
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
        let started = Instant::now();
        let outcome = self
            .uncapped_runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                turn_id: None,
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
        let duration_ms = started.elapsed().as_millis() as i64;
        let stdout = wrapper_artifact("git_commit_stdout", outcome.stdout);
        let stderr = wrapper_artifact("git_commit_stderr", outcome.stderr);
        let cap = self.config.output_limit_bytes as i64;
        let truncated = stdout.size_bytes > cap || stderr.size_bytes > cap;
        let passed = outcome.exit_code == Some(0);
        let exit_label = outcome
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let typed_output = exec_typed_output(
            &outcome.process.status,
            outcome.exit_code,
            passed,
            duration_ms,
            &stdout.artifact_id,
            truncated,
        );
        Ok(WrapperExecution {
            status: outcome.process.status,
            summary: format!("git commit completed with {exit_label}"),
            typed_output,
            output_artifacts: vec![stdout, stderr],
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
        let typed_output = serde_json::json!({
            "status": "completed",
            "path": path.display().to_string(),
            "bytes_read": bytes.len() as i64,
            "content_hash": artifact.content_hash.clone(),
            "output_artifact_id": artifact.artifact_id.clone(),
        });
        Ok(WrapperExecution {
            status: "completed".to_string(),
            summary: format!("{kind} read {}", path.display()),
            typed_output,
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
        let before = fs::read(&path).unwrap_or_default();
        let before_hash = content_hash(&before);

        // ACI3: an expected-precondition hash makes blind clobbers impossible.
        // If the caller declares the content hash they believe is on disk and it
        // does not match, return a typed precondition-failed result WITHOUT
        // writing, carrying the expected/actual hashes for the loop to reflect.
        if let Some(expected_hash) = request.input.get("expected_hash").and_then(Value::as_str)
            && expected_hash != before_hash
        {
            let typed_output = serde_json::json!({
                "status": "precondition_failed",
                "path": path.display().to_string(),
                "mode": "precondition",
                "before_hash": before_hash,
                "after_hash": before_hash,
                "bytes_written": 0,
                "output_artifact_id": "none",
                "expected_hash": expected_hash,
                "actual_hash": before_hash,
            });
            return Ok(WrapperExecution {
                status: "precondition_failed".to_string(),
                summary: format!(
                    "file_write precondition failed for {}: expected {expected_hash} but on-disk is {before_hash}",
                    path.display()
                ),
                typed_output,
                output_artifacts: Vec::new(),
            });
        }

        // ACI3: accept either a whole-file `content` overwrite OR a structured
        // `replace`/`with` substitution against the current on-disk content.
        let (new_content, mode) = match (
            request.input.get("content").and_then(Value::as_str),
            request.input.get("replace").and_then(Value::as_str),
        ) {
            (Some(content), None) => (content.as_bytes().to_vec(), "overwrite"),
            (None, Some(needle)) => {
                let with = request
                    .input
                    .get("with")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "file_write structured replace requires a string `with` field".to_string()
                    })?;
                let current = String::from_utf8_lossy(&before);
                if !current.contains(needle) {
                    return Err(format!(
                        "file_write replace target not found in {}",
                        path.display()
                    ));
                }
                (current.replacen(needle, with, 1).into_bytes(), "replace")
            }
            (Some(_), Some(_)) => {
                return Err(
                    "file_write accepts either `content` or a `replace`/`with` pair, not both"
                        .to_string(),
                );
            }
            (None, None) => {
                return Err(
                    "file_write requires either a `content` string or a `replace`/`with` pair"
                        .to_string(),
                );
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, &new_content).map_err(|error| error.to_string())?;
        let after = fs::read(&path).map_err(|error| error.to_string())?;
        let after_hash = content_hash(&after);

        // ACI3: emit a real unified diff artifact (before -> after), not just a
        // before/after hash summary, so the change is reviewable.
        let diff = unified_diff(&before, &after, &path.display().to_string());
        let artifact = self.write_tool_artifact(
            request,
            "file_write_diff",
            &format!("unified diff for {}", path.display()),
            diff.as_bytes(),
            "safe",
        )?;
        let typed_output = serde_json::json!({
            "status": "completed",
            "path": path.display().to_string(),
            "mode": mode,
            "before_hash": before_hash,
            "after_hash": after_hash,
            "bytes_written": after.len() as i64,
            "output_artifact_id": artifact.artifact_id.clone(),
        });
        Ok(WrapperExecution {
            status: "completed".to_string(),
            summary: format!("file_write {mode} wrote {}", path.display()),
            typed_output,
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

    /// A runner whose runtime output limit is effectively unbounded (ACI3).
    ///
    /// Execution wrappers want the FULL output preserved in the artifact and a
    /// `truncated` marker in the typed result, rather than a hard
    /// `OutputLimitExceeded` failure that discards the artifacts. The wrapper
    /// then compares the artifact size against the configured inline
    /// `output_limit_bytes` cap to decide `truncated`.
    fn uncapped_runtime_runner(&self) -> LocalProcessRunner {
        let mut config = LocalProcessConfig::for_test(
            self.config.workspace_root.clone(),
            self.config.artifact_root.clone(),
        );
        config.env_allowlist = self.config.env_allowlist.clone();
        config.redaction_rules = self.config.redaction_rules.clone();
        config.output_limit_bytes = usize::MAX;
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
        // Build the candidate against the CANONICAL workspace root, then lexically
        // fold `.`/`..` so a not-yet-created target with interior `..` (e.g.
        // `src/sub/../../../escape.txt`) cannot escape confinement by hiding
        // behind a non-existent intermediate dir -- the same fix applied to the
        // shared `confine_write_path` engine. Canonicalizing the root up front
        // keeps the lexical containment check valid even when `workspace_root`
        // itself is a symlink (e.g. macOS `/var` -> `/private/var`).
        let canonical_root = self
            .config
            .workspace_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let candidate = lexically_normalize(&workspace_path(&canonical_root, path));
        ensure_under_workspace(&candidate, &canonical_root)?;
        if candidate.exists() {
            let canonical = candidate
                .canonicalize()
                .map_err(|error| error.to_string())?;
            ensure_under_workspace(&canonical, &canonical_root)?;
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
        ensure_under_workspace(&ancestor, &canonical_root)?;
        Ok(candidate)
    }
}

/// Narrow typed output shape for an execution wrapper (`shell_run`, the git
/// wrappers): the observed `status`, an `exit_status`, a `passed`
/// interpretation, the wall-clock `duration_ms`, the primary
/// `output_artifact_id`, and a `truncated` marker. Full stdout/stderr live in
/// the artifacts, never inline (ACI3).
pub(crate) const EXEC_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"exit_status\":\"integer?\",\"passed\":\"boolean\",\"duration_ms\":\"integer\",\"output_artifact_id\":\"string\",\"truncated\":\"boolean\"}}";

/// Narrow typed output shape for `file_read`: the read `path`, the
/// `bytes_read` count, the `content_hash`, and the `output_artifact_id` that
/// carries the file payload (ACI3).
pub(crate) const FILE_READ_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"path\":\"string\",\"bytes_read\":\"integer\",\"content_hash\":\"string\",\"output_artifact_id\":\"string\"}}";

/// Narrow typed output shape for `file_write`: the written `path`, the write
/// `mode` (`overwrite`/`replace`), the `before_hash`/`after_hash`, the
/// `bytes_written`, the unified-diff `output_artifact_id`, plus the
/// precondition fields surfaced on a precondition failure (`expected_hash`,
/// `actual_hash`) (ACI3).
pub(crate) const FILE_WRITE_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"path\":\"string\",\"mode\":\"string\",\"before_hash\":\"string\",\"after_hash\":\"string\",\"bytes_written\":\"integer\",\"output_artifact_id\":\"string\",\"expected_hash\":\"string?\",\"actual_hash\":\"string?\"}}";

/// The declared `output_schema` descriptor for a runtime wrapper tool (ACI3).
pub(crate) fn wrapper_output_schema(tool_id: &str) -> &'static str {
    match tool_id {
        "capo.shell_run" | "capo.git_status" | "capo.git_diff" | "capo.git_commit" => {
            EXEC_OUTPUT_SCHEMA
        }
        "capo.file_write" => FILE_WRITE_OUTPUT_SCHEMA,
        // file_read and the read-only workpad/project-memory aliases.
        _ => FILE_READ_OUTPUT_SCHEMA,
    }
}

/// Per-tool redaction policy descriptor for a runtime wrapper.
///
/// Execution tools (`shell_run`, the git wrappers) capture process stdout/stderr
/// where secrets leak, so their policy is the credential-shape scan applied to
/// output; read/write tools scrub input/output content. Every wrapper declares
/// a non-empty policy (ACI2).
pub(crate) fn wrapper_redaction_policy(tool_id: &str) -> String {
    match tool_id {
        "capo.shell_run" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"stdout\",\"stderr\"]}".to_string()
        }
        "capo.git_status" | "capo.git_diff" | "capo.git_commit" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"stdout\",\"stderr\"]}".to_string()
        }
        "capo.file_write" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"content\"]}".to_string()
        }
        _ => "{\"strategy\":\"credential_scan\",\"fields\":[\"content\"]}".to_string(),
    }
}

struct WrapperExecution {
    status: String,
    summary: String,
    /// The narrow typed, per-tool output object the handler built (ACI3),
    /// validatable against the tool's declared `output_schema`.
    typed_output: Value,
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

/// Build the narrow typed output object for an execution wrapper (ACI3).
///
/// Carries the observed `status`, the `exit_status` (null on a signal), the
/// `passed` interpretation, the wall-clock `duration_ms`, the primary
/// `output_artifact_id` (full output lives in the artifact), and the
/// `truncated` marker. Validatable against [`EXEC_OUTPUT_SCHEMA`].
fn exec_typed_output(
    status: &str,
    exit_status: Option<i32>,
    passed: bool,
    duration_ms: i64,
    output_artifact_id: &str,
    truncated: bool,
) -> Value {
    serde_json::json!({
        "status": status,
        "exit_status": exit_status,
        "passed": passed,
        "duration_ms": duration_ms,
        "output_artifact_id": output_artifact_id,
        "truncated": truncated,
    })
}

/// Render a unified diff between `before` and `after` for `path` (ACI3).
///
/// Uses the `similar` line differ so `file_write` emits a real reviewable diff
/// artifact rather than only a before/after hash summary. Non-UTF-8 content is
/// rendered lossily for the diff text; the on-disk write itself is byte-exact.
fn unified_diff(before: &[u8], after: &[u8], path: &str) -> String {
    let before_text = String::from_utf8_lossy(before);
    let after_text = String::from_utf8_lossy(after);
    let diff = TextDiff::from_lines(before_text.as_ref(), after_text.as_ref());
    diff.unified_diff()
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}
