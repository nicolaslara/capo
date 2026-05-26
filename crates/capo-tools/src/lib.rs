//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 adds the first Capo-owned tool registry and an auditable invocation
//! lifecycle. Permission policy remains a separate boundary even when the
//! trusted local prototype allows broadly.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, ToolCallId};
use capo_runtime::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, RedactionRule, RuntimeError,
    RuntimeOutputArtifact,
};
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

/// First Capo-governed wrapper tools for local execution and workspace access.
pub const CAPO_WRAPPER_TOOLS: &[&str] = &[
    "capo.shell_run",
    "capo.git_status",
    "capo.git_diff",
    "capo.git_commit",
    "capo.file_read",
    "capo.file_write",
    "capo.workpad_read",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Capo(CapoToolRegistry),
    Runtime(RuntimeToolWrappers),
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

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Capo(exposure) => exposure.binding(),
            Self::Runtime(exposure) => exposure.binding(),
            Self::Fake(exposure) => exposure.binding(),
        }
    }

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        match self {
            Self::Capo(_) => FakeToolExposure.invoke(request),
            Self::Runtime(_) => FakeToolExposure.invoke(request),
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
    pub schema_json: String,
    pub required_scopes_json: String,
    pub risk: String,
    pub exposure: String,
    pub instrumentation_level: String,
    pub status: String,
    pub mutates_state: bool,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrapperToolRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: capo_core::RunId,
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
    fn denied(
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

struct WrapperExecution {
    status: String,
    summary: String,
    output_artifacts: Vec<WrapperArtifact>,
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
                "tool:invoke:capo.file_read",
                "tool:invoke:capo.git_status",
                "tool:invoke:capo.git_diff",
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
                "tool:invoke:capo.file_read",
                "tool:invoke:capo.git_status",
                "tool:invoke:capo.git_diff",
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

fn is_workpad_path(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return false;
    }
    let normalized = path.display().to_string();
    normalized == "TASKS.md"
        || normalized == "project.md"
        || (normalized.starts_with("workpads/")
            && normalized.ends_with(".md")
            && !normalized.contains("/research-clones/")
            && !normalized.contains("/scratch/"))
}

fn workspace_path(workspace_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn workspace_relative_path(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "git path must be workspace-relative: {}",
            path.display()
        ));
    }
    Ok(path.display().to_string())
}

fn sanitize_path_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "tool-call".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitized_run_id(run_id: &RunId) -> RunId {
    RunId::new(sanitize_path_component(run_id.as_str()))
}

fn ensure_under_workspace(path: &Path, workspace_root: &Path) -> Result<(), String> {
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if path.starts_with(&workspace_root) {
        Ok(())
    } else {
        Err(format!(
            "path escapes workspace: {} not under {}",
            path.display(),
            workspace_root.display()
        ))
    }
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cursor = path.parent();
    while let Some(parent) = cursor {
        if parent.exists() {
            return Some(parent.to_path_buf());
        }
        cursor = parent.parent();
    }
    None
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

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
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
    use capo_core::RunId;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn runtime_wrappers_define_shell_git_file_and_workpad_tools() {
        let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-workspace"),
            PathBuf::from("/tmp/capo-artifacts"),
        ));
        let tools = wrappers.list_tools();

        assert_eq!(tools.len(), 7);
        for tool_id in CAPO_WRAPPER_TOOLS {
            let definition = wrappers.describe_tool(tool_id).expect("wrapper definition");
            assert_eq!(definition.origin, "runtime");
            assert_eq!(definition.handler_kind, "runtime_wrapper");
            assert_eq!(definition.instrumentation_level, "full");
            assert!(
                definition
                    .required_scopes_json
                    .contains(&format!("tool:invoke:{tool_id}"))
            );
        }
        assert_eq!(
            wrappers
                .describe_tool("capo.shell_run")
                .expect("shell tool")
                .risk,
            "high"
        );
        assert!(
            wrappers
                .describe_tool("capo.git_commit")
                .expect("git commit tool")
                .mutates_state
        );
        assert_eq!(
            wrappers
                .describe_tool("capo.git_commit")
                .expect("git commit tool")
                .risk,
            "high"
        );
        assert!(
            wrappers
                .describe_tool("capo.file_write")
                .expect("file write tool")
                .mutates_state
        );
    }

    #[test]
    fn acp_client_capabilities_require_wrappers_and_policy_allow() {
        let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-workspace"),
            PathBuf::from("/tmp/capo-artifacts"),
        ));

        let trusted = AcpClientCapabilityPlan::from_runtime_wrappers(
            &wrappers,
            &PermissionPolicy::allow_trusted_local(),
            SessionId::new("session-acp-trusted"),
        );
        assert_eq!(
            trusted.advertised_capabilities(),
            vec![
                "filesystem.read_text_file",
                "filesystem.write_text_file",
                "terminal"
            ]
        );
        assert_eq!(
            trusted.filesystem_read.reason,
            "backing_wrapper_tool_allowed"
        );
        assert_eq!(trusted.terminal.permission_effect.as_deref(), Some("allow"));

        let read_only = AcpClientCapabilityPlan::from_runtime_wrappers(
            &wrappers,
            &PermissionPolicy::static_read_only_local(),
            SessionId::new("session-acp-read-only"),
        );
        assert!(read_only.filesystem_read.advertise);
        assert!(!read_only.filesystem_write.advertise);
        assert!(!read_only.terminal.advertise);
        assert_eq!(
            read_only.advertised_capabilities(),
            vec!["filesystem.read_text_file"]
        );
        assert_eq!(
            read_only.filesystem_write.permission_effect.as_deref(),
            Some("deny")
        );
        assert!(
            read_only
                .terminal
                .reason
                .contains("permission_policy_rejected")
        );
    }

    #[test]
    fn acp_client_capabilities_fail_closed_without_backing_wrappers() {
        let definitions = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-workspace"),
            PathBuf::from("/tmp/capo-artifacts"),
        ))
        .list_tools()
        .into_iter()
        .filter(|definition| definition.tool_id != "capo.shell_run")
        .collect::<Vec<_>>();

        let plan = AcpClientCapabilityPlan::from_tool_definitions(
            &definitions,
            &PermissionPolicy::allow_trusted_local(),
            SessionId::new("session-acp-missing-wrapper"),
        );

        assert!(!plan.terminal.advertise);
        assert_eq!(plan.terminal.reason, "missing_backing_wrapper_tool");
        assert_eq!(plan.terminal.required_scopes_json, None);
        assert_eq!(plan.terminal.permission_effect, None);
        assert!(plan.filesystem_read.advertise);
    }

    #[test]
    fn file_wrappers_record_input_output_artifacts_and_reject_workspace_escape() {
        let workspace = temp_root("tool-wrapper-workspace");
        let artifacts = temp_root("tool-wrapper-artifacts");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::write(workspace.join("note.md"), "hello").expect("write note");
        let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            workspace.clone(),
            artifacts.clone(),
        ));
        let policy = PermissionPolicy::allow_trusted_local();

        let read = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-file-read",
                "run-file-read",
                "capo.file_read",
                serde_json::json!({"path":"note.md"}),
            ),
            &policy,
        );
        assert_eq!(read.status, "completed");
        assert!(read.input_artifact.is_some());
        assert_eq!(read.output_artifacts.len(), 1);
        assert_eq!(
            fs::read_to_string(&read.output_artifacts[0].uri).expect("read artifact"),
            "hello"
        );
        assert!(read.events.iter().any(|event| {
            event.kind == "tool.output_artifact_recorded" && event.status == "safe"
        }));

        let write = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-file-write",
                "run-file-write",
                "capo.file_write",
                serde_json::json!({"path":"nested/out.txt","content":"new text"}),
            ),
            &policy,
        );
        assert_eq!(write.status, "completed");
        assert_eq!(
            fs::read_to_string(workspace.join("nested/out.txt")).expect("written file"),
            "new text"
        );
        assert_eq!(write.output_artifacts[0].kind, "file_write_diff");
        assert!(
            fs::read_to_string(&write.output_artifacts[0].uri)
                .expect("diff summary")
                .contains("before=fnv1a64:")
        );

        let escaped = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-file-escape",
                "run-file-escape",
                "capo.file_read",
                serde_json::json!({"path":"../outside.txt"}),
            ),
            &policy,
        );
        assert_eq!(escaped.status, "failed");
        assert!(escaped.summary.contains("workspace path does not exist"));

        let workpad_escape = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-workpad-escape",
                "run-workpad-escape",
                "capo.workpad_read",
                serde_json::json!({"path":"note.md"}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );
        assert_eq!(workpad_escape.status, "failed");
        assert!(
            workpad_escape
                .summary
                .contains("workpad_read only supports")
        );

        fs::create_dir_all(workspace.join("workpads/features")).expect("workpad dir");
        fs::write(workspace.join("workpads/features/tasks.md"), "# Tasks\n").expect("workpad");
        let workpad = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-workpad-read",
                "run-workpad-read",
                "capo.workpad_read",
                serde_json::json!({"path":"workpads/features/tasks.md"}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );
        assert_eq!(workpad.status, "completed");
        assert_eq!(workpad.output_artifacts[0].kind, "workpad_read");

        let denied = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-file-write-denied",
                "run-file-write-denied",
                "capo.file_write",
                serde_json::json!({"path":"denied.txt","content":"nope"}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );
        assert_eq!(denied.status, "denied");
        assert!(denied.output_artifacts.is_empty());
        assert!(denied.events.iter().any(|event| {
            event.kind == "tool.call_canceled" && event.status == "permission_denied"
        }));
    }

    #[test]
    fn wrapper_split_authorization_cannot_be_replayed_for_another_tool() {
        let workspace = temp_root("tool-wrapper-replay-workspace");
        let artifacts = temp_root("tool-wrapper-replay-artifacts");
        fs::create_dir_all(&workspace).expect("workspace");
        let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            workspace.clone(),
            artifacts,
        ));
        let authorization = wrappers.authorize_tool_call(
            &wrapper_request(
                "call-status-auth",
                "run-status-auth",
                "capo.git_status",
                serde_json::json!({}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );

        let replay = wrappers.invoke_authorized(
            wrapper_request(
                "call-shell-replay",
                "run-shell-replay",
                "capo.shell_run",
                serde_json::json!({"program":"/bin/sh","argv":["-c","touch replayed"]}),
            ),
            authorization,
        );

        assert_eq!(replay.status, "denied");
        assert!(replay.summary.contains("authorization tool mismatch"));
        assert!(!workspace.join("replayed").exists());

        let shell_authorization = wrappers.authorize_tool_call(
            &wrapper_request(
                "call-shell-auth",
                "run-shell-auth",
                "capo.shell_run",
                serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
            ),
            &PermissionPolicy::allow_trusted_local(),
        );
        let changed_input = wrappers.invoke_authorized(
            wrapper_request(
                "call-shell-auth",
                "run-shell-auth",
                "capo.shell_run",
                serde_json::json!({"program":"/bin/sh","argv":["-c","touch replayed"]}),
            ),
            shell_authorization,
        );
        assert_eq!(changed_input.status, "denied");
        assert!(
            changed_input
                .summary
                .contains("authorization input mismatch")
        );
        assert!(!workspace.join("replayed").exists());
    }

    #[test]
    fn shell_and_git_wrappers_execute_through_runtime_with_artifacts() {
        let workspace = temp_root("tool-wrapper-git-workspace");
        let artifacts = temp_root("tool-wrapper-git-artifacts");
        fs::create_dir_all(&workspace).expect("workspace");
        Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .expect("git init");
        fs::write(workspace.join("tracked.txt"), "tracked\n").expect("write tracked");

        let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
        config.redaction_rules.push(RedactionRule {
            pattern: "SECRET".to_string(),
            replacement: "[REDACTED]".to_string(),
        });
        let wrappers = RuntimeToolWrappers::new(config);
        let policy = PermissionPolicy::allow_trusted_local();

        let shell = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-shell",
                "run-shell",
                "capo.shell_run",
                serde_json::json!({
                    "program":"/bin/sh",
                    "argv":["-c","printf SECRET"],
                    "cwd":"."
                }),
            ),
            &policy,
        );
        assert_eq!(shell.status, "exited");
        let shell_input = shell.input_artifact.as_ref().expect("shell input");
        assert_eq!(shell_input.redaction_state, "redacted");
        assert!(
            fs::read_to_string(&shell_input.uri)
                .expect("shell input artifact")
                .contains("[REDACTED]")
        );
        assert_eq!(shell.output_artifacts.len(), 2);
        assert!(
            shell
                .output_artifacts
                .iter()
                .any(|artifact| artifact.redaction_state == "redacted")
        );
        assert!(
            shell
                .events
                .iter()
                .any(|event| event.kind == "capability.grant_used")
        );

        let git_status = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-git-status",
                "run-git-status",
                "capo.git_status",
                serde_json::json!({}),
            ),
            &policy,
        );
        assert_eq!(git_status.status, "exited");
        let stdout = git_status
            .output_artifacts
            .iter()
            .find(|artifact| artifact.kind == "git_stdout")
            .expect("git stdout");
        assert!(
            fs::read_to_string(&stdout.uri)
                .expect("git stdout artifact")
                .contains("tracked.txt")
        );

        let denied_shell = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-shell-denied",
                "run-shell-denied",
                "capo.shell_run",
                serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );
        assert_eq!(denied_shell.status, "denied");
        assert!(
            !denied_shell
                .events
                .iter()
                .any(|event| event.kind == "tool.invocation_started")
        );

        let escaped_artifact = wrappers.authorize_and_invoke(
            wrapper_request(
                "../call-shell-escape",
                "../run-shell-escape",
                "capo.shell_run",
                serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
            ),
            &policy,
        );
        assert_eq!(escaped_artifact.status, "exited");
        assert!(
            !workspace
                .parent()
                .expect("workspace parent")
                .join("call-shell-escape")
                .exists()
        );
        assert!(
            !workspace
                .parent()
                .expect("workspace parent")
                .join("run-shell-escape")
                .exists()
        );
    }

    #[test]
    fn git_commit_wrapper_commits_staged_changes_and_denies_static_profiles() {
        let workspace = temp_root("tool-wrapper-git-commit-workspace");
        let artifacts = temp_root("tool-wrapper-git-commit-artifacts");
        fs::create_dir_all(&workspace).expect("workspace");
        Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .expect("git init");
        fs::write(workspace.join("tracked.txt"), "tracked\n").expect("write tracked");
        Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&workspace)
            .output()
            .expect("git add");

        let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
            workspace.clone(),
            artifacts,
        ));

        let commit = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-git-commit",
                "run-git-commit",
                "capo.git_commit",
                serde_json::json!({"message":"Capo wrapper commit"}),
            ),
            &PermissionPolicy::allow_trusted_local(),
        );
        assert_eq!(commit.status, "exited");
        assert!(commit.input_artifact.is_some());
        assert_eq!(commit.output_artifacts.len(), 2);
        assert!(
            commit
                .output_artifacts
                .iter()
                .any(|artifact| artifact.kind == "git_commit_stdout")
        );
        assert!(
            commit
                .events
                .iter()
                .any(|event| event.kind == "tool.invocation_started")
        );
        let log = Command::new("git")
            .args(["log", "--oneline", "-1"])
            .current_dir(&workspace)
            .output()
            .expect("git log");
        assert!(
            String::from_utf8_lossy(&log.stdout).contains("Capo wrapper commit"),
            "git log should show wrapper commit"
        );

        fs::write(workspace.join("denied.txt"), "denied\n").expect("write denied");
        Command::new("git")
            .args(["add", "denied.txt"])
            .current_dir(&workspace)
            .output()
            .expect("git add denied");
        let denied = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-git-commit-denied",
                "run-git-commit-denied",
                "capo.git_commit",
                serde_json::json!({"message":"Denied commit"}),
            ),
            &PermissionPolicy::static_read_only_local(),
        );
        assert_eq!(denied.status, "denied");
        assert!(denied.output_artifacts.is_empty());
        assert!(denied.summary.contains("git:commit:workspace"));
        assert!(
            !denied
                .events
                .iter()
                .any(|event| event.kind == "tool.invocation_started")
        );

        let reviewer_denied = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-git-commit-reviewer-denied",
                "run-git-commit-reviewer-denied",
                "capo.git_commit",
                serde_json::json!({"message":"Reviewer denied commit"}),
            ),
            &PermissionPolicy::static_reviewer(),
        );
        assert_eq!(reviewer_denied.status, "denied");
        assert!(reviewer_denied.summary.contains("git:commit:workspace"));

        let missing_message = wrappers.authorize_and_invoke(
            wrapper_request(
                "call-git-commit-empty",
                "run-git-commit-empty",
                "capo.git_commit",
                serde_json::json!({"message":"   "}),
            ),
            &PermissionPolicy::allow_trusted_local(),
        );
        assert_eq!(missing_message.status, "failed");
        assert!(
            missing_message
                .summary
                .contains("git_commit requires a non-empty message")
        );
        assert!(missing_message.output_artifacts.is_empty());
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
    fn capo_registry_split_authorization_cannot_be_replayed_with_changed_context() {
        let registry = CapoToolRegistry;
        let request = CapoToolRequest {
            tool_call_id: ToolCallId::new("call-evidence"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.evidence_record".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            context: tool_context(),
        };
        let authorization =
            registry.authorize_tool_call(&request, &PermissionPolicy::allow_trusted_local());
        let replay = registry.invoke_authorized(
            CapoToolRequest {
                context: CapoToolContext {
                    evidence_note: "different evidence".to_string(),
                    ..tool_context()
                },
                ..request
            },
            authorization,
        );

        assert_eq!(replay.output, "authorization input mismatch");
        assert_eq!(replay.output_artifact_id, "none");
        assert!(replay.events.iter().any(|event| {
            event.kind == "tool.call_canceled" && event.status == "authorization_mismatch"
        }));

        let ambiguous = CapoToolRequest {
            tool_call_id: ToolCallId::new("call-ambiguous"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.evidence_record".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            context: CapoToolContext {
                task_status: "a\nb".to_string(),
                agent_status: "c".to_string(),
                session_summary: "summary text".to_string(),
                workpad_excerpt: "workpad section".to_string(),
                evidence_note: "tests passed".to_string(),
                capability_scope: "shell:execute:workspace".to_string(),
            },
        };
        let ambiguous_authorization =
            registry.authorize_tool_call(&ambiguous, &PermissionPolicy::allow_trusted_local());
        let ambiguous_replay = registry.invoke_authorized(
            CapoToolRequest {
                context: CapoToolContext {
                    task_status: "a".to_string(),
                    agent_status: "b\nc".to_string(),
                    session_summary: "summary text".to_string(),
                    workpad_excerpt: "workpad section".to_string(),
                    evidence_note: "tests passed".to_string(),
                    capability_scope: "shell:execute:workspace".to_string(),
                },
                ..ambiguous
            },
            ambiguous_authorization,
        );
        assert_eq!(ambiguous_replay.output, "authorization input mismatch");
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

    fn wrapper_request(
        tool_call_id: &str,
        run_id: &str,
        tool_id: &str,
        input: Value,
    ) -> WrapperToolRequest {
        WrapperToolRequest {
            tool_call_id: ToolCallId::new(tool_call_id),
            session_id: SessionId::new("session-wrapper"),
            run_id: RunId::new(run_id),
            tool_id: tool_id.to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            input,
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-tools-{name}-{nanos}"))
    }
}
