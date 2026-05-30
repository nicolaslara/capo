use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use capo_core::{BoundaryBinding, BoundaryKind};
use capo_runtime::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, RuntimeError,
    RuntimeOutputArtifact,
};
use serde_json::Value;
use similar::TextDiff;

use crate::apply_patch::{NoMatch, PatchHunk, apply_hunks};
use crate::lint::{LintFinding, Linter};
use crate::runtime_wrapper_paths::{
    ensure_under_workspace, is_workpad_path, lexically_normalize, nearest_existing_ancestor,
    sanitize_path_component, sanitized_run_id, workspace_path, workspace_relative_path,
};
use crate::runtime_wrapper_types::{denied_typed_output, failed_typed_output};
use crate::search::{SearchCaps, SearchMatch, apply_caps, parse_ripgrep_json};
use crate::test_run::{FailingItemsCaps, extract_failing_items};
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
            "capo.apply_patch" => (
                "Apply Patch",
                true,
                "medium",
                vec!["tool:invoke:capo.apply_patch", "filesystem:write:workspace"],
                "{\"input\":{\"path\":\"string\",\"hunks\":\"array\",\"auto_lint\":\"boolean?\"}}",
            ),
            "capo.search" => (
                "Search",
                false,
                "low",
                vec!["tool:invoke:capo.search", "filesystem:read:workspace"],
                "{\"input\":{\"query\":\"string\",\"path\":\"string?\",\"max_matches\":\"integer?\",\"max_preview_bytes\":\"integer?\"}}",
            ),
            "capo.test_run" => (
                "Test Run",
                // A test/check run is observation, not a state mutation in its own
                // right (the gate, not this tool, interprets the result).
                false,
                "high",
                vec!["tool:invoke:capo.test_run", "shell:execute:workspace"],
                "{\"input\":{\"program\":\"string\",\"argv\":\"string[]\",\"cwd\":\"string?\",\"max_failing_items\":\"integer?\"}}",
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
            Ok(execution) if execution.reached_completion => {
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
            // A terminal outcome that did NOT complete a unit of work (no write,
            // no artifact) -- e.g. a `precondition_failed` guard. It must NOT
            // emit the success audit sequence (`tool.output_artifact_recorded` /
            // `tool.call_completed`); instead it flows through the same
            // non-completed audit shape as a handler failure so the dispatch
            // layer stamps its real terminal status without ever marking it a
            // completed call (ACI3).
            Ok(execution) => {
                events.extend([
                    ToolAuditEvent::new("tool.output_observed", execution.status.clone()),
                    ToolAuditEvent::new("tool.call_failed", execution.status.clone()),
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
            "capo.apply_patch" => self.apply_patch(request),
            "capo.search" => self.search(request),
            "capo.test_run" => self.test_run(request),
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
            .bounded_runtime_runner()
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
        Ok(WrapperExecution::completed(
            outcome.process.status,
            format!("shell exited with {exit_label}"),
            typed_output,
            vec![stdout, stderr],
        ))
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
            .bounded_runtime_runner()
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
        Ok(WrapperExecution::completed(
            outcome.process.status,
            format!("git {label} completed"),
            typed_output,
            vec![stdout, stderr],
        ))
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
            .bounded_runtime_runner()
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
        Ok(WrapperExecution::completed(
            outcome.process.status,
            format!("git commit completed with {exit_label}"),
            typed_output,
            vec![stdout, stderr],
        ))
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
        Ok(WrapperExecution::completed(
            "completed".to_string(),
            format!("{kind} read {}", path.display()),
            typed_output,
            vec![artifact],
        ))
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
                reached_completion: false,
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
        Ok(WrapperExecution::completed(
            "completed".to_string(),
            format!("file_write {mode} wrote {}", path.display()),
            typed_output,
            vec![artifact],
        ))
    }

    /// `capo.apply_patch`: apply a typed sequence of search/replace hunks to one
    /// file with whitespace/fuzzy-tolerant location, then run a syntax/lint check
    /// (ACI4).
    ///
    /// Path confinement reuses [`Self::resolve_workspace_path`] so a patch cannot
    /// edit outside the workspace. A hunk that no strategy can locate returns a
    /// STRUCTURED retryable no-match result (status `no_match`, carrying the
    /// rejected hunk index, the reason, and the nearest candidate) WITHOUT
    /// writing -- the loop reflects and retries. A successful apply returns a
    /// typed diff result (files touched, hunks applied/rejected, changed line
    /// ranges, the full diff as an artifact) and, for a Rust file, typed
    /// `rustfmt --check` findings the loop can repair.
    fn apply_patch(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let path = self.resolve_workspace_path(&required_input(request, "path")?, true)?;
        let hunks = parse_patch_hunks(request)?;
        let before = fs::read(&path).unwrap_or_default();
        // The patch operates on UTF-8 text; non-UTF-8 files are rejected up front
        // rather than corrupted by a lossy round-trip.
        let before_text = String::from_utf8(before.clone())
            .map_err(|_| format!("apply_patch target is not utf-8 text: {}", path.display()))?;

        let applied = match apply_hunks(&before_text, &hunks) {
            Ok(applied) => applied,
            // A hunk that no strategy located: a structured retryable no-match,
            // NOT a raw error string. It made no change and produced no artifact,
            // so it flows through the non-completed audit shape (no
            // `tool.call_completed`) like a precondition guard.
            Err(no_match) => {
                return Ok(self.no_match_execution(&path.display().to_string(), &no_match));
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, applied.new_content.as_bytes()).map_err(|error| error.to_string())?;

        let diff = unified_diff(
            &before,
            applied.new_content.as_bytes(),
            &path.display().to_string(),
        );
        let artifact = self.write_tool_artifact(
            request,
            "apply_patch_diff",
            &format!("unified diff for {}", path.display()),
            diff.as_bytes(),
            "safe",
        )?;

        // Lint-on-edit: Rust-first via `rustfmt --check`, language-pluggable. Off
        // when the caller passes `auto_lint:false`.
        let auto_lint = request
            .input
            .get("auto_lint")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let (lint_status, lint_findings) = if auto_lint {
            self.lint_edited(&path, request)
        } else {
            ("skipped".to_string(), Vec::new())
        };

        let changed_ranges: Vec<Value> = applied
            .changed_line_ranges
            .iter()
            .map(|range| Value::String(range.clone()))
            .collect();
        let lint_findings_json: Vec<Value> =
            lint_findings.iter().map(LintFinding::to_json).collect();
        let strategies: Vec<String> = applied
            .matches
            .iter()
            .map(|located| located.strategy.label().to_string())
            .collect();
        let typed_output = serde_json::json!({
            "status": "completed",
            "path": path.display().to_string(),
            "hunks_total": hunks.len() as i64,
            "hunks_applied": applied.matches.len() as i64,
            "hunks_rejected": 0,
            "changed_line_ranges": changed_ranges,
            "output_artifact_id": artifact.artifact_id.clone(),
            "lint_status": lint_status,
            "lint_findings": lint_findings_json,
        });
        Ok(WrapperExecution::completed(
            "completed".to_string(),
            format!(
                "apply_patch applied {} hunk(s) to {} via [{}]; lint {}",
                applied.matches.len(),
                path.display(),
                strategies.join(", "),
                lint_status
            ),
            typed_output,
            vec![artifact],
        ))
    }

    /// `capo.search`: ripgrep-backed search returning typed, bounded
    /// `path:line:preview` matches with an explicit truncation marker (ACI5).
    ///
    /// The search root defaults to the workspace and may be narrowed to a
    /// workspace-relative subpath; it is confined with the same
    /// [`Self::resolve_workspace_path`] every read/write wrapper uses, so a query
    /// cannot read outside the workspace. ripgrep runs through the bounded runtime
    /// runner with deterministic ordering (`--sort path`), and its line-delimited
    /// JSON is parsed and CAPPED by a per-call match cap AND a total preview byte
    /// cap. When either cap is hit the typed result is marked `truncated` (with a
    /// `truncation_reason`) so the agent knows the result is partial rather than
    /// silently incomplete. Each preview line is scrubbed through the configured
    /// redaction before it reaches the agent, and full output is never inlined --
    /// only the bounded `path:line:preview` matches are returned, so the agent
    /// finds edit targets without the tool dumping whole files.
    fn search(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
        let query = required_input(request, "query")?;
        if query.is_empty() {
            return Err("capo.search requires a non-empty `query`".to_string());
        }
        // The search root is the workspace, optionally narrowed to a confined
        // subpath. Confinement reuses the shared resolver so a `..`/absolute
        // escape is rejected before ripgrep runs. `resolve_workspace_path`
        // returns a CANONICAL absolute path; we canonicalize the workspace root
        // the same way so the two can be relativized below.
        let canonical_root = self
            .config
            .workspace_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let search_root = match request.input.get("path").and_then(Value::as_str) {
            Some(path) if !path.is_empty() => self.resolve_workspace_path(path, false)?,
            _ => canonical_root.clone(),
        };
        // Run ripgrep RELATIVE to the workspace root (cwd is the canonical root)
        // and pass the workspace-relative subpath -- `.` for the whole workspace
        // -- as the search argument. ripgrep echoes the argument form back in
        // `path.text`, so a relative argument yields `./`-relative paths that
        // `normalize_match_path` folds into clean workspace-relative paths. This
        // is what keeps `capo.search` consistent with every other wrapper
        // (which feed/return workspace-relative paths) instead of leaking the
        // host's absolute filesystem layout, and it produces paths the agent can
        // hand straight back to `capo.file_read`/`capo.apply_patch`.
        let search_arg = match search_root.strip_prefix(&canonical_root) {
            Ok(relative) if relative.as_os_str().is_empty() => ".".to_string(),
            Ok(relative) => relative.display().to_string(),
            // The resolver already confined `search_root` under the workspace, so
            // a failed strip would be a logic error; fall back to `.` (the whole
            // workspace) rather than leaking the absolute path to ripgrep.
            Err(_) => ".".to_string(),
        };
        let caps = self.search_caps(request)?;

        let started = Instant::now();
        let outcome = self
            .bounded_runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                turn_id: None,
                program: resolve_search_program(),
                argv: vec![
                    "--json".to_string(),
                    "--sort".to_string(),
                    "path".to_string(),
                    // Do NOT cap ripgrep itself at the per-call match cap: doing so
                    // (`--max-count = max_matches`, a PER-FILE limit) would make the
                    // reported `total_matches` a capped undercount instead of the
                    // true pre-cap total, defeating the contract that lets the agent
                    // see how much was elided. The per-call match cap and the total
                    // byte cap are enforced in-process by `apply_caps`, and the
                    // runner's artifact ceiling stays the hard backstop that rejects
                    // a pathological hot query (megabytes of `--json`) with
                    // OutputLimitExceeded rather than filling memory/disk.
                    "--".to_string(),
                    query.clone(),
                    search_arg,
                ],
                cwd: canonical_root.clone(),
                env: HashMap::new(),
            })
            .map_err(runtime_error)?;
        let duration_ms = started.elapsed().as_millis() as i64;

        // ripgrep exits 1 when there are simply no matches; that is a normal,
        // successful empty search, NOT a failure. A genuinely abnormal exit
        // (2+, e.g. a bad regex) is surfaced as a handler error.
        if let Some(code) = outcome.exit_code
            && code > 1
        {
            let stderr = fs::read_to_string(&outcome.stderr.path).unwrap_or_default();
            return Err(format!("search failed (rg exit {code}): {}", stderr.trim()));
        }

        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap_or_default();
        let raw = parse_ripgrep_json(&stdout);
        // Redact every preview line BEFORE capping/returning so a configured
        // secret on a matched line never reaches the agent in the clear.
        let redacted: Vec<SearchMatch> = raw
            .into_iter()
            .map(|item| SearchMatch {
                preview: String::from_utf8_lossy(&self.redact_bytes(item.preview.as_bytes()))
                    .into_owned(),
                ..item
            })
            .collect();
        let bounded = apply_caps(redacted, caps);

        let matches_json: Vec<Value> = bounded.matches.iter().map(SearchMatch::to_json).collect();
        let typed_output = serde_json::json!({
            "status": "completed",
            "query": query,
            "matches": matches_json,
            "returned_matches": bounded.matches.len() as i64,
            "total_matches": bounded.total_matches as i64,
            "truncated": bounded.truncated,
            "truncation_reason": bounded.truncation_reason,
            "duration_ms": duration_ms,
        });
        Ok(WrapperExecution::completed(
            "completed".to_string(),
            format!(
                "search matched {} line(s){}",
                bounded.total_matches,
                if bounded.truncated {
                    format!(
                        " (truncated to {} via {})",
                        bounded.matches.len(),
                        bounded.truncation_reason
                    )
                } else {
                    String::new()
                }
            ),
            typed_output,
            Vec::new(),
        ))
    }

    /// `capo.test_run` / `capo.check`: a specialized shell wrapper that runs a
    /// test/check command and returns a typed
    /// `{command, exit_status, passed, failing_items, duration_ms,
    /// output_artifact_id}` record (ACI6).
    ///
    /// This tool emits decision-grade EVIDENCE only. It does NOT compute a score
    /// or own the verification gate -- `safety-gates`' `VerificationRunner`
    /// consumes this typed record and owns `score_run`. ACI never scores a run.
    ///
    /// The full command output is always written to a redacted artifact; the
    /// inline `failing_items` list is bounded (failing test names, or the
    /// first-N failure lines when no names are recognized) so the result stays
    /// decision-grade and never dumps the whole log. `passed` is the exit-status
    /// interpretation (exit 0 == passed). `started_at`/`completed_at` are
    /// captured as wall-clock millis-since-epoch and `duration_ms` is the
    /// measured elapsed time, for later evaluation by the gate.
    fn test_run(&self, request: &WrapperToolRequest) -> Result<WrapperExecution, String> {
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
        let caps = self.failing_items_caps(request)?;
        let command_display = render_command(&program, &argv);

        // Run with the bounded runner (full output preserved up to the artifact
        // ceiling) like the other execution wrappers, so a large-but-bounded test
        // log lands in the artifact rather than failing the call.
        let started_at_ms = epoch_millis();
        let started = Instant::now();
        let outcome = self
            .bounded_runtime_runner()
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
        let completed_at_ms = epoch_millis();

        // The full, un-truncated stdout+stderr live in a redacted artifact; the
        // inline `failing_items` list is derived from (and bounded against) it.
        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap_or_default();
        let stderr = fs::read_to_string(&outcome.stderr.path).unwrap_or_default();
        let combined = format!("{stdout}\n{stderr}");
        let passed = outcome.exit_code == Some(0);
        let failing = extract_failing_items(&combined, passed, caps);

        let artifact = self.write_tool_artifact(
            request,
            "test_run_output",
            &format!("test_run output for `{command_display}`"),
            self.redact_bytes(combined.as_bytes()).as_slice(),
            "redacted",
        )?;

        let failing_items_json: Vec<Value> = failing
            .items
            .iter()
            .map(|item| Value::String(item.clone()))
            .collect();
        let typed_output = serde_json::json!({
            "status": outcome.process.status,
            "command": command_display,
            "exit_status": outcome.exit_code,
            "passed": passed,
            "failing_items": failing_items_json,
            "failing_items_total": failing.total as i64,
            "failing_items_truncated": failing.truncated,
            "duration_ms": duration_ms,
            "started_at": started_at_ms,
            "completed_at": completed_at_ms,
            "output_artifact_id": artifact.artifact_id.clone(),
        });
        Ok(WrapperExecution::completed(
            outcome.process.status,
            format!(
                "test_run `{command_display}` {} ({} failing item(s){})",
                if passed { "passed" } else { "failed" },
                failing.total,
                if failing.truncated { ", truncated" } else { "" }
            ),
            typed_output,
            vec![artifact],
        ))
    }

    /// Resolve the per-call `failing_items` caps from the request, falling back
    /// to the bounded defaults (ACI6). A caller may TIGHTEN the count cap via
    /// `max_failing_items`; a non-positive value is rejected so the cap can never
    /// be disabled into a whole-log dump.
    fn failing_items_caps(&self, request: &WrapperToolRequest) -> Result<FailingItemsCaps, String> {
        let mut caps = FailingItemsCaps::default();
        if let Some(max_items) = request.input.get("max_failing_items") {
            let value = max_items
                .as_i64()
                .ok_or_else(|| "test_run `max_failing_items` must be an integer".to_string())?;
            if value <= 0 {
                return Err("test_run `max_failing_items` must be a positive integer".to_string());
            }
            caps.max_items = value as usize;
        }
        Ok(caps)
    }

    /// Resolve the per-call search caps from the request, falling back to the
    /// bounded defaults (ACI5). A caller may TIGHTEN or widen the match/byte caps;
    /// non-positive values are rejected so a cap can never be disabled into a
    /// whole-repo dump.
    fn search_caps(&self, request: &WrapperToolRequest) -> Result<SearchCaps, String> {
        let mut caps = SearchCaps::default();
        if let Some(max_matches) = request.input.get("max_matches") {
            let value = max_matches
                .as_i64()
                .ok_or_else(|| "search `max_matches` must be an integer".to_string())?;
            if value <= 0 {
                return Err("search `max_matches` must be a positive integer".to_string());
            }
            caps.max_matches = value as usize;
        }
        if let Some(max_preview_bytes) = request.input.get("max_preview_bytes") {
            let value = max_preview_bytes
                .as_i64()
                .ok_or_else(|| "search `max_preview_bytes` must be an integer".to_string())?;
            if value <= 0 {
                return Err("search `max_preview_bytes` must be a positive integer".to_string());
            }
            caps.max_preview_bytes = value as usize;
        }
        Ok(caps)
    }

    /// Run the language-pluggable lint check for an edited file and return
    /// `(lint_status, findings)` (ACI4).
    ///
    /// Rust files run `rustfmt --check` through the bounded runtime runner;
    /// non-Rust files report `skipped`. A clean check reports `passed`; findings
    /// report `failed` so the loop knows to reflect and repair.
    fn lint_edited(
        &self,
        path: &std::path::Path,
        request: &WrapperToolRequest,
    ) -> (String, Vec<LintFinding>) {
        let Some(linter) = Linter::for_path(path) else {
            return ("skipped".to_string(), Vec::new());
        };
        let (program, argv) = linter.command(&path.display().to_string());
        let outcome = match self
            .bounded_runtime_runner()
            .start_process(LocalProcessRequest {
                run_id: sanitized_run_id(&request.run_id),
                turn_id: None,
                program,
                argv,
                cwd: self.config.workspace_root.clone(),
                env: HashMap::new(),
            }) {
            Ok(outcome) => outcome,
            // A linter that cannot be spawned (not installed) is not a patch
            // failure: the edit already landed. Report it as unavailable so the
            // loop is not misled into thinking the code passed lint.
            Err(error) => {
                return (
                    "unavailable".to_string(),
                    vec![LintFinding {
                        file: path.display().to_string(),
                        line: 0,
                        rule: "lint".to_string(),
                        message: format!("lint runner unavailable: {error:?}"),
                    }],
                );
            }
        };
        let stderr = fs::read_to_string(&outcome.stderr.path).unwrap_or_default();
        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap_or_default();
        let combined = format!("{stdout}\n{stderr}");
        let findings = linter.parse(&path.display().to_string(), outcome.exit_code, &combined);
        let status = if findings.is_empty() {
            "passed".to_string()
        } else {
            "failed".to_string()
        };
        (status, findings)
    }

    /// Build the typed `no_match` execution for a structured, retryable
    /// apply_patch miss (ACI4). It made no change and produced no artifact, so it
    /// is NOT audited as a completed call (`reached_completion: false`).
    ///
    /// The nearest-candidate preview is a window of the TARGET FILE's own
    /// content, so it is scrubbed with the same [`Self::redact_bytes`] every
    /// other content surface uses before it reaches the operator/loop-facing
    /// summary -- otherwise a configured secret sitting next to the agent's
    /// near-miss search block would leak into the summary in the clear.
    fn no_match_execution(&self, path: &str, no_match: &NoMatch) -> WrapperExecution {
        let redacted_preview = no_match.nearest_preview.as_deref().map(|preview| {
            String::from_utf8_lossy(&self.redact_bytes(preview.as_bytes())).into_owned()
        });
        let typed_output = serde_json::json!({
            "status": "no_match",
            "path": path,
            "hunks_total": 0,
            "hunks_applied": 0,
            "hunks_rejected": 1,
            "changed_line_ranges": Vec::<Value>::new(),
            "output_artifact_id": "none",
            "lint_status": "skipped",
            "lint_findings": Vec::<Value>::new(),
            "rejected_hunk_index": no_match.hunk_index as i64,
            "reject_reason": no_match.reason.clone(),
            "nearest_line": no_match.nearest_start_line.map(|line| line as i64),
            "nearest_preview": redacted_preview.clone(),
        });
        let preview = redacted_preview
            .as_deref()
            .map(|preview| format!("; nearest candidate:\n{preview}"))
            .unwrap_or_default();
        WrapperExecution {
            status: "no_match".to_string(),
            summary: format!(
                "apply_patch hunk {} did not match {} ({}; similarity {:.2}){preview}",
                no_match.hunk_index, path, no_match.reason, no_match.nearest_similarity
            ),
            typed_output,
            output_artifacts: Vec::new(),
            reached_completion: false,
        }
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

    /// A runner bounded by the configured `artifact_limit_bytes` ceiling (ACI3).
    ///
    /// Execution wrappers want the FULL output preserved in the artifact and a
    /// `truncated` marker in the typed result (rather than a hard
    /// `OutputLimitExceeded` failure that discards the artifacts) whenever output
    /// merely exceeds the small inline `output_limit_bytes` cap. So the runner's
    /// cap is raised to the much larger `artifact_limit_bytes` ceiling -- but it
    /// stays a REAL bound: `start_process` buffers the whole child stdout/stderr
    /// in memory and then persists it, so a runaway command (`yes`) that blows
    /// past the ceiling is rejected with `OutputLimitExceeded` rather than
    /// filling memory/disk. The wrapper then compares the (bounded) artifact size
    /// against the inline `output_limit_bytes` cap to decide `truncated`. The cap
    /// is never `usize::MAX`.
    fn bounded_runtime_runner(&self) -> LocalProcessRunner {
        let mut config = LocalProcessConfig::for_test(
            self.config.workspace_root.clone(),
            self.config.artifact_root.clone(),
        );
        config.env_allowlist = self.config.env_allowlist.clone();
        config.redaction_rules = self.config.redaction_rules.clone();
        config.output_limit_bytes = self.config.artifact_limit_bytes;
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

/// Narrow typed output shape for `apply_patch`: the patched `path`, the patch
/// `status`, the per-hunk accounting (`hunks_total`/`hunks_applied`/
/// `hunks_rejected`), the changed line ranges, the unified-diff
/// `output_artifact_id`, the lint outcome (`lint_status`, `lint_findings`), and
/// the structured no-match fields surfaced on a rejected hunk (ACI4).
pub(crate) const APPLY_PATCH_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"path\":\"string\",\"hunks_total\":\"integer\",\"hunks_applied\":\"integer\",\"hunks_rejected\":\"integer\",\"changed_line_ranges\":\"string[]\",\"output_artifact_id\":\"string\",\"lint_status\":\"string\",\"lint_findings\":\"array\",\"rejected_hunk_index\":\"integer?\",\"reject_reason\":\"string?\",\"nearest_line\":\"integer?\",\"nearest_preview\":\"string?\"}}";

/// Narrow typed output shape for `capo.search` (ACI5): the bounded, decision-grade
/// `matches` array (each `{path, line, preview}`), the `returned_matches` /
/// `total_matches` counts, an explicit `truncated` marker with a
/// `truncation_reason`, and the wall-clock `duration_ms`. Full file content is
/// never inlined -- only the capped `path:line:preview` matches are returned.
pub(crate) const SEARCH_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"query\":\"string\",\"matches\":\"array\",\"returned_matches\":\"integer\",\"total_matches\":\"integer\",\"truncated\":\"boolean\",\"truncation_reason\":\"string\",\"duration_ms\":\"integer\"}}";

/// Narrow typed output shape for `capo.test_run` / `capo.check` (ACI6): the run
/// `command`, the `exit_status` (null on a signal), the `passed` interpretation,
/// the BOUNDED `failing_items` list (with its pre-cap `failing_items_total` and a
/// `failing_items_truncated` marker), the wall-clock timing
/// (`started_at`/`completed_at` as millis-since-epoch and the measured
/// `duration_ms`), and the `output_artifact_id` that carries the full redacted
/// output. The full log is never inlined -- only the capped `failing_items`. This
/// is typed EVIDENCE only; `safety-gates`' `VerificationRunner` owns `score_run`.
pub(crate) const TEST_RUN_OUTPUT_SCHEMA: &str = "{\"output\":{\"status\":\"string\",\"command\":\"string\",\"exit_status\":\"integer?\",\"passed\":\"boolean\",\"failing_items\":\"string[]\",\"failing_items_total\":\"integer\",\"failing_items_truncated\":\"boolean\",\"duration_ms\":\"integer\",\"started_at\":\"integer\",\"completed_at\":\"integer\",\"output_artifact_id\":\"string\"}}";

/// The declared `output_schema` descriptor for a runtime wrapper tool (ACI3).
pub(crate) fn wrapper_output_schema(tool_id: &str) -> &'static str {
    match tool_id {
        "capo.shell_run" | "capo.git_status" | "capo.git_diff" | "capo.git_commit" => {
            EXEC_OUTPUT_SCHEMA
        }
        "capo.file_write" => FILE_WRITE_OUTPUT_SCHEMA,
        "capo.apply_patch" => APPLY_PATCH_OUTPUT_SCHEMA,
        "capo.search" => SEARCH_OUTPUT_SCHEMA,
        "capo.test_run" => TEST_RUN_OUTPUT_SCHEMA,
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
        "capo.apply_patch" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"output_artifact_id\"]}".to_string()
        }
        "capo.search" => "{\"strategy\":\"credential_scan\",\"fields\":[\"preview\"]}".to_string(),
        "capo.test_run" => {
            "{\"strategy\":\"credential_scan\",\"fields\":[\"failing_items\",\"output_artifact_id\"]}"
                .to_string()
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
    /// Whether this execution actually completed a unit of work (wrote output /
    /// produced an artifact). When `false` (e.g. a `precondition_failed` write
    /// that made no change and produced no artifact), the call is NOT audited as
    /// a completed call: it must not emit `tool.output_artifact_recorded` /
    /// `tool.call_completed`, so downstream consumers never mis-bucket a no-op
    /// terminal outcome as a successful completion (ACI3).
    reached_completion: bool,
}

impl WrapperExecution {
    /// A successful execution that produced output/artifacts and is audited as a
    /// completed call.
    fn completed(
        status: String,
        summary: String,
        typed_output: Value,
        output_artifacts: Vec<WrapperArtifact>,
    ) -> Self {
        Self {
            status,
            summary,
            typed_output,
            output_artifacts,
            reached_completion: true,
        }
    }
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

/// Parse the `hunks` input of a `capo.apply_patch` request into typed
/// search/replace hunks (ACI4). Each hunk is an object with a `search` and a
/// `replace` string. At least one hunk is required.
fn parse_patch_hunks(request: &WrapperToolRequest) -> Result<Vec<PatchHunk>, String> {
    let Some(Value::Array(items)) = request.input.get("hunks") else {
        return Err("apply_patch requires a `hunks` array".to_string());
    };
    if items.is_empty() {
        return Err("apply_patch requires at least one hunk".to_string());
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let search = item
                .get("search")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("apply_patch hunk {index} requires a string `search`"))?;
            let replace = item
                .get("replace")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("apply_patch hunk {index} requires a string `replace`"))?;
            Ok(PatchHunk {
                search: search.to_string(),
                replace: replace.to_string(),
            })
        })
        .collect()
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

/// Wall-clock millis-since-epoch for `started_at`/`completed_at` (ACI6).
///
/// The workspace carries no date/time crate, so the typed test/check record uses
/// a millis-since-epoch integer for its wall-clock timestamps (consistent with
/// the integer `duration_ms`); the `safety-gates` `VerificationRunner` consumes
/// these for later evaluation. A clock before the epoch (impossible in practice)
/// is clamped to 0 rather than panicking.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Render a `program`/`argv` pair into a single display `command` string for the
/// typed test/check record (ACI6). Arguments are space-joined; this is a
/// human-readable echo, not a shell-quoted re-executable string.
fn render_command(program: &str, argv: &[String]) -> String {
    if argv.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", argv.join(" "))
    }
}

/// Resolve the ripgrep program for `capo.search` to an ABSOLUTE path (ACI5).
///
/// The bounded runtime runner clears the environment (no inherited `PATH`), so a
/// bare `rg` would only resolve against the OS default path and miss a
/// Homebrew/cargo install. Resolve against the current process `PATH` up front,
/// like the linter does, so ripgrep is found deterministically; fall back to the
/// bare name (the runner then reports the spawn failure) if nothing resolves.
fn resolve_search_program() -> String {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("rg");
            if candidate.is_file() {
                return candidate.display().to_string();
            }
        }
    }
    "rg".to_string()
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
