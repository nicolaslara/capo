//! The published, web-client-independent wire contract (ST9).
//!
//! This module is the single in-code source of truth for two artifacts that are
//! *checked into the tree* under `crates/capo-server/contract/`:
//!
//! 1. A language-neutral **schema** ([`contract_schema`]) describing every
//!    JSON-RPC method (the typed `ServerCommand` surface), the success/error
//!    response envelopes, the server-initiated notification variant (the live
//!    event tail and the in-band `cancel`/`interrupt` client notifications), and
//!    the SSE event-tail framing a browser bridge re-exposes.
//! 2. A set of **wire snapshots** ([`wire_samples`]) -- real serialized frames,
//!    produced by the *same* `jsonrpc`/`codec`/[`EventNotification`] path the
//!    live transport uses, never hand-typed JSON. The checked-in copies are the
//!    enforced source of truth; the `contract::wire_snapshots_match_checked_in`
//!    test fails on any unintended wire-shape change so the contract cannot drift
//!    silently.
//!
//! The schema and snapshots are the authoritative contract. TypeScript types
//! (`contract/capo-wire.d.ts`) are an optional downstream convenience generated
//! FROM this schema, owned by the web agent, and are not themselves the
//! contract. Everything here is verified WITHOUT any web client: the samples are
//! built by serializing typed `ServerRequest`/`ServerResponse`/`ServerEvent`
//! values through the production codec.
//!
//! ## SSE framing
//!
//! The persistent JSON-RPC connection carries an `event` notification per
//! committed event (see [`EventNotification::for_event`]). A browser bridge
//! (`capo-web`, ST8) re-exposes the *same* notification frame over Server-Sent
//! Events. Per the SSE wire format, one event is an `event:`/`data:` block
//! terminated by a blank line; [`sse_frame`] is the canonical encoding and is
//! pinned by a snapshot so the SSE shape is part of the published contract even
//! though `capo-web` is not built in this workspace.

use serde_json::{Value, json};

use super::jsonrpc::{
    Notification, encode_error_response, encode_notification, encode_request,
    encode_success_response,
};
use super::{EVENT_TAIL_METHOD, EventNotification, TransportError};
use crate::{
    AgentSummary, ServerClientOrigin, ServerCommand, ServerEvent, ServerInputOrigin, ServerRequest,
    ServerResponse, ServerResponsePayload, SubscriptionBacklog,
};

/// The SSE `event:` type a browser bridge labels each event-tail frame with. It
/// mirrors the JSON-RPC notification method ([`EVENT_TAIL_METHOD`]) so the SSE
/// stream and the raw socket stream carry the same event-name vocabulary.
pub const SSE_EVENT_NAME: &str = EVENT_TAIL_METHOD;

/// Encode a server-to-client notification frame as a single Server-Sent Events
/// block: an `event:` line naming the event type and a `data:` line carrying the
/// exact JSON-RPC notification frame, terminated by the mandatory blank line.
///
/// The `data:` payload is byte-for-byte the same JSON-RPC notification the raw
/// socket transport pushes, so a browser bridge re-exposes the contract verbatim
/// rather than inventing a second wire shape. This is the canonical SSE framing
/// of the published contract; `capo-web` (ST8) reuses it.
pub fn sse_frame(notification: &EventNotification) -> String {
    format!(
        "event: {}\ndata: {}\n\n",
        SSE_EVENT_NAME,
        notification.to_wire_frame()
    )
}

/// A single named wire snapshot: a stable file name plus the exact frame the
/// production codec emits. The file name (without extension) is the snapshot's
/// identity on disk under `contract/snapshots/`.
#[derive(Clone, Debug)]
pub struct WireSample {
    /// Stable snapshot identity (also the on-disk file stem).
    pub name: &'static str,
    /// One sentence describing what the frame is, mirrored into the schema docs.
    pub description: &'static str,
    /// The exact serialized frame, produced by the live transport codec.
    pub frame: String,
}

/// A fixed, deterministic origin so the request/response snapshots are stable
/// across runs and machines (no clock, no randomness).
fn fixed_origin() -> ServerClientOrigin {
    ServerClientOrigin {
        client_id: "local-cli".to_string(),
        actor_id: "local-user".to_string(),
        input_origin: ServerInputOrigin::Cli,
    }
}

/// A fixed request with a stable, explicit `request_id` (which becomes the
/// JSON-RPC `id`), so the snapshot pins the id propagation too.
fn request(request_id: &str, command: ServerCommand) -> ServerRequest {
    ServerRequest {
        request_id: request_id.to_string(),
        origin: fixed_origin(),
        command,
    }
}

/// A fixed successful response echoing a stable origin and `request_id`.
fn response(request_id: &str, payload: ServerResponsePayload) -> ServerResponse {
    ServerResponse {
        request_id: request_id.to_string(),
        client_id: "local-cli".to_string(),
        actor_id: "local-user".to_string(),
        input_origin: ServerInputOrigin::Cli,
        payload,
    }
}

/// A fixed committed event used in the event-tail snapshots. All fields are set
/// to stable literals so the snapshot pins the full `ServerEvent` wire shape.
fn sample_event() -> ServerEvent {
    ServerEvent {
        sequence: 44,
        event_id: "event-0000000044".to_string(),
        kind: "session.summary_updated".to_string(),
        actor: "local-user".to_string(),
        project_id: Some("project-capo".to_string()),
        task_id: Some("task-demo".to_string()),
        agent_id: Some("agent-demo".to_string()),
        session_id: Some("session-demo".to_string()),
        run_id: Some("run-demo".to_string()),
        turn_id: Some("turn-2".to_string()),
        item_id: Some("item-7".to_string()),
        payload_json: "{\"summary\":\"inspected workspace state\"}".to_string(),
        redaction_state: "safe".to_string(),
    }
}

/// The complete, ordered set of checked-in wire snapshots, each produced by the
/// live transport codec. Adding or changing a frame here is a deliberate change
/// to the published contract and must be accompanied by regenerating the
/// checked-in copies (`CAPO_REGENERATE_WIRE_SNAPSHOTS=1`).
pub fn wire_samples() -> Vec<WireSample> {
    // --- Request frames (client -> server), one per representative method. ---
    let list_agents = request("req-list-agents-1", ServerCommand::ListAgents);
    let subscribe = request(
        "req-subscribe-1",
        ServerCommand::Subscribe {
            session_id: Some("session-demo".to_string()),
            from_sequence: 42,
        },
    );
    let read_thread = request(
        "req-read-thread-1",
        ServerCommand::ReadThread {
            session_id: "session-demo".to_string(),
            from_sequence: 7,
        },
    );

    // --- Response frames (server -> client). ---
    let agents_response = response(
        "req-list-agents-1",
        ServerResponsePayload::Agents(vec![AgentSummary {
            agent_id: capo_core::AgentId::new("agent-demo"),
            name: "demo".to_string(),
            status: "available".to_string(),
            current_session_id: None,
            session: None,
        }]),
    );
    let subscribed_response = response(
        "req-subscribe-1",
        ServerResponsePayload::Subscribed(SubscriptionBacklog {
            session_id: Some("session-demo".to_string()),
            from_sequence: 42,
            next_sequence: 44,
            events: vec![sample_event()],
        }),
    );

    // --- Error response frame (server -> client). A cancelled in-flight request
    // is the canonical typed error: it pins the `error.code`, the human message,
    // and the machine-readable `error.data.kind` clients branch on. ---
    let cancelled_error = encode_error_response(
        Some("req-subscribe-1"),
        &TransportError::Cancelled {
            request_id: "req-subscribe-1".to_string(),
        },
    );

    // --- Server-initiated notification (server -> client): the live event tail.
    let event_notification = EventNotification::for_event(&sample_event());

    // --- Client-initiated notifications (client -> server, no id): the in-band
    // cancel (request-id-scoped) and the typed mid-turn interrupt (session-scoped).
    let cancel_notification = Notification {
        method: super::CANCEL_METHOD.to_string(),
        params: json!({ "request_id": "req-subscribe-1" }),
    };
    let interrupt_notification = Notification {
        method: super::INTERRUPT_METHOD.to_string(),
        params: json!({ "session_id": "session-demo", "reason": "operator ctrl-c" }),
    };

    vec![
        WireSample {
            name: "request-list-agents",
            description: "JSON-RPC request: a no-params method (list_agents) carrying only origin.",
            frame: encode_request(&list_agents),
        },
        WireSample {
            name: "request-subscribe",
            description: "JSON-RPC request: Subscribe { session_id, from_sequence } -- opens the event tail.",
            frame: encode_request(&subscribe),
        },
        WireSample {
            name: "request-read-thread",
            description: "JSON-RPC request: ReadThread { session_id, from_sequence } -- incremental thread read.",
            frame: encode_request(&read_thread),
        },
        WireSample {
            name: "response-agents",
            description: "JSON-RPC success response: result.payload carries the typed Agents payload.",
            frame: encode_success_response(&agents_response),
        },
        WireSample {
            name: "response-subscribed",
            description: "JSON-RPC success response to Subscribe: the catch-up backlog of committed events.",
            frame: encode_success_response(&subscribed_response),
        },
        WireSample {
            name: "response-error-cancelled",
            description: "JSON-RPC error response: a cancelled in-flight request (error.data.kind=cancelled).",
            frame: cancelled_error,
        },
        WireSample {
            name: "notification-event-tail",
            description: "Server-initiated notification (no id): one committed event on the live tail.",
            frame: encode_notification(&Notification {
                method: event_notification.method.clone(),
                params: event_notification.params.clone(),
            }),
        },
        WireSample {
            name: "notification-cancel",
            description: "Client notification (no id): in-band cancel of an in-flight request by request_id.",
            frame: encode_notification(&cancel_notification),
        },
        WireSample {
            name: "notification-interrupt",
            description: "Client notification (no id): typed mid-turn interrupt of a session by session_id.",
            frame: encode_notification(&interrupt_notification),
        },
        WireSample {
            name: "sse-event-tail",
            description: "SSE re-exposure of the event-tail notification: an event:/data: block (capo-web/ST8).",
            frame: sse_frame(&event_notification),
        },
    ]
}

/// The language-neutral schema for the published contract: a hand-authored,
/// JSON-Schema-shaped document describing the envelope grammar, the method
/// surface, the notification variant, the error frame, and the SSE framing.
///
/// This is the human/cross-team-readable companion to [`wire_samples`]: the
/// snapshots are the *enforced* source of truth (they fail a test on drift),
/// and this schema is the *described* contract a web agent (or any client) reads
/// to implement against. The two are kept consistent by the contract test, which
/// also asserts every method/notification named here appears in the snapshots.
pub fn contract_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://capo.local/contract/jsonrpc-event-stream/v1",
        "title": "Capo JSON-RPC 2.0 + event-stream wire contract",
        "version": "1",
        "description": concat!(
            "The authoritative, web-client-independent wire contract for the Capo ",
            "server transport: JSON-RPC 2.0 request/response framing over a ",
            "persistent bidirectional connection, a server-initiated notification ",
            "variant carrying the live event tail, client-initiated cancel/interrupt ",
            "notifications, and the SSE re-exposure a browser bridge consumes. ",
            "Checked-in wire snapshots under contract/snapshots/ are the enforced ",
            "source of truth; this schema is the described companion. TypeScript ",
            "types under contract/ are an optional downstream convenience generated ",
            "from this schema, not the contract itself."
        ),
        "envelope": {
            "jsonrpc": {
                "const": "2.0",
                "description": "Every frame on the connection (and inside an SSE data line) is JSON-RPC 2.0."
            },
            "request": {
                "description": "Client -> server. The JSON-RPC id mirrors the existing request_id (idempotency key).",
                "required": ["jsonrpc", "id", "method", "params"],
                "properties": {
                    "jsonrpc": { "const": "2.0" },
                    "id": { "type": "string", "description": "The request_id; echoed on the matching response." },
                    "method": {
                        "type": "string",
                        "description": "The ServerCommand discriminant (snake_case type tag).",
                        "enum": command_methods()
                    },
                    "params": {
                        "type": "object",
                        "description": "The command fields plus a reserved `origin` object.",
                        "required": ["origin"],
                        "properties": {
                            "origin": { "$ref": "#/components/origin" }
                        }
                    }
                }
            },
            "success_response": {
                "description": "Server -> client. Carries the matching id and a typed result.payload.",
                "required": ["jsonrpc", "id", "result"],
                "properties": {
                    "jsonrpc": { "const": "2.0" },
                    "id": { "type": "string", "description": "Mirrors the request id." },
                    "result": {
                        "type": "object",
                        "required": ["client_id", "actor_id", "input_origin", "payload"],
                        "properties": {
                            "client_id": { "type": "string" },
                            "actor_id": { "type": "string" },
                            "input_origin": { "$ref": "#/components/input_origin" },
                            "payload": {
                                "type": "object",
                                "description": "Tagged by `type`; one of the ServerResponsePayload variants.",
                                "required": ["type"],
                                "properties": {
                                    "type": { "type": "string", "enum": payload_types() }
                                }
                            }
                        }
                    }
                }
            },
            "error_response": {
                "description": "Server -> client. id is null when the request could not be parsed.",
                "required": ["jsonrpc", "id", "error"],
                "properties": {
                    "jsonrpc": { "const": "2.0" },
                    "id": { "type": ["string", "null"] },
                    "error": {
                        "type": "object",
                        "required": ["code", "message", "data"],
                        "properties": {
                            "code": {
                                "const": -32603,
                                "description": "Always JSON-RPC Internal error; the precise kind is in data.kind."
                            },
                            "message": { "type": "string" },
                            "data": {
                                "type": "object",
                                "required": ["kind"],
                                "properties": {
                                    "kind": {
                                        "type": "string",
                                        "description": "Machine-readable Capo error kind clients branch on.",
                                        "enum": error_kinds()
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "notification": {
                "description": "No id. Server-initiated (the live event tail) or client-initiated (cancel/interrupt).",
                "required": ["jsonrpc", "method", "params"],
                "properties": {
                    "jsonrpc": { "const": "2.0" },
                    "id": { "not": {}, "description": "A notification MUST NOT carry an id." },
                    "method": { "type": "string", "enum": notification_methods() },
                    "params": { "type": "object" }
                }
            },
            "sse": {
                "description": concat!(
                    "A browser bridge re-exposes each `event` notification over Server-Sent ",
                    "Events as `event: <name>\\ndata: <json-rpc notification frame>\\n\\n`. The ",
                    "data line is byte-for-byte the JSON-RPC notification, so SSE and the raw ",
                    "socket carry the same wire shape."
                ),
                "event_name": SSE_EVENT_NAME,
                "data_is": "the JSON-RPC `event` notification frame, verbatim"
            }
        },
        "methods": method_descriptors(),
        "notifications": notification_descriptors(),
        "components": {
            "origin": {
                "type": "object",
                "required": ["client_id", "actor_id", "input_origin"],
                "properties": {
                    "client_id": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "input_origin": { "$ref": "#/components/input_origin" }
                }
            },
            "input_origin": {
                "type": "string",
                "enum": ["cli", "dashboard", "mobile", "voice", "api", "system"]
            },
            "event": {
                "type": "object",
                "description": "One committed event, identical in the Subscribed backlog and the live `event` notification.",
                "required": ["sequence", "event_id", "kind", "actor", "payload_json", "redaction_state"],
                "properties": {
                    "sequence": { "type": "integer", "description": "Monotonic commit watermark; resume a Subscribe from here." },
                    "event_id": { "type": "string" },
                    "kind": { "type": "string" },
                    "actor": { "type": "string" },
                    "project_id": { "type": ["string", "null"] },
                    "task_id": { "type": ["string", "null"] },
                    "agent_id": { "type": ["string", "null"] },
                    "session_id": { "type": ["string", "null"] },
                    "run_id": { "type": ["string", "null"] },
                    "turn_id": { "type": ["string", "null"] },
                    "item_id": { "type": ["string", "null"] },
                    "payload_json": { "type": "string", "description": "The event body, already redacted on egress (ST7)." },
                    "redaction_state": {
                        "type": "string",
                        "description": "Egress classification; a withheld/sensitive body is downgraded to `redacted`."
                    }
                }
            }
        }
    })
}

/// The JSON-RPC method names every typed `ServerCommand` maps onto, in the
/// declaration order of the enum. Kept exhaustive by a `match` in the contract
/// test so a new command cannot be added without extending the published schema.
fn command_methods() -> Vec<&'static str> {
    vec![
        "register_agent",
        "send_task",
        "steer_agent",
        "interrupt_agent",
        "stop_agent",
        "list_agents",
        "agent_status",
        "dashboard",
        "start_session",
        "replay_adapter_fixture",
        "plan_dispatch",
        "preflight_live_provider",
        "gate_dispatch",
        "run_dispatch_local",
        "run_live_provider_local",
        "recover",
        "subscribe",
        "read_thread",
    ]
}

/// The `result.payload.type` tags every `ServerResponsePayload` variant emits.
fn payload_types() -> Vec<&'static str> {
    vec![
        "agent_registered",
        "task_sent",
        "agents",
        "agent_status",
        "dashboard",
        "session_started",
        "adapter_fixture_replayed",
        "dispatch_planned",
        "live_provider_preflighted",
        "dispatch_gated",
        "dispatch_run",
        "recovery",
        "subscribed",
        "thread",
    ]
}

/// The `error.data.kind` values a client may observe.
fn error_kinds() -> Vec<&'static str> {
    vec![
        "io",
        "json",
        "protocol",
        "state",
        "adapter_fixture",
        "unknown_agent",
        "agent_has_no_active_session",
        "agent_already_has_session",
        "session_already_exists",
        "run_already_exists",
        "unknown_dispatch_plan",
        "unknown_session",
        "run_session_mismatch",
        "adapter_session_mismatch",
        "remote",
        "cancelled",
        "interrupted",
    ]
}

/// The method names that travel as JSON-RPC notifications (no id): the
/// server-initiated event tail and the client-initiated cancel/interrupt.
fn notification_methods() -> Vec<&'static str> {
    vec![
        EVENT_TAIL_METHOD,
        super::CANCEL_METHOD,
        super::INTERRUPT_METHOD,
    ]
}

fn method_descriptors() -> Value {
    json!({
        "list_agents": "List registered agents. No params beyond origin.",
        "subscribe": "Open the event tail: catch-up backlog strictly after from_sequence, then live `event` notifications.",
        "read_thread": "Read a session's projected multi-turn thread incrementally from from_sequence.",
        "_note": "Every ServerCommand maps 1:1 onto a method; see envelope.request.method.enum for the full set."
    })
}

fn notification_descriptors() -> Value {
    json!({
        "event": "Server -> client. One committed event on the live tail; params.event is the shared event shape.",
        "cancel": "Client -> server. Abort one in-flight request by params.request_id (request-id-scoped).",
        "interrupt": "Client -> server. Typed mid-turn interrupt of params.session_id with params.reason (session-scoped)."
    })
}
