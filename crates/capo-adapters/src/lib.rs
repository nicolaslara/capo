//! Agent adapter and provider connector scaffolding.
//!
//! P6 adds fixture parsers for Codex, Claude Code, and ACP streams. The
//! parsers preserve provider-specific records as adapter facts and emit
//! normalized adapter events for the controller pipeline.

mod local_subscription;
mod scripted_mock_agent;

#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, ToolCallId, TurnId};
use capo_tools::{
    AcpClientCapabilityDecision, AcpClientCapabilityPlan, PermissionPolicy, ToolDefinition,
    WrapperToolRequest,
};
use serde_json::Value;

pub use local_subscription::{
    LocalAdapterLaunchPlan, LocalAdapterSmokeError, LocalAdapterSmokePlan, LocalAdapterSmokeResult,
    LocalAdapterSmokeRunner, scan_artifacts_for_sensitive_markers,
};
pub use scripted_mock_agent::{ScriptedMockAgent, ScriptedMockEvent, ScriptedMockTurn};

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] =
    &["fake", "scripted-mock", "codex-exec", "claude-code", "acp"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentAdapter {
    Fake(FakeAdapter),
    ScriptedMock(ScriptedMockAgent),
}

impl AgentAdapter {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn scripted_mock(script: ScriptedMockAgent) -> Self {
        Self::ScriptedMock(script)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(adapter) => adapter.binding(),
            Self::ScriptedMock(agent) => agent.binding(),
        }
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.open_session(request),
            Self::ScriptedMock(agent) => agent.open_session(request),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.send_turn(session, request),
            Self::ScriptedMock(agent) => agent.send_turn(session, request),
        }
    }

    pub fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.attach_session(session_id, external_session_ref),
            Self::ScriptedMock(agent) => agent.attach_session(session_id, external_session_ref),
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.interrupt(session, reason),
            Self::ScriptedMock(agent) => agent.interrupt(session, reason),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.stop(session, reason),
            Self::ScriptedMock(agent) => agent.stop(session, reason),
        }
    }

    pub fn scripted_turn_events(&self, turn_ref: &str) -> Option<Vec<NormalizedAdapterEvent>> {
        match self {
            Self::Fake(_) => None,
            Self::ScriptedMock(agent) => agent.turn_events(turn_ref),
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
    Mock,
    CodexExec,
    ClaudeCode,
    Acp,
}

impl NormalizedAdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
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
    pub(crate) fn new(
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

    pub(crate) fn with_timeline(
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
mod tests;
