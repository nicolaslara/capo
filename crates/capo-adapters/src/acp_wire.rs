//! DP1: the LIVE ACP JSON-RPC 2.0 stdio client below the `AgentAdapter` trait.
//!
//! This promotes the fixture-only [`crate::AcpAdapter::session_setup_plan`]
//! (capability planning over `ToolDefinition`s) into a real wire client the loop
//! can drive: it speaks JSON-RPC 2.0 over a line-delimited stdio transport,
//! implementing the agent-call surface from `protocol-provider.md` --
//! `initialize` (recording the negotiated integer `protocolVersion`, stable `1`
//! today), `session/new`, `session/prompt`, `session/cancel` -- and ingests
//! `session/update` NOTIFICATIONS through the SAME `parse_acp_record` normalizer
//! the deterministic replay fixtures use (never a parallel ingestion route).
//!
//! It also implements the LIVE `session/request_permission` CLIENT round-trip on
//! the wire: when the agent calls `session/request_permission`, the client maps
//! the offered ACP `PermissionOption[]` through the
//! [`crate::map_acp_options_trusted_local`] table into a Capo decision and
//! answers the agent with the chosen option (or `cancelled`). The live wire
//! round-trip lands HERE, not in `safety-gates` (which scoped it to fakes +
//! option mapping only).
//!
//! The transport is abstracted ([`AcpTransport`]) so the whole protocol is
//! DETERMINISTICALLY testable against a scripted in-memory server transcript
//! with NO live process ([`ScriptedAcpTransport`]); the real process transport
//! ([`PipedProcessTransport`]) is launched through `RuntimeRunner`
//! (`LocalProcessRunner::spawn_piped_process`), so adapters never own the process
//! group -- the runtime does. ACP stays strictly an adapter: no `session/update`
//! is directly authoritative for read models, and Capo never exposes itself as
//! an ACP agent backend.

use std::io::{BufRead, BufReader, Read, Write};

use serde_json::{Value, json};

use crate::{
    AcpAdapter, AcpPermissionOption, AcpPermissionOptionKind, AcpPermissionOutcome,
    AcpSessionSetupPlan, NormalizedAdapterEvent, map_acp_options_trusted_local,
};

/// The negotiated ACP protocol version Capo proposes/accepts. Integer, stable
/// `1` today per `protocol-provider.md` / `acp-replay-dedupe.md`.
pub const ACP_PROTOCOL_VERSION: i64 = 1;

/// A line-delimited JSON-RPC 2.0 transport to an ACP agent.
///
/// Each frame is one JSON object on its own line. The wire client writes its
/// outbound requests/responses through [`AcpTransport::send_line`] and reads the
/// agent's responses, notifications, and inbound requests through
/// [`AcpTransport::recv_line`]. `recv_line` returns `Ok(None)` at end-of-stream.
pub trait AcpTransport {
    fn send_line(&mut self, line: &str) -> Result<(), AcpWireError>;
    fn recv_line(&mut self) -> Result<Option<String>, AcpWireError>;
}

/// A typed error from the live ACP wire client.
#[derive(Debug)]
pub enum AcpWireError {
    /// An I/O failure writing to or reading from the transport.
    Transport(String),
    /// A frame off the wire was not valid JSON.
    Decode { line: usize, message: String },
    /// The agent returned a JSON-RPC error response to one of our requests.
    AgentError { method: String, message: String },
    /// The stream ended before the awaited response arrived.
    UnexpectedEof { awaiting: String },
    /// A protocol invariant was violated (e.g. a response id we never sent).
    Protocol(String),
}

impl std::fmt::Display for AcpWireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(detail) => write!(f, "acp transport error: {detail}"),
            Self::Decode { line, message } => {
                write!(f, "acp frame decode error at line {line}: {message}")
            }
            Self::AgentError { method, message } => {
                write!(f, "acp agent error for `{method}`: {message}")
            }
            Self::UnexpectedEof { awaiting } => {
                write!(f, "acp stream ended while awaiting `{awaiting}`")
            }
            Self::Protocol(detail) => write!(f, "acp protocol violation: {detail}"),
        }
    }
}

impl std::error::Error for AcpWireError {}

/// The result of one driven ACP turn over the wire.
///
/// Carries the normalized events ingested from `session/update` notifications
/// (through the shared `parse_acp_record` path) plus an audit trail of the
/// permission round-trips the client answered, so a deterministic test can
/// assert both the normalized-event shape AND the live permission outcomes
/// without a parallel ingestion route.
#[derive(Clone, Debug, Default)]
pub struct AcpTurnTranscript {
    /// Normalized events from every ingested `session/update` notification, in
    /// wire order.
    pub events: Vec<NormalizedAdapterEvent>,
    /// Every `session/request_permission` round-trip the client answered.
    pub permission_round_trips: Vec<AcpPermissionRoundTrip>,
    /// The `stopReason` the agent reported on the `session/prompt` response, if
    /// the prompt completed (e.g. `end_turn`, `cancelled`).
    pub stop_reason: Option<String>,
    /// Whether a `session/cancel` was issued during this turn.
    pub cancelled: bool,
}

/// The audited outcome of one live `session/request_permission` round-trip: what
/// the agent offered, what Capo chose, and the option id returned on the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpPermissionRoundTrip {
    pub tool_call_id: Option<String>,
    pub offered_option_ids: Vec<String>,
    pub capo_decision: String,
    pub outcome: AcpPermissionOutcome,
}

/// The live ACP JSON-RPC 2.0 client. Generic over the [`AcpTransport`] so the
/// identical protocol logic runs against a scripted in-memory server (tests) and
/// a runtime-spawned process pipe (live).
pub struct AcpWireClient<T: AcpTransport> {
    transport: T,
    next_id: i64,
    /// The capability setup plan: which `fs/*` / `terminal/*` client calls Capo
    /// advertises, and the option-mapping policy seam for `request_permission`.
    setup_plan: AcpSessionSetupPlan,
    negotiated_protocol_version: Option<i64>,
    external_session_id: Option<String>,
}

impl<T: AcpTransport> AcpWireClient<T> {
    /// Attach the client to a started transport with the given capability setup
    /// plan. The transport is created by launching the ACP process through
    /// `RuntimeRunner` (live) or by a scripted server (tests); the client is
    /// "attached after start".
    pub fn attach(transport: T, setup_plan: AcpSessionSetupPlan) -> Self {
        Self {
            transport,
            next_id: 1,
            setup_plan,
            negotiated_protocol_version: None,
            external_session_id: None,
        }
    }

    pub fn negotiated_protocol_version(&self) -> Option<i64> {
        self.negotiated_protocol_version
    }

    pub fn external_session_id(&self) -> Option<&str> {
        self.external_session_id.as_deref()
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// `initialize`: negotiate the integer `protocolVersion` and advertise the
    /// client capabilities the setup plan permits. Records the negotiated
    /// version (stable `1` today).
    pub fn initialize(&mut self) -> Result<i64, AcpWireError> {
        let id = self.alloc_id();
        let params = json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "clientCapabilities": {
                "fs": {
                    "readTextFile": self.setup_plan.filesystem_read.advertise,
                    "writeTextFile": self.setup_plan.filesystem_write.advertise,
                },
                "terminal": self.setup_plan.terminal.advertise,
            },
        });
        let result = self.request("initialize", params, id)?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_i64)
            .unwrap_or(ACP_PROTOCOL_VERSION);
        self.negotiated_protocol_version = Some(version);
        Ok(version)
    }

    /// `session/new`: create a new session, recording the external session id.
    pub fn session_new(&mut self, cwd: &str) -> Result<String, AcpWireError> {
        let id = self.alloc_id();
        let params = json!({ "cwd": cwd, "mcpServers": [] });
        let result = self.request("session/new", params, id)?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                AcpWireError::Protocol("session/new response missing sessionId".to_string())
            })?;
        self.external_session_id = Some(session_id.clone());
        Ok(session_id)
    }

    /// `session/prompt`: send the prompt and pump the wire until the agent
    /// returns the prompt response, ingesting every interleaved `session/update`
    /// notification AND answering every `session/request_permission` request on
    /// the wire. Returns the full turn transcript.
    pub fn prompt(
        &mut self,
        session_id: &str,
        prompt: &str,
    ) -> Result<AcpTurnTranscript, AcpWireError> {
        let id = self.alloc_id();
        let params = json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": prompt }],
        });
        self.send_request("session/prompt", &params, id)?;
        let mut transcript = AcpTurnTranscript::default();
        let response = self.pump_until_response(id, "session/prompt", &mut transcript)?;
        transcript.stop_reason = response
            .get("stopReason")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(transcript)
    }

    /// `session/cancel`: notify the agent to cancel the in-flight prompt. ACP
    /// cancel is a NOTIFICATION (no response); late `session/update`s and the
    /// terminal prompt response with `stopReason: cancelled` are still accepted
    /// by the caller's pump.
    pub fn cancel(&mut self, session_id: &str) -> Result<(), AcpWireError> {
        let params = json!({ "sessionId": session_id });
        self.send_notification("session/cancel", &params)
    }

    /// Send a request and pump until its matching response, returning the
    /// `result` object. Used for the request/response calls (`initialize`,
    /// `session/new`).
    fn request(&mut self, method: &str, params: Value, id: i64) -> Result<Value, AcpWireError> {
        self.send_request(method, &params, id)?;
        let mut transcript = AcpTurnTranscript::default();
        self.pump_until_response(id, method, &mut transcript)
    }

    /// Pump inbound frames until the response to `awaited_id` arrives:
    /// - `session/update` notifications are normalized and appended to the
    ///   transcript through the shared `parse_acp_record` path;
    /// - inbound `session/request_permission` requests are answered on the wire;
    /// - the matching response is returned.
    fn pump_until_response(
        &mut self,
        awaited_id: i64,
        method: &str,
        transcript: &mut AcpTurnTranscript,
    ) -> Result<Value, AcpWireError> {
        let mut line_number = 0usize;
        loop {
            let Some(line) = self.transport.recv_line()? else {
                return Err(AcpWireError::UnexpectedEof {
                    awaiting: method.to_string(),
                });
            };
            line_number += 1;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(line).map_err(|error| AcpWireError::Decode {
                    line: line_number,
                    message: error.to_string(),
                })?;

            // A response to one of our requests (carries `id`, and `result` or
            // `error`, but no `method`).
            if value.get("method").is_none() && value.get("id").is_some() {
                let response_id = value.get("id").and_then(Value::as_i64);
                if response_id == Some(awaited_id) {
                    if let Some(error) = value.get("error") {
                        return Err(AcpWireError::AgentError {
                            method: method.to_string(),
                            message: error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                        });
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
                // A stray response to an id we are not awaiting: ignore (the
                // agent should not send these, but we never crash on noise).
                continue;
            }

            // An inbound request or notification from the agent.
            let frame_method = value.get("method").and_then(Value::as_str).unwrap_or("");
            match frame_method {
                "session/update" => {
                    let events = AcpAdapter::normalize_update(&value);
                    transcript.events.extend(events);
                }
                "session/request_permission" => {
                    self.answer_permission(&value, transcript)?;
                }
                "session/cancel" => {
                    // Agents do not normally send cancel TO the client; record it
                    // as observed if they do, but take no action.
                    transcript.cancelled = true;
                }
                _ => {
                    // Unknown notification/request: ingest as a raw normalized
                    // event so nothing is silently dropped, but never project it
                    // authoritatively.
                    let events = AcpAdapter::normalize_update(&value);
                    transcript.events.extend(events);
                }
            }
        }
    }

    /// Answer an inbound `session/request_permission` request on the wire: map
    /// the offered options through the TrustedLocal table and reply with the
    /// chosen `optionId` (or `cancelled`).
    fn answer_permission(
        &mut self,
        request: &Value,
        transcript: &mut AcpTurnTranscript,
    ) -> Result<(), AcpWireError> {
        let request_id = request.get("id").cloned().ok_or_else(|| {
            AcpWireError::Protocol("session/request_permission missing id".to_string())
        })?;
        let params = request.get("params").unwrap_or(&Value::Null);
        let tool_call_id = params
            .get("toolCall")
            .and_then(|tc| tc.get("toolCallId"))
            .or_else(|| params.get("toolCallId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let options = parse_permission_options(params);
        let offered_option_ids = options.iter().map(|o| o.option_id.clone()).collect();

        let mapping = map_acp_options_trusted_local(&options);
        let outcome_value = match &mapping.outcome {
            AcpPermissionOutcome::Selected { option_id } => {
                json!({ "outcome": "selected", "optionId": option_id })
            }
            AcpPermissionOutcome::Cancelled => json!({ "outcome": "cancelled" }),
        };
        let response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": { "outcome": outcome_value },
        });
        self.transport
            .send_line(&serde_json::to_string(&response).unwrap())?;

        transcript
            .permission_round_trips
            .push(AcpPermissionRoundTrip {
                tool_call_id,
                offered_option_ids,
                capo_decision: mapping.capo_decision.to_string(),
                outcome: mapping.outcome.clone(),
            });
        Ok(())
    }

    fn send_request(&mut self, method: &str, params: &Value, id: i64) -> Result<(), AcpWireError> {
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.transport
            .send_line(&serde_json::to_string(&frame).unwrap())
    }

    fn send_notification(&mut self, method: &str, params: &Value) -> Result<(), AcpWireError> {
        let frame = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.transport
            .send_line(&serde_json::to_string(&frame).unwrap())
    }
}

/// Parse the `options` array of a `session/request_permission` request into the
/// adapter-native [`AcpPermissionOption`] taxonomy, dropping any option whose
/// `kind` is not a known ACP option kind (so an agent cannot smuggle an
/// un-mapped option past the policy).
fn parse_permission_options(params: &Value) -> Vec<AcpPermissionOption> {
    let Some(options) = params.get("options").and_then(Value::as_array) else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|option| {
            let option_id = option.get("optionId").and_then(Value::as_str)?;
            let name = option
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(option_id);
            let kind = option
                .get("kind")
                .and_then(Value::as_str)
                .and_then(AcpPermissionOptionKind::parse)?;
            Some(AcpPermissionOption::new(option_id, name, kind))
        })
        .collect()
}

/// A DETERMINISTIC scripted ACP server transport: it replays a fixed sequence of
/// server-originated frames (responses, `session/update` notifications, inbound
/// `session/request_permission` requests) and records every client-originated
/// frame, with NO live process.
///
/// The scripting model is a queue of [`ScriptedServerFrame`]s: a `Response` is
/// emitted when the client sends a request whose method matches; a `Notify` /
/// `Request` is emitted immediately after, in order, on the next `recv_line`.
/// This lets a test assert the full
/// `initialize -> session/new -> prompt -> updates -> request_permission ->
/// cancel` transcript over the real wire client.
pub struct ScriptedAcpTransport {
    /// Outbound (server -> client) frames queued for delivery.
    outbound: std::collections::VecDeque<String>,
    /// Recorded inbound (client -> server) frames, in order.
    pub recorded: Vec<Value>,
    /// Pending scripted reactions keyed by request method: when the client sends
    /// that method, the listed frames are enqueued for delivery.
    reactions: Vec<(String, Vec<String>)>,
}

/// One frame the scripted server emits.
pub enum ScriptedServerFrame {
    /// A JSON-RPC response with the given `result`, addressed to the matching
    /// request's id (the transport fills in the id from the recorded request).
    Response(Value),
    /// A `session/update` notification with the given params.
    Update(Value),
    /// An inbound `session/request_permission` request with the given params
    /// (the transport assigns a server-side id).
    RequestPermission(Value),
    /// A raw frame emitted verbatim.
    Raw(Value),
}

impl ScriptedAcpTransport {
    pub fn new() -> Self {
        Self {
            outbound: std::collections::VecDeque::new(),
            recorded: Vec::new(),
            reactions: Vec::new(),
        }
    }

    /// Script that when the client sends `method`, the server emits `frames` (in
    /// order) before the client's next read resolves.
    #[must_use]
    pub fn on_request(mut self, method: &str, frames: Vec<ScriptedServerFrame>) -> Self {
        let encoded = frames
            .into_iter()
            .map(|frame| match frame {
                ScriptedServerFrame::Response(result) => {
                    // Id is filled at send-time from the matching request; mark
                    // it with a sentinel the matcher rewrites.
                    json!({ "jsonrpc": "2.0", "id": null, "result": result }).to_string()
                }
                ScriptedServerFrame::Update(params) => {
                    json!({ "jsonrpc": "2.0", "method": "session/update", "params": params })
                        .to_string()
                }
                ScriptedServerFrame::RequestPermission(params) => json!({
                    "jsonrpc": "2.0",
                    "id": format!("perm-{}", params.get("toolCall").and_then(|tc| tc.get("toolCallId")).and_then(Value::as_str).unwrap_or("0")),
                    "method": "session/request_permission",
                    "params": params,
                })
                .to_string(),
                ScriptedServerFrame::Raw(frame) => frame.to_string(),
            })
            .collect();
        self.reactions.push((method.to_string(), encoded));
        self
    }
}

impl Default for ScriptedAcpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl ScriptedAcpTransport {
    /// Replace the scripted reaction for `method` with a single JSON-RPC error
    /// response (the id is rewritten to the matching request id at send time).
    fn with_error_reaction(mut self, method: &str, error: Value) -> Self {
        self.reactions.retain(|(m, _)| m != method);
        self.reactions.push((
            method.to_string(),
            vec![json!({ "jsonrpc": "2.0", "id": null, "error": error }).to_string()],
        ));
        self
    }
}

impl AcpTransport for ScriptedAcpTransport {
    fn send_line(&mut self, line: &str) -> Result<(), AcpWireError> {
        let value: Value = serde_json::from_str(line).map_err(|error| AcpWireError::Decode {
            line: self.recorded.len() + 1,
            message: error.to_string(),
        })?;
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let request_id = value.get("id").cloned();
        self.recorded.push(value);

        // If this client frame is a request that matches a scripted reaction,
        // enqueue that reaction's frames, rewriting any `Response` sentinel id to
        // the request's id.
        if let Some(method) = method
            && let Some(index) = self.reactions.iter().position(|(m, _)| m == &method)
        {
            let (_, frames) = self.reactions.remove(index);
            for frame in frames {
                let rewritten = rewrite_response_id(&frame, request_id.as_ref());
                self.outbound.push_back(rewritten);
            }
        }
        Ok(())
    }

    fn recv_line(&mut self) -> Result<Option<String>, AcpWireError> {
        Ok(self.outbound.pop_front())
    }
}

/// Drive the wire client through a borrowed transport so a test can inspect the
/// transport's recording after the client drops.
impl<T: AcpTransport + ?Sized> AcpTransport for &mut T {
    fn send_line(&mut self, line: &str) -> Result<(), AcpWireError> {
        (**self).send_line(line)
    }

    fn recv_line(&mut self) -> Result<Option<String>, AcpWireError> {
        (**self).recv_line()
    }
}

/// Rewrite a scripted `Response` frame's sentinel `"id": null` to the matching
/// request id, leaving notifications and inbound requests untouched.
fn rewrite_response_id(frame: &str, request_id: Option<&Value>) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(frame) else {
        return frame.to_string();
    };
    let is_response = value.get("method").is_none()
        && (value.get("result").is_some() || value.get("error").is_some());
    if is_response && let (Some(object), Some(request_id)) = (value.as_object_mut(), request_id) {
        object.insert("id".to_string(), request_id.clone());
    }
    value.to_string()
}

/// The LIVE process transport: line-delimited JSON-RPC over the runtime-spawned
/// child's stdin (write) and stdout (read) pipes.
///
/// The process is launched through `RuntimeRunner`
/// (`LocalProcessRunner::spawn_piped_process`), which owns the process group; the
/// adapter only borrows the taken pipe handles, so it never owns the process
/// group itself.
pub struct PipedProcessTransport<W: Write, R: Read> {
    writer: W,
    reader: BufReader<R>,
}

impl<W: Write, R: Read> PipedProcessTransport<W, R> {
    pub fn new(writer: W, reader: R) -> Self {
        Self {
            writer,
            reader: BufReader::new(reader),
        }
    }
}

impl<W: Write, R: Read> AcpTransport for PipedProcessTransport<W, R> {
    fn send_line(&mut self, line: &str) -> Result<(), AcpWireError> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|error| AcpWireError::Transport(error.to_string()))
    }

    fn recv_line(&mut self) -> Result<Option<String>, AcpWireError> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(line)),
            Err(error) => Err(AcpWireError::Transport(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AcpAdapter, AdapterTimelineConfidence};
    use capo_core::SessionId;

    fn setup_plan() -> AcpSessionSetupPlan {
        // A trusted-local capability plan advertises fs read + write + terminal,
        // so the wire client's `initialize` capabilities reflect the plan.
        let wrappers =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                std::path::PathBuf::from("/tmp/capo-acp-wire-ws"),
                std::path::PathBuf::from("/tmp/capo-acp-wire-art"),
            ));
        AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &capo_tools::PermissionPolicy::allow_trusted_local(),
            SessionId::new("session-acp-wire"),
        )
    }

    /// The full scripted transcript with NO live process:
    /// `initialize -> session/new -> prompt -> updates -> request_permission ->
    /// (prompt response)`. Asserts the negotiated protocol version, the recorded
    /// outbound client frames, the normalized events ingested through the shared
    /// `parse_acp_record` path, and the live permission round-trip outcome.
    #[test]
    fn scripted_transcript_drives_full_acp_flow() {
        let transport = ScriptedAcpTransport::new()
            .on_request(
                "initialize",
                vec![ScriptedServerFrame::Response(
                    json!({ "protocolVersion": 1, "agentCapabilities": {} }),
                )],
            )
            .on_request(
                "session/new",
                vec![ScriptedServerFrame::Response(
                    json!({ "sessionId": "acp-session-wire-1" }),
                )],
            )
            .on_request(
                "session/prompt",
                vec![
                    // A streamed assistant message chunk (heuristic, ID-less).
                    ScriptedServerFrame::Update(json!({
                        "sessionId": "acp-session-wire-1",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "Working on it." }
                        }
                    })),
                    // A tool call (stable timeline key by toolCallId).
                    ScriptedServerFrame::Update(json!({
                        "sessionId": "acp-session-wire-1",
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tool-wire-1",
                            "title": "write file",
                            "status": "pending"
                        }
                    })),
                    // The agent asks the client for permission on the wire.
                    ScriptedServerFrame::RequestPermission(json!({
                        "sessionId": "acp-session-wire-1",
                        "toolCall": { "toolCallId": "tool-wire-1" },
                        "options": [
                            { "optionId": "opt-allow", "name": "Allow", "kind": "allow_once" },
                            { "optionId": "opt-reject", "name": "Reject", "kind": "reject_once" }
                        ]
                    })),
                    // The tool completes after the grant.
                    ScriptedServerFrame::Update(json!({
                        "sessionId": "acp-session-wire-1",
                        "update": {
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": "tool-wire-1",
                            "status": "completed",
                            "content": { "type": "text", "text": "done" }
                        }
                    })),
                    // The terminal prompt response.
                    ScriptedServerFrame::Response(json!({ "stopReason": "end_turn" })),
                ],
            );

        let mut client = AcpWireClient::attach(transport, setup_plan());
        let version = client.initialize().expect("initialize");
        assert_eq!(version, 1);
        assert_eq!(client.negotiated_protocol_version(), Some(1));

        let session_id = client
            .session_new("/tmp/capo-acp-wire-ws")
            .expect("session/new");
        assert_eq!(session_id, "acp-session-wire-1");
        assert_eq!(client.external_session_id(), Some("acp-session-wire-1"));

        let transcript = client.prompt(&session_id, "do the task").expect("prompt");

        // The prompt's stopReason is recorded.
        assert_eq!(transcript.stop_reason.as_deref(), Some("end_turn"));

        // The streamed chunk normalized to an item_delta (heuristic confidence).
        let delta = transcript
            .events
            .iter()
            .find(|event| event.kind == "adapter.item_delta")
            .expect("item delta from agent_message_chunk");
        assert_eq!(delta.content.as_deref(), Some("Working on it."));
        assert_eq!(
            delta.timeline_confidence,
            AdapterTimelineConfidence::Heuristic
        );

        // The tool call normalized to a stable tool timeline key.
        assert!(transcript.events.iter().any(|event| {
            event.timeline_key.as_deref() == Some("acp:acp-session-wire-1:tool:tool-wire-1")
                && event.kind == "adapter.tool_call_requested"
        }));
        assert!(transcript.events.iter().any(|event| {
            event.timeline_key.as_deref() == Some("acp:acp-session-wire-1:tool:tool-wire-1")
                && event.kind == "adapter.tool_call_completed"
        }));

        // The live permission round-trip: TrustedLocal selected the allow_once
        // option and answered the agent on the wire.
        assert_eq!(transcript.permission_round_trips.len(), 1);
        let round_trip = &transcript.permission_round_trips[0];
        assert_eq!(round_trip.tool_call_id.as_deref(), Some("tool-wire-1"));
        assert_eq!(round_trip.capo_decision, "allow");
        assert_eq!(
            round_trip.outcome,
            AcpPermissionOutcome::Selected {
                option_id: "opt-allow".to_string()
            }
        );
        assert_eq!(
            round_trip.offered_option_ids,
            vec!["opt-allow".to_string(), "opt-reject".to_string()]
        );
    }

    /// The recorded client frames are well-formed JSON-RPC 2.0 in the expected
    /// order, including the on-wire permission RESPONSE the client sent back to
    /// the agent's `session/request_permission` request. Driving the client
    /// through a `&mut` borrow keeps the transport's recording observable after
    /// the client drops.
    #[test]
    fn client_writes_wellformed_jsonrpc_including_permission_response() {
        let mut transport = ScriptedAcpTransport::new()
            .on_request(
                "initialize",
                vec![ScriptedServerFrame::Response(json!({ "protocolVersion": 1 }))],
            )
            .on_request(
                "session/new",
                vec![ScriptedServerFrame::Response(json!({ "sessionId": "s1" }))],
            )
            .on_request(
                "session/prompt",
                vec![
                    ScriptedServerFrame::RequestPermission(json!({
                        "sessionId": "s1",
                        "toolCall": { "toolCallId": "t1" },
                        "options": [{ "optionId": "opt-allow", "name": "Allow", "kind": "allow_once" }]
                    })),
                    ScriptedServerFrame::Response(json!({ "stopReason": "end_turn" })),
                ],
            );

        {
            let mut client = AcpWireClient::attach(&mut transport, setup_plan());
            client.initialize().unwrap();
            let session_id = client.session_new("/tmp").unwrap();
            client.prompt(&session_id, "go").unwrap();
        }

        let methods: Vec<String> = transport
            .recorded
            .iter()
            .filter_map(|frame| {
                frame
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(
            methods,
            vec![
                "initialize".to_string(),
                "session/new".to_string(),
                "session/prompt".to_string()
            ]
        );

        // Every recorded frame is JSON-RPC 2.0.
        assert!(
            transport
                .recorded
                .iter()
                .all(|frame| frame.get("jsonrpc").and_then(Value::as_str) == Some("2.0"))
        );

        // The permission RESPONSE the client wrote back: no `method`, carries the
        // selected optionId, addressed to the agent's request id.
        let permission_response = transport
            .recorded
            .iter()
            .find(|frame| {
                frame.get("method").is_none() && frame.pointer("/result/outcome/optionId").is_some()
            })
            .expect("client wrote a permission response on the wire");
        assert_eq!(
            permission_response
                .pointer("/result/outcome/outcome")
                .and_then(Value::as_str),
            Some("selected")
        );
        assert_eq!(
            permission_response
                .pointer("/result/outcome/optionId")
                .and_then(Value::as_str),
            Some("opt-allow")
        );
    }

    /// An agent error response to one of our requests surfaces as a typed
    /// [`AcpWireError::AgentError`] rather than a panic or a silent success.
    #[test]
    fn agent_error_response_surfaces_typed_error() {
        let transport = ScriptedAcpTransport::new().on_request(
            "initialize",
            vec![ScriptedServerFrame::Response(json!({ "ignored": true }))],
        );
        // Re-script the reaction to a frame carrying an `error` instead of a
        // `result` (the scripted `Response` rewrites the id to the request id).
        let transport = transport.with_error_reaction(
            "initialize",
            json!({ "code": -32600, "message": "unsupported protocol" }),
        );
        let mut client = AcpWireClient::attach(transport, setup_plan());
        let error = client.initialize().expect_err("agent error must surface");
        assert!(matches!(error, AcpWireError::AgentError { .. }));
    }
}
