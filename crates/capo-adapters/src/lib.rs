//! Agent adapter and provider connector scaffolding.
//!
//! P6 adds fixture parsers for Codex, Claude Code, and ACP streams. The
//! parsers preserve provider-specific records as adapter facts and emit
//! normalized adapter events for the controller pipeline.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, ToolCallId, TurnId};
use capo_runtime::{
    LocalProcessConfig, LocalProcessOutcome, LocalProcessRequest, LocalProcessRunner,
    RedactionRule, RuntimeError,
};
use capo_tools::{
    AcpClientCapabilityDecision, AcpClientCapabilityPlan, PermissionPolicy, ToolDefinition,
    WrapperToolRequest,
};
use serde_json::Value;

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] = &["fake", "codex-exec", "claude-code", "acp"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentAdapter {
    Fake(FakeAdapter),
}

impl AgentAdapter {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(adapter) => adapter.binding(),
        }
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.open_session(request),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.send_turn(session, request),
        }
    }

    pub fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.attach_session(session_id, external_session_ref),
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.interrupt(session, reason),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.stop(session, reason),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapter;

impl FakeAdapter {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::AgentAdapter, "fake-adapter")
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        FakeAdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("fake-adapter-session-{}", request.agent_name),
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: request.turn_id,
            external_session_ref: session.external_session_ref.clone(),
            summary: format!(
                "Fake adapter processed goal for {}: {}",
                request.agent_name, request.goal
            ),
            confidence: 82,
            status: "active".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    pub fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> FakeAdapterSession {
        FakeAdapterSession {
            session_id,
            external_session_ref,
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter interrupted session: {reason}"),
            confidence: 70,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter stopped session: {reason}"),
            confidence: 70,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterSessionRequest {
    pub session_id: SessionId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterSession {
    pub session_id: SessionId,
    pub external_session_ref: String,
    pub adapter_capability: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterTurnRequest {
    pub turn_id: TurnId,
    pub agent_name: String,
    pub goal: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterTurnOutput {
    pub turn_id: TurnId,
    pub external_session_ref: String,
    pub summary: String,
    pub confidence: i64,
    pub status: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderConnector {
    Fake(FakeProviderConnector),
}

impl ProviderConnector {
    pub fn fake() -> Self {
        Self::Fake(FakeProviderConnector)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(connector) => connector.binding(),
        }
    }

    pub fn describe_provider(&self) -> FakeProviderInfo {
        match self {
            Self::Fake(connector) => connector.describe_provider(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeProviderConnector;

impl FakeProviderConnector {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ProviderConnector, "fake-provider")
    }

    pub fn describe_provider(&self) -> FakeProviderInfo {
        FakeProviderInfo {
            provider_kind: "fake".to_string(),
            auth_mode: "none".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeProviderInfo {
    pub provider_kind: String,
    pub auth_mode: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterLaunchPlan {
    pub adapter_kind: NormalizedAdapterKind,
    pub provider_kind: String,
    pub credential_scope: String,
    pub program: String,
    pub argv: Vec<String>,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
    pub stdout_format: String,
    pub stderr_policy: String,
}

impl LocalAdapterLaunchPlan {
    pub fn runtime_config(&self) -> LocalProcessConfig {
        LocalProcessConfig {
            workspace_roots: vec![self.workspace_root.clone()],
            artifact_root: self.artifact_root.clone(),
            env_allowlist: self.env_allowlist.clone(),
            redaction_rules: self.redaction_rules.clone(),
            output_limit_bytes: self.output_limit_bytes,
        }
    }

    pub fn runtime_request(&self, run_id: RunId) -> LocalProcessRequest {
        LocalProcessRequest {
            run_id,
            program: self.program.clone(),
            argv: self.argv.clone(),
            cwd: self.workspace_root.clone(),
            env: HashMap::new(),
        }
    }

    pub fn assert_subscription_safe(&self) -> Result<(), String> {
        if self.credential_scope != "user_local_subscription" {
            return Err(format!(
                "unsupported credential scope for local subscription launch: {}",
                self.credential_scope
            ));
        }
        if self.env_allowlist.iter().any(|name| {
            let upper = name.to_ascii_uppercase();
            upper.contains("TOKEN")
                || upper.contains("KEY")
                || upper.contains("SECRET")
                || upper.contains("COOKIE")
        }) {
            return Err(
                "local subscription launch env allowlist includes secret-like names".into(),
            );
        }
        if self.argv.iter().any(|arg| sensitive_marker(arg).is_some()) {
            return Err("local subscription launch argv includes secret-like markers".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterSmokePlan {
    pub adapter_kind: NormalizedAdapterKind,
    pub opt_in_env: &'static str,
    pub program: String,
    pub argv: Vec<String>,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
    pub expected_output_marker: &'static str,
}

impl LocalAdapterSmokePlan {
    pub fn runtime_config(&self) -> LocalProcessConfig {
        LocalProcessConfig {
            workspace_roots: vec![self.workspace_root.clone()],
            artifact_root: self.artifact_root.clone(),
            env_allowlist: self.env_allowlist.clone(),
            redaction_rules: self.redaction_rules.clone(),
            output_limit_bytes: self.output_limit_bytes,
        }
    }

    pub fn runtime_request(&self, run_id: RunId) -> LocalProcessRequest {
        LocalProcessRequest {
            run_id,
            program: self.program.clone(),
            argv: self.argv.clone(),
            cwd: self.workspace_root.clone(),
            env: HashMap::new(),
        }
    }

    pub fn is_opted_in(&self) -> bool {
        std::env::var(self.opt_in_env).as_deref() == Ok("1")
    }
}

#[derive(Debug)]
pub enum LocalAdapterSmokeError {
    Io(std::io::Error),
    Runtime(RuntimeError),
    NotOptedIn(&'static str),
    SensitiveArtifact { path: PathBuf, marker: String },
    MarkerMissing { marker: &'static str },
}

impl From<std::io::Error> for LocalAdapterSmokeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RuntimeError> for LocalAdapterSmokeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

pub type LocalAdapterSmokeResult<T> = Result<T, LocalAdapterSmokeError>;

pub struct LocalAdapterSmokeRunner;

impl LocalAdapterSmokeRunner {
    pub fn run_if_opted_in(
        plan: &LocalAdapterSmokePlan,
    ) -> LocalAdapterSmokeResult<Option<LocalProcessOutcome>> {
        if !plan.is_opted_in() {
            return Ok(None);
        }

        Self::run(plan).map(Some)
    }

    pub fn run(plan: &LocalAdapterSmokePlan) -> LocalAdapterSmokeResult<LocalProcessOutcome> {
        fs::create_dir_all(&plan.workspace_root)?;
        fs::create_dir_all(&plan.artifact_root)?;
        let runner = LocalProcessRunner::new(plan.runtime_config());
        let outcome = runner.start_process(
            plan.runtime_request(RunId::new(format!("{}-smoke", plan.adapter_kind.as_str()))),
        )?;
        scan_artifacts_for_sensitive_markers([&outcome.stdout.path, &outcome.stderr.path])?;
        let stdout = fs::read_to_string(&outcome.stdout.path)?;
        let stderr = fs::read_to_string(&outcome.stderr.path)?;
        if !stdout.contains(plan.expected_output_marker)
            && !stderr.contains(plan.expected_output_marker)
        {
            return Err(LocalAdapterSmokeError::MarkerMissing {
                marker: plan.expected_output_marker,
            });
        }
        Ok(outcome)
    }
}

impl CodexExecAdapter {
    pub fn local_launch_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        prompt: impl Into<String>,
    ) -> LocalAdapterLaunchPlan {
        LocalAdapterLaunchPlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            provider_kind: "codex_subscription".to_string(),
            credential_scope: "user_local_subscription".to_string(),
            program: "codex".to_string(),
            argv: vec![
                "exec".to_string(),
                "--json".to_string(),
                "--sandbox".to_string(),
                "read-only".to_string(),
                "--ephemeral".to_string(),
                "--ignore-user-config".to_string(),
                "--ignore-rules".to_string(),
                "--cd".to_string(),
                workspace_root.to_string_lossy().to_string(),
                prompt.into(),
            ],
            workspace_root,
            artifact_root,
            env_allowlist: local_subscription_cli_env_allowlist(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 128 * 1024,
            stdout_format: "jsonl".to_string(),
            stderr_policy: "logs_redacted".to_string(),
        }
    }

    pub fn local_smoke_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> LocalAdapterSmokePlan {
        let launch_plan = Self::local_launch_plan(
            workspace_root,
            artifact_root,
            "Reply with exactly CAPO_CODEX_SMOKE_OK and do not inspect files.",
        );
        LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            opt_in_env: "CAPO_RUN_CODEX_LOCAL_SMOKE",
            program: launch_plan.program,
            argv: launch_plan.argv,
            workspace_root: launch_plan.workspace_root,
            artifact_root: launch_plan.artifact_root,
            env_allowlist: launch_plan.env_allowlist,
            redaction_rules: launch_plan.redaction_rules,
            output_limit_bytes: launch_plan.output_limit_bytes,
            expected_output_marker: "CAPO_CODEX_SMOKE_OK",
        }
    }
}

impl ClaudeCodeAdapter {
    pub fn local_launch_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        prompt: impl Into<String>,
    ) -> LocalAdapterLaunchPlan {
        LocalAdapterLaunchPlan {
            adapter_kind: NormalizedAdapterKind::ClaudeCode,
            provider_kind: "claude_subscription".to_string(),
            credential_scope: "user_local_subscription".to_string(),
            program: "claude".to_string(),
            argv: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--permission-mode".to_string(),
                "plan".to_string(),
                "--no-session-persistence".to_string(),
                "--disable-slash-commands".to_string(),
                "--tools".to_string(),
                "".to_string(),
                "--disallowedTools".to_string(),
                "*".to_string(),
                "--mcp-config".to_string(),
                "/dev/null".to_string(),
                "--strict-mcp-config".to_string(),
                prompt.into(),
            ],
            workspace_root,
            artifact_root,
            env_allowlist: local_subscription_cli_env_allowlist(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 128 * 1024,
            stdout_format: "stream-json".to_string(),
            stderr_policy: "logs_redacted".to_string(),
        }
    }

    pub fn local_smoke_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> LocalAdapterSmokePlan {
        let launch_plan = Self::local_launch_plan(
            workspace_root,
            artifact_root,
            "Reply with exactly CAPO_CLAUDE_SMOKE_OK and do not inspect files.",
        );
        LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::ClaudeCode,
            opt_in_env: "CAPO_RUN_CLAUDE_LOCAL_SMOKE",
            program: launch_plan.program,
            argv: launch_plan.argv,
            workspace_root: launch_plan.workspace_root,
            artifact_root: launch_plan.artifact_root,
            env_allowlist: launch_plan.env_allowlist,
            redaction_rules: launch_plan.redaction_rules,
            output_limit_bytes: launch_plan.output_limit_bytes,
            expected_output_marker: "CAPO_CLAUDE_SMOKE_OK",
        }
    }
}

pub fn scan_artifacts_for_sensitive_markers<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> LocalAdapterSmokeResult<()> {
    for path in paths {
        let contents = fs::read_to_string(path)?;
        if let Some(marker) = sensitive_marker(&contents) {
            return Err(LocalAdapterSmokeError::SensitiveArtifact {
                path: path.clone(),
                marker,
            });
        }
    }
    Ok(())
}

fn local_subscription_cli_env_allowlist() -> Vec<String> {
    vec![
        "HOME".to_string(),
        "PATH".to_string(),
        "TMPDIR".to_string(),
        "USER".to_string(),
        "LOGNAME".to_string(),
        "SHELL".to_string(),
        "LANG".to_string(),
    ]
}

fn local_adapter_redaction_rules() -> Vec<RedactionRule> {
    [
        ("Authorization:", "Authorization: [REDACTED]"),
        ("Cookie:", "Cookie: [REDACTED]"),
        ("session_token", "session_[REDACTED]"),
        ("api_key", "api_[REDACTED]"),
        ("access_token", "access_[REDACTED]"),
        ("refresh_token", "refresh_[REDACTED]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| RedactionRule {
        pattern: pattern.to_string(),
        replacement: replacement.to_string(),
    })
    .collect()
}

fn sensitive_marker(contents: &str) -> Option<String> {
    for line in contents.lines() {
        if line.to_ascii_lowercase().contains("[redacted]") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(marker) = [
            "authorization:",
            "cookie:",
            "set-cookie:",
            "session_token",
            "access_token",
            "refresh_token",
            "oauth",
            "api_key",
            "anthropic_api_key",
            "openai_api_key",
            "sk-",
        ]
        .into_iter()
        .find(|marker| lower.contains(marker))
        {
            return Some(marker.to_string());
        }
    }
    None
}

pub type AdapterParseResult<T> = Result<T, AdapterParseError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterParseError {
    pub line: usize,
    pub message: String,
}

impl AdapterParseError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalizedAdapterKind {
    CodexExec,
    ClaudeCode,
    Acp,
}

impl NormalizedAdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CodexExec => "codex_exec",
            Self::ClaudeCode => "claude_code",
            Self::Acp => "acp",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterTimelineConfidence {
    Stable,
    Heuristic,
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedAdapterEvent {
    pub adapter_kind: NormalizedAdapterKind,
    pub kind: String,
    pub external_session_ref: Option<String>,
    pub external_item_ref: Option<String>,
    pub timeline_key: Option<String>,
    pub timeline_confidence: AdapterTimelineConfidence,
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_name: Option<String>,
    pub status: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub raw_event_hash: String,
    pub idempotency_key: Option<String>,
    pub provider_event_kind: String,
}

impl NormalizedAdapterEvent {
    fn new(
        adapter_kind: NormalizedAdapterKind,
        kind: impl Into<String>,
        provider_event_kind: impl Into<String>,
        raw: &Value,
    ) -> Self {
        Self {
            adapter_kind,
            kind: kind.into(),
            external_session_ref: None,
            external_item_ref: None,
            timeline_key: None,
            timeline_confidence: AdapterTimelineConfidence::None,
            role: None,
            content: None,
            tool_name: None,
            status: None,
            input_tokens: None,
            output_tokens: None,
            raw_event_hash: json_hash(raw),
            idempotency_key: None,
            provider_event_kind: provider_event_kind.into(),
        }
    }

    fn with_timeline(
        mut self,
        external_session_ref: Option<String>,
        external_item_ref: Option<String>,
        timeline_key: String,
        confidence: AdapterTimelineConfidence,
        operation: &str,
    ) -> Self {
        self.external_session_ref = external_session_ref;
        self.external_item_ref = external_item_ref;
        self.timeline_key = Some(timeline_key.clone());
        self.timeline_confidence = confidence;
        self.idempotency_key = Some(format!(
            "{}:{}:{}:{}",
            self.adapter_kind.as_str(),
            self.kind,
            timeline_key,
            operation
        ));
        self
    }

    pub fn tool_observation(&self) -> Option<AdapterToolObservation> {
        if !matches!(
            self.kind.as_str(),
            "adapter.tool_call_requested"
                | "adapter.tool_call_started"
                | "adapter.tool_call_completed"
                | "adapter.tool_call_failed"
        ) {
            return None;
        }
        Some(AdapterToolObservation {
            source_adapter: self.adapter_kind.as_str().to_string(),
            external_tool_ref: self
                .external_item_ref
                .clone()
                .or_else(|| self.timeline_key.clone()),
            tool_name: self
                .tool_name
                .clone()
                .unwrap_or_else(|| "adapter-native-tool".to_string()),
            observed_status: self
                .status
                .clone()
                .unwrap_or_else(|| "observed".to_string()),
            instrumentation_level: "observed_only".to_string(),
            confidence: match self.timeline_confidence {
                AdapterTimelineConfidence::Stable => "high",
                AdapterTimelineConfidence::Heuristic => "medium",
                AdapterTimelineConfidence::None => "low",
            }
            .to_string(),
            raw_event_hash: self.raw_event_hash.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterFixtureParse {
    pub raw_event_count: usize,
    pub events: Vec<NormalizedAdapterEvent>,
}

impl AdapterFixtureParse {
    pub fn deduped_by_idempotency(&self) -> Vec<NormalizedAdapterEvent> {
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for event in &self.events {
            match &event.idempotency_key {
                Some(key) if seen.insert(key.clone()) => deduped.push(event.clone()),
                Some(_) => {}
                None => deduped.push(event.clone()),
            }
        }
        deduped
    }

    pub fn tool_observations(&self) -> Vec<AdapterToolObservation> {
        self.deduped_by_idempotency()
            .into_iter()
            .filter_map(|event| event.tool_observation())
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterToolObservation {
    pub source_adapter: String,
    pub external_tool_ref: Option<String>,
    pub tool_name: String,
    pub observed_status: String,
    pub instrumentation_level: String,
    pub confidence: String,
    pub raw_event_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexExecAdapter;

impl CodexExecAdapter {
    pub fn parse_jsonl(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_codex_record)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn parse_stream_json(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_claude_record)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpAdapter;

impl AcpAdapter {
    pub fn parse_replay_jsonl(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_acp_record)
    }

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

fn parse_jsonl(
    input: &str,
    parser: fn(&Value) -> Vec<NormalizedAdapterEvent>,
) -> AdapterParseResult<AdapterFixtureParse> {
    let mut raw_event_count = 0;
    let mut events = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        raw_event_count += 1;
        let value: Value = serde_json::from_str(line)
            .map_err(|error| AdapterParseError::new(line_number, error.to_string()))?;
        events.extend(parser(&value));
    }
    Ok(AdapterFixtureParse {
        raw_event_count,
        events,
    })
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

fn parse_codex_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["type"]).unwrap_or_else(|| "unknown".to_string());
    let session_ref = string_at(raw, &["thread_id"]).or_else(|| string_at(raw, &["session_id"]));
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::CodexExec,
        "adapter.raw_event",
        &provider_kind,
        raw,
    );

    match provider_kind.as_str() {
        "thread.started" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("codex:{session}:session"));
        }
        "item.completed" | "item.updated" => {
            let item_ref = string_at(raw, &["item", "id"]).or_else(|| string_at(raw, &["id"]));
            let role = string_at(raw, &["item", "role"]);
            let content = text_from_content_array(raw.pointer("/item/content"));
            let timeline_key = item_ref
                .clone()
                .map(|item| format!("codex:item:{item}"))
                .unwrap_or_else(|| format!("codex:item:{}", json_hash(raw)));
            event = event.with_timeline(
                session_ref,
                item_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "upsert",
            );
            event.kind = "adapter.item_completed".to_string();
            event.role = role;
            event.content = content;
            event.status = Some("completed".to_string());
        }
        "exec_command.begin" | "exec_command.end" | "tool_call.started" | "tool_call.completed" => {
            let call_ref = string_at(raw, &["call_id"])
                .or_else(|| string_at(raw, &["tool_call_id"]))
                .or_else(|| string_at(raw, &["id"]));
            let timeline_key = call_ref
                .clone()
                .map(|call| format!("codex:tool:{call}"))
                .unwrap_or_else(|| format!("codex:tool:{}", json_hash(raw)));
            let operation =
                if provider_kind.ends_with(".begin") || provider_kind.ends_with(".started") {
                    "started"
                } else {
                    "completed"
                };
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                operation,
            );
            event.kind = if operation == "started" {
                "adapter.tool_call_started".to_string()
            } else {
                "adapter.tool_call_completed".to_string()
            };
            event.tool_name = string_at(raw, &["tool_name"])
                .or_else(|| string_at(raw, &["name"]))
                .or_else(|| Some("exec_command".to_string()));
            event.status = Some(operation.to_string());
        }
        "turn.completed" | "thread.completed" => {
            event.kind = "adapter.turn_completed".to_string();
            event.external_session_ref = session_ref;
            event.status = Some("completed".to_string());
            event.input_tokens = integer_at(raw, &["usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["usage", "output_tokens"]);
        }
        _ => {}
    }

    vec![event]
}

fn parse_claude_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["type"]).unwrap_or_else(|| "unknown".to_string());
    let session_ref =
        string_at(raw, &["session_id"]).or_else(|| string_at(raw, &["message", "session_id"]));
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::ClaudeCode,
        "adapter.raw_event",
        &provider_kind,
        raw,
    );

    match provider_kind.as_str() {
        "system" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("claude:{session}:session"));
        }
        "assistant" | "user" => {
            let item_ref = string_at(raw, &["message", "id"]).or_else(|| string_at(raw, &["id"]));
            let role = string_at(raw, &["message", "role"]).or_else(|| Some(provider_kind.clone()));
            let content = text_from_content_array(raw.pointer("/message/content"));
            let timeline_key = item_ref
                .clone()
                .map(|item| format!("claude:item:{item}"))
                .unwrap_or_else(|| format!("claude:item:{}", json_hash(raw)));
            event = event.with_timeline(
                session_ref,
                item_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "upsert",
            );
            event.kind = "adapter.item_completed".to_string();
            event.role = role;
            event.content = content;
            event.status = Some("completed".to_string());
            event.input_tokens = integer_at(raw, &["message", "usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["message", "usage", "output_tokens"]);
        }
        "tool_use" | "tool_result" => {
            let call_ref = string_at(raw, &["id"]).or_else(|| string_at(raw, &["tool_use_id"]));
            let timeline_key = call_ref
                .clone()
                .map(|call| format!("claude:tool:{call}"))
                .unwrap_or_else(|| format!("claude:tool:{}", json_hash(raw)));
            let operation = if provider_kind == "tool_use" {
                "started"
            } else {
                "completed"
            };
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                operation,
            );
            event.kind = if operation == "started" {
                "adapter.tool_call_started".to_string()
            } else {
                "adapter.tool_call_completed".to_string()
            };
            event.tool_name = string_at(raw, &["name"]);
            event.status = Some(operation.to_string());
            event.content = string_at(raw, &["content"]);
        }
        "result" => {
            event.kind = "adapter.turn_completed".to_string();
            event.external_session_ref = session_ref;
            event.status = string_at(raw, &["subtype"]).or_else(|| Some("completed".to_string()));
            event.input_tokens = integer_at(raw, &["usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["usage", "output_tokens"]);
        }
        _ => {}
    }

    vec![event]
}

fn parse_acp_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["method"])
        .or_else(|| string_at(raw, &["result", "kind"]))
        .unwrap_or_else(|| "unknown".to_string());
    let session_ref = string_at(raw, &["params", "sessionId"])
        .or_else(|| string_at(raw, &["params", "session_id"]))
        .or_else(|| string_at(raw, &["result", "sessionId"]));
    let update = raw.pointer("/params/update").unwrap_or(&Value::Null);
    let update_kind = string_at(update, &["sessionUpdate"])
        .or_else(|| string_at(update, &["kind"]))
        .unwrap_or_else(|| provider_kind.clone());
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::Acp,
        "adapter.raw_event",
        &update_kind,
        raw,
    );

    match update_kind.as_str() {
        "session_started" | "session_info" | "session_info_update" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("acp:{session}:session_info"));
        }
        "agent_message_chunk" | "user_message_chunk" | "agent_thought_chunk" => {
            let role = if update_kind == "user_message_chunk" {
                "user"
            } else {
                "assistant"
            };
            let content = string_at(update, &["content", "text"])
                .or_else(|| string_at(update, &["content"]))
                .unwrap_or_default();
            let synthetic = format!("{}:{}", role, stable_hash(content.as_bytes()));
            let timeline_key = match &session_ref {
                Some(session) => format!("acp:{session}:message:{synthetic}"),
                None => format!("acp:unknown:message:{synthetic}"),
            };
            event = event.with_timeline(
                session_ref,
                None,
                timeline_key,
                AdapterTimelineConfidence::Heuristic,
                "delta",
            );
            event.kind = "adapter.item_delta".to_string();
            event.role = Some(role.to_string());
            event.content = Some(content);
            event.status = Some("streaming".to_string());
        }
        "tool_call" | "tool_call_update" => {
            let call_ref =
                string_at(update, &["toolCallId"]).or_else(|| string_at(update, &["tool_call_id"]));
            let timeline_key = match (&session_ref, &call_ref) {
                (Some(session), Some(call)) => format!("acp:{session}:tool:{call}"),
                (_, Some(call)) => format!("acp:unknown:tool:{call}"),
                _ => format!("acp:unknown:tool:{}", json_hash(raw)),
            };
            let operation = string_at(update, &["status"]).unwrap_or_else(|| {
                if update_kind == "tool_call" {
                    "requested".to_string()
                } else {
                    "updated".to_string()
                }
            });
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                &operation,
            );
            event.kind = match operation.as_str() {
                "completed" => "adapter.tool_call_completed",
                "failed" => "adapter.tool_call_failed",
                "in_progress" => "adapter.tool_call_started",
                _ => "adapter.tool_call_requested",
            }
            .to_string();
            event.tool_name = string_at(update, &["title"])
                .or_else(|| string_at(update, &["name"]))
                .or_else(|| string_at(update, &["toolKind"]));
            event.status = Some(operation);
            event.content = string_at(update, &["content", "text"])
                .or_else(|| string_at(update, &["rawOutput"]));
        }
        "plan" | "plan_update" => {
            let timeline_key = match &session_ref {
                Some(session) => format!("acp:{session}:plan:current"),
                None => "acp:unknown:plan:current".to_string(),
            };
            event = event.with_timeline(
                session_ref,
                None,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "replace",
            );
            event.kind = "adapter.plan_replaced".to_string();
            event.content = raw.pointer("/params/update/entries").map(Value::to_string);
            event.status = Some("current".to_string());
        }
        _ => {}
    }

    vec![event]
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    match current {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_i64()
}

fn text_from_content_array(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    string_at(item, &["text"])
                        .or_else(|| string_at(item, &["content"]))
                        .or_else(|| string_at(item, &["input"]))
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() { None } else { Some(text) }
        }
        Value::Object(_) => {
            string_at(value?, &["text"]).or_else(|| string_at(value?, &["content"]))
        }
        _ => None,
    }
}

fn json_hash(value: &Value) -> String {
    stable_hash(value.to_string().as_bytes())
}

fn stable_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn planned_adapters_include_fake_and_first_real_targets() {
        assert!(PLANNED_ADAPTERS.contains(&"fake"));
        assert!(PLANNED_ADAPTERS.contains(&"codex-exec"));
        assert!(PLANNED_ADAPTERS.contains(&"claude-code"));
        assert!(PLANNED_ADAPTERS.contains(&"acp"));
    }

    #[test]
    fn fake_adapter_reports_adapter_boundary() {
        assert_eq!(
            AgentAdapter::fake().binding().kind,
            BoundaryKind::AgentAdapter
        );
    }

    #[test]
    fn fake_provider_reports_provider_boundary() {
        assert_eq!(
            ProviderConnector::fake().binding().kind,
            BoundaryKind::ProviderConnector
        );
    }

    #[test]
    fn codex_jsonl_fixture_maps_to_normalized_events() {
        let parsed =
            CodexExecAdapter::parse_jsonl(include_str!("../fixtures/codex-exec.jsonl")).unwrap();

        assert_eq!(parsed.raw_event_count, 5);
        assert!(parsed.events.iter().any(|event| {
            event.kind == "adapter.session_started"
                && event.external_session_ref.as_deref() == Some("codex-thread-1")
        }));
        let message = parsed
            .events
            .iter()
            .find(|event| event.kind == "adapter.item_completed")
            .expect("message event");
        assert_eq!(message.external_item_ref.as_deref(), Some("codex-item-1"));
        assert_eq!(message.role.as_deref(), Some("assistant"));
        assert_eq!(
            message.timeline_confidence,
            AdapterTimelineConfidence::Stable
        );
        assert!(parsed.events.iter().any(|event| {
            event.kind == "adapter.tool_call_completed"
                && event.tool_name.as_deref() == Some("exec_command")
        }));
        assert!(parsed.events.iter().any(|event| {
            event.kind == "adapter.turn_completed"
                && event.input_tokens == Some(11)
                && event.output_tokens == Some(7)
        }));
    }

    #[test]
    fn claude_stream_json_fixture_maps_to_normalized_events() {
        let parsed = ClaudeCodeAdapter::parse_stream_json(include_str!(
            "../fixtures/claude-code-stream.jsonl"
        ))
        .unwrap();

        assert_eq!(parsed.raw_event_count, 5);
        assert!(parsed.events.iter().any(|event| {
            event.kind == "adapter.session_started"
                && event.external_session_ref.as_deref() == Some("claude-session-1")
        }));
        let message = parsed
            .events
            .iter()
            .find(|event| event.external_item_ref.as_deref() == Some("msg_1"))
            .expect("claude message");
        assert_eq!(message.content.as_deref(), Some("Claude fixture response."));
        assert_eq!(message.input_tokens, Some(13));
        assert_eq!(message.output_tokens, Some(8));
        assert!(parsed.events.iter().any(|event| {
            event.kind == "adapter.tool_call_completed"
                && event.external_item_ref.as_deref() == Some("toolu_1")
        }));
    }

    #[test]
    fn acp_replay_fixture_maps_stable_and_heuristic_timeline_keys() {
        let parsed =
            AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();

        assert_eq!(parsed.raw_event_count, 7);
        let message = parsed
            .events
            .iter()
            .find(|event| event.kind == "adapter.item_delta")
            .expect("message delta");
        assert_eq!(
            message.timeline_confidence,
            AdapterTimelineConfidence::Heuristic
        );
        assert_eq!(message.role.as_deref(), Some("assistant"));
        let tool_events = parsed
            .events
            .iter()
            .filter(|event| event.timeline_key.as_deref() == Some("acp:acp-session-1:tool:tool-1"))
            .collect::<Vec<_>>();
        assert_eq!(tool_events.len(), 4);
        assert!(
            tool_events
                .iter()
                .all(|event| event.timeline_confidence == AdapterTimelineConfidence::Stable)
        );
    }

    #[test]
    fn acp_duplicate_tool_updates_dedupe_by_stable_idempotency_key() {
        let parsed =
            AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();

        let before = parsed
            .events
            .iter()
            .filter(|event| event.kind == "adapter.tool_call_completed")
            .count();
        let after = parsed
            .deduped_by_idempotency()
            .iter()
            .filter(|event| event.kind == "adapter.tool_call_completed")
            .count();

        assert_eq!(before, 2);
        assert_eq!(after, 1);
    }

    #[test]
    fn adapter_tool_observations_are_observed_only() {
        let acp =
            AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();
        let acp_observations = acp.tool_observations();

        assert_eq!(acp_observations.len(), 3);
        assert!(acp_observations.iter().all(|observation| {
            observation.source_adapter == "acp"
                && observation.instrumentation_level == "observed_only"
                && observation.confidence == "high"
                && observation.external_tool_ref.as_deref() == Some("tool-1")
        }));
        assert!(
            acp_observations
                .iter()
                .any(|observation| observation.observed_status == "completed")
        );

        let codex =
            CodexExecAdapter::parse_jsonl(include_str!("../fixtures/codex-exec.jsonl")).unwrap();
        let codex_observations = codex.tool_observations();
        assert!(codex_observations.iter().any(|observation| {
            observation.source_adapter == "codex_exec"
                && observation.instrumentation_level == "observed_only"
                && observation.tool_name == "exec_command"
        }));

        let claude = ClaudeCodeAdapter::parse_stream_json(include_str!(
            "../fixtures/claude-code-stream.jsonl"
        ))
        .unwrap();
        let claude_observations = claude.tool_observations();
        assert!(claude_observations.iter().any(|observation| {
            observation.source_adapter == "claude_code"
                && observation.instrumentation_level == "observed_only"
                && observation.external_tool_ref.as_deref() == Some("toolu_1")
        }));
    }

    #[test]
    fn acp_session_setup_uses_tool_capability_plan() {
        let wrappers =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                PathBuf::from("/tmp/capo-acp-workspace"),
                PathBuf::from("/tmp/capo-acp-artifacts"),
            ));

        let setup = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &capo_tools::PermissionPolicy::static_read_only_local(),
            SessionId::new("session-acp-setup"),
        );

        assert_eq!(setup.protocol_version, 1);
        assert_eq!(setup.client_kind, "capo");
        assert_eq!(
            setup.advertised_capabilities,
            vec!["filesystem.read_text_file"]
        );
        assert!(setup.filesystem_read.advertise);
        assert!(!setup.filesystem_write.advertise);
        assert!(!setup.terminal.advertise);
        assert_eq!(setup.credential_policy, "not_inspected");
        assert_eq!(setup.mcp_server_count, 0);
        assert!(!setup.runtime_started);
        assert!(!setup.provider_cli_executed);
    }

    #[test]
    fn acp_session_setup_fails_closed_when_backing_tool_missing() {
        let definitions =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                PathBuf::from("/tmp/capo-acp-workspace"),
                PathBuf::from("/tmp/capo-acp-artifacts"),
            ))
            .list_tools()
            .into_iter()
            .filter(|definition| definition.tool_id != "capo.file_read")
            .collect::<Vec<_>>();

        let setup = AcpAdapter::session_setup_plan(
            &definitions,
            &capo_tools::PermissionPolicy::allow_trusted_local(),
            SessionId::new("session-acp-missing-file-read"),
        );

        assert!(!setup.filesystem_read.advertise);
        assert_eq!(setup.filesystem_read.reason, "missing_backing_wrapper_tool");
        assert!(
            !setup
                .advertised_capabilities
                .contains(&"filesystem.read_text_file".to_string())
        );
    }

    #[test]
    fn acp_client_calls_route_only_when_capability_advertised() {
        let wrappers =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                PathBuf::from("/tmp/capo-acp-workspace"),
                PathBuf::from("/tmp/capo-acp-artifacts"),
            ));
        let read_only_setup = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &capo_tools::PermissionPolicy::static_read_only_local(),
            SessionId::new("session-acp-client-read-only"),
        );

        let read = read_only_setup
            .wrapper_request_for_client_call(acp_client_call(
                "fs/read_text_file",
                serde_json::json!({"path":"README.md"}),
            ))
            .expect("read advertised");
        assert_eq!(read.tool_id, "capo.file_read");
        assert_eq!(read.input["path"].as_str(), Some("README.md"));
        assert_eq!(read.capability_profile_id, "read-only-local");

        let write = read_only_setup.wrapper_request_for_client_call(acp_client_call(
            "fs/write_text_file",
            serde_json::json!({"path":"README.md","content":"changed"}),
        ));
        assert!(write.unwrap_err().contains("filesystem.write_text_file"));

        let terminal = read_only_setup.wrapper_request_for_client_call(acp_client_call(
            "terminal/run",
            serde_json::json!({"program":"cargo","argv":["test"],"cwd":"."}),
        ));
        assert!(terminal.unwrap_err().contains("terminal"));
    }

    #[test]
    fn acp_terminal_call_routes_to_shell_wrapper_for_trusted_profile() {
        let wrappers =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                PathBuf::from("/tmp/capo-acp-workspace"),
                PathBuf::from("/tmp/capo-acp-artifacts"),
            ));
        let setup = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &capo_tools::PermissionPolicy::allow_trusted_local(),
            SessionId::new("session-acp-client-trusted"),
        );

        let request = setup
            .wrapper_request_for_client_call(acp_client_call_with_profile(
                "terminal/run",
                serde_json::json!({"program":"cargo","argv":["test","-p","capo-adapters"],"cwd":"."}),
                "trusted-local-dev",
            ))
            .expect("terminal advertised");

        assert_eq!(request.tool_id, "capo.shell_run");
        assert_eq!(request.input["program"].as_str(), Some("cargo"));
        assert_eq!(request.input["argv"].as_array().expect("argv").len(), 3);
    }

    #[test]
    fn codex_launch_plan_builds_subscription_safe_runtime_request() {
        let workspace = temp_root("codex-launch-workspace");
        let artifacts = temp_root("codex-launch-artifacts");
        let plan = CodexExecAdapter::local_launch_plan(
            workspace.clone(),
            artifacts.clone(),
            "Summarize this project state.",
        );

        plan.assert_subscription_safe().unwrap();
        assert_eq!(plan.provider_kind, "codex_subscription");
        assert_eq!(plan.credential_scope, "user_local_subscription");
        assert_eq!(plan.stdout_format, "jsonl");
        assert_eq!(plan.stderr_policy, "logs_redacted");
        assert_eq!(
            plan.runtime_config().workspace_roots,
            vec![workspace.clone()]
        );
        let request = plan.runtime_request(RunId::new("run-codex-launch"));
        assert_eq!(request.program, "codex");
        assert_eq!(request.cwd, workspace);
        assert!(request.env.is_empty());
        assert!(
            request
                .argv
                .windows(2)
                .any(|args| args == ["--sandbox", "read-only"])
        );
        assert!(request.argv.iter().any(|arg| arg == "--ephemeral"));
        assert!(request.argv.iter().any(|arg| arg == "--ignore-user-config"));
        assert!(request.argv.iter().any(|arg| arg == "--ignore-rules"));
        assert!(
            request
                .argv
                .windows(2)
                .any(|args| args == ["--cd", workspace.to_string_lossy().as_ref()])
        );
        assert_eq!(
            request.argv.last().map(String::as_str),
            Some("Summarize this project state.")
        );
        assert_eq!(plan.artifact_root, artifacts);
    }

    #[test]
    fn claude_launch_plan_builds_subscription_safe_runtime_request() {
        let workspace = temp_root("claude-launch-workspace");
        let artifacts = temp_root("claude-launch-artifacts");
        let plan = ClaudeCodeAdapter::local_launch_plan(
            workspace.clone(),
            artifacts,
            "Summarize this project state.",
        );

        plan.assert_subscription_safe().unwrap();
        assert_eq!(plan.provider_kind, "claude_subscription");
        assert_eq!(plan.credential_scope, "user_local_subscription");
        assert_eq!(plan.stdout_format, "stream-json");
        let request = plan.runtime_request(RunId::new("run-claude-launch"));
        assert_eq!(request.program, "claude");
        assert_eq!(request.cwd, workspace);
        assert!(request.env.is_empty());
        assert!(
            request
                .argv
                .windows(2)
                .any(|args| args == ["--output-format", "stream-json"])
        );
        assert!(
            request
                .argv
                .windows(2)
                .any(|args| args == ["--permission-mode", "plan"])
        );
        assert!(
            request
                .argv
                .iter()
                .any(|arg| arg == "--no-session-persistence")
        );
        assert!(
            request
                .argv
                .iter()
                .any(|arg| arg == "--disable-slash-commands")
        );
        assert!(request.argv.windows(2).any(|args| args == ["--tools", ""]));
        assert!(
            request
                .argv
                .windows(2)
                .any(|args| args == ["--disallowedTools", "*"])
        );
        assert!(request.argv.iter().any(|arg| arg == "--strict-mcp-config"));
        assert_eq!(
            request.argv.last().map(String::as_str),
            Some("Summarize this project state.")
        );
    }

    #[test]
    fn launch_plan_rejects_secret_like_env_or_argv_markers() {
        let workspace = temp_root("unsafe-launch-workspace");
        let artifacts = temp_root("unsafe-launch-artifacts");
        let mut plan = CodexExecAdapter::local_launch_plan(workspace, artifacts, "hello");
        plan.env_allowlist.push("OPENAI_API_KEY".to_string());
        assert!(
            plan.assert_subscription_safe()
                .unwrap_err()
                .contains("env allowlist")
        );

        plan.env_allowlist = local_subscription_cli_env_allowlist();
        plan.argv.push("Authorization: bearer secret".to_string());
        assert!(
            plan.assert_subscription_safe()
                .unwrap_err()
                .contains("argv")
        );
    }

    #[test]
    fn codex_local_smoke_plan_uses_restrictive_defaults() {
        let workspace = temp_root("codex-workspace");
        let artifacts = temp_root("codex-artifacts");
        let plan = CodexExecAdapter::local_smoke_plan(workspace.clone(), artifacts.clone());

        assert_eq!(plan.opt_in_env, "CAPO_RUN_CODEX_LOCAL_SMOKE");
        assert_eq!(plan.program, "codex");
        assert!(
            plan.argv
                .windows(2)
                .any(|args| args == ["--sandbox", "read-only"])
        );
        assert!(plan.argv.iter().any(|arg| arg == "--ephemeral"));
        assert!(plan.argv.iter().any(|arg| arg == "--ignore-user-config"));
        assert!(plan.argv.iter().any(|arg| arg == "--ignore-rules"));
        assert!(
            plan.argv
                .windows(2)
                .any(|args| args == ["--cd", workspace.to_string_lossy().as_ref()])
        );
        assert_eq!(plan.workspace_root, workspace);
        assert_eq!(plan.artifact_root, artifacts);
        assert!(!plan.env_allowlist.iter().any(|name| name.contains("TOKEN")));
    }

    #[test]
    fn claude_local_smoke_plan_disables_tools_and_mcp_by_default() {
        let workspace = temp_root("claude-workspace");
        let artifacts = temp_root("claude-artifacts");
        let plan = ClaudeCodeAdapter::local_smoke_plan(workspace, artifacts);

        assert_eq!(plan.opt_in_env, "CAPO_RUN_CLAUDE_LOCAL_SMOKE");
        assert_eq!(plan.program, "claude");
        assert!(
            plan.argv
                .windows(2)
                .any(|args| args == ["--output-format", "stream-json"])
        );
        assert!(
            plan.argv
                .windows(2)
                .any(|args| args == ["--permission-mode", "plan"])
        );
        assert!(
            plan.argv
                .iter()
                .any(|arg| arg == "--no-session-persistence")
        );
        assert!(
            plan.argv
                .iter()
                .any(|arg| arg == "--disable-slash-commands")
        );
        assert!(plan.argv.windows(2).any(|args| args == ["--tools", ""]));
        assert!(
            plan.argv
                .windows(2)
                .any(|args| args == ["--disallowedTools", "*"])
        );
        assert!(plan.argv.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(!plan.env_allowlist.iter().any(|name| name.contains("TOKEN")));
    }

    #[test]
    fn local_adapter_smoke_runner_skips_without_explicit_opt_in() {
        let plan = LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            opt_in_env: "CAPO_TEST_UNSET_LOCAL_SMOKE",
            program: "/bin/echo".to_string(),
            argv: vec!["CAPO_CODEX_SMOKE_OK".to_string()],
            workspace_root: temp_root("skip-workspace"),
            artifact_root: temp_root("skip-artifacts"),
            env_allowlist: Vec::new(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 1024,
            expected_output_marker: "CAPO_CODEX_SMOKE_OK",
        };

        let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan).unwrap();

        assert!(outcome.is_none());
    }

    #[test]
    fn local_adapter_smoke_runner_executes_through_runtime_boundary() {
        let workspace = temp_root("echo-workspace");
        let artifact_root = temp_root("echo-artifacts");
        let plan = LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            opt_in_env: "CAPO_TEST_UNSET_LOCAL_SMOKE",
            program: "/bin/echo".to_string(),
            argv: vec!["CAPO_CODEX_SMOKE_OK".to_string()],
            workspace_root: workspace,
            artifact_root,
            env_allowlist: Vec::new(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 1024,
            expected_output_marker: "CAPO_CODEX_SMOKE_OK",
        };

        let outcome = LocalAdapterSmokeRunner::run(&plan).unwrap();

        assert_eq!(outcome.process.status, "exited");
        assert!(
            fs::read_to_string(&outcome.stdout.path)
                .unwrap()
                .contains("CAPO_CODEX_SMOKE_OK")
        );
        assert!(outcome.events.iter().any(|event| {
            event.kind == "runtime.output_artifact_recorded"
                && event.status == outcome.stdout.redaction_state
        }));
    }

    #[test]
    #[ignore = "requires CAPO_RUN_CODEX_LOCAL_SMOKE=1 and local Codex login"]
    fn local_codex_adapter_smoke() {
        let plan = CodexExecAdapter::local_smoke_plan(
            temp_root("real-codex-workspace"),
            temp_root("real-codex-artifacts"),
        );
        let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan)
            .expect("codex local smoke should either skip or pass");

        assert!(
            outcome.is_some() || !plan.is_opted_in(),
            "set CAPO_RUN_CODEX_LOCAL_SMOKE=1 to execute the Codex local smoke"
        );
    }

    #[test]
    #[ignore = "requires CAPO_RUN_CLAUDE_LOCAL_SMOKE=1 and verified restricted Claude Code args"]
    fn local_claude_adapter_smoke() {
        let plan = ClaudeCodeAdapter::local_smoke_plan(
            temp_root("real-claude-workspace"),
            temp_root("real-claude-artifacts"),
        );
        let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan)
            .expect("claude local smoke should either skip or pass");

        assert!(
            outcome.is_some() || !plan.is_opted_in(),
            "set CAPO_RUN_CLAUDE_LOCAL_SMOKE=1 after verifying restricted Claude Code args"
        );
    }

    #[test]
    fn artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets() {
        let root = temp_root("scan");
        fs::create_dir_all(&root).unwrap();
        let redacted = root.join("redacted.txt");
        let raw = root.join("raw.txt");
        fs::write(&redacted, "Authorization: [REDACTED]\n").unwrap();
        fs::write(&raw, "Authorization: bearer secret\n").unwrap();

        scan_artifacts_for_sensitive_markers([&redacted]).unwrap();
        let error = scan_artifacts_for_sensitive_markers([&raw]).unwrap_err();

        assert!(matches!(
            error,
            LocalAdapterSmokeError::SensitiveArtifact { marker, .. } if marker == "authorization:"
        ));
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-adapter-{name}-{nanos}"))
    }

    fn acp_client_call(method: &str, params: Value) -> AcpClientCall {
        acp_client_call_with_profile(method, params, "read-only-local")
    }

    fn acp_client_call_with_profile(
        method: &str,
        params: Value,
        capability_profile_id: &str,
    ) -> AcpClientCall {
        AcpClientCall {
            method: method.to_string(),
            params,
            tool_call_id: ToolCallId::new(format!("tool-call-{}", method.replace(['/', '_'], "-"))),
            session_id: SessionId::new("session-acp-client-call"),
            run_id: RunId::new("run-acp-client-call"),
            capability_profile_id: capability_profile_id.to_string(),
        }
    }
}
