//! JSON-RPC 2.0 framing for the capo-server transport (ST2).
//!
//! This module owns the *wire envelope* only: a JSON-RPC 2.0 request/response
//! pair plus a server-initiated notification variant. It sits strictly below
//! the `AgentAdapter`/`CapoServer` boundary -- it is the codec layer, not the
//! domain model. The typed [`ServerCommand`]/[`ServerResponsePayload`] surface
//! in `crate::types` remains the domain contract; this layer only serializes it
//! onto the persistent bidirectional connection.
//!
//! Mapping decisions (see `workpads/streaming-transport/knowledge.md`):
//!
//! - The JSON-RPC `method` is exactly the existing command `type` discriminant
//!   (`register_agent`, `dashboard`, ...). Every [`ServerCommand`] therefore
//!   maps onto a JSON-RPC method 1:1 with no change to domain semantics.
//! - The command fields plus the request `origin` become the JSON-RPC `params`
//!   object. The origin (`client_id`/`actor_id`/`input_origin`) travels under a
//!   reserved `params.origin` key so the existing origin propagation continues
//!   to flow through to the server handler.
//! - The JSON-RPC `id` carries the existing `request_id`. Request-identity
//!   idempotency is preserved: the same `request_id` the handler keys on is the
//!   JSON-RPC `id` echoed back on the response.
//! - A **notification** (no `id`, server-initiated) lets the server push an
//!   event to a connected client without a prior request. ST4 fills these with
//!   the event tail; ST2 defines and round-trips the frame shape.

use serde_json::{Value, json};

use super::codec::{
    decode_command, decode_origin, decode_response_result, encode_command, encode_origin,
    encode_response_result,
};
use super::wire::{parse_value, required_string, required_value, transport_error_wire};
use super::{TransportError, TransportResult};
use crate::{ServerRequest, ServerResponse};

/// The only JSON-RPC version this transport speaks.
const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC error code for a malformed/invalid request envelope. We map every
/// transport-layer failure onto the standard "Internal error" code and carry
/// the precise Capo error `kind` in `error.data.kind`, which the client lifts
/// back into a [`TransportError::Remote`].
const JSONRPC_INTERNAL_ERROR: i64 = -32603;

/// Encode a [`ServerRequest`] as a JSON-RPC 2.0 request frame.
///
/// Shape: `{"jsonrpc":"2.0","id":<request_id>,"method":<type>,"params":{...,"origin":{...}}}`.
pub(super) fn encode_request(request: &ServerRequest) -> String {
    // `encode_command` always produces a JSON object tagged with a `type`
    // discriminant. The discriminant becomes the JSON-RPC method; the remaining
    // fields plus the request origin become `params`.
    let mut params = encode_command(&request.command);
    let method = method_of(&params);
    if let Value::Object(map) = &mut params {
        map.remove("type");
        map.insert("origin".to_string(), encode_origin(&request.origin));
    }
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": request.request_id,
        "method": method,
        "params": params,
    })
    .to_string()
}

/// Decode a JSON-RPC 2.0 request frame back into a [`ServerRequest`].
pub(super) fn decode_request(line: &str) -> TransportResult<ServerRequest> {
    let value = parse_value(line)?;
    expect_version(&value)?;
    let method = required_string(&value, "method")?;
    let params = required_value(&value, "params")?;
    let origin = params
        .get("origin")
        .ok_or_else(|| TransportError::Protocol("missing params.origin".to_string()))
        .and_then(decode_origin)?;
    // Reconstruct the legacy command object (`type` + fields) the existing
    // `decode_command` understands, so the domain mapping stays untouched.
    let mut command_value = params.clone();
    if let Value::Object(map) = &mut command_value {
        map.remove("origin");
        map.insert("type".to_string(), Value::String(method));
    } else {
        return Err(TransportError::Protocol(
            "params must be a JSON object".to_string(),
        ));
    }
    let command = decode_command(&command_value)?;
    Ok(ServerRequest {
        request_id: required_string(&value, "id")?,
        origin,
        command,
    })
}

/// Encode a successful [`ServerResponse`] as a JSON-RPC 2.0 response frame.
///
/// Shape: `{"jsonrpc":"2.0","id":<request_id>,"result":{...}}`.
pub(super) fn encode_success_response(response: &ServerResponse) -> String {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": response.request_id,
        "result": encode_response_result(response),
    })
    .to_string()
}

/// Encode a transport error as a JSON-RPC 2.0 error response frame.
///
/// The precise Capo error `kind` is preserved in `error.data.kind` so a client
/// can reconstruct a [`TransportError::Remote { kind, message }`]. `id` is
/// `null` when the request could not be parsed (no recoverable id).
pub(super) fn encode_error_response(id: Option<&str>, error: &TransportError) -> String {
    let (kind, message) = transport_error_wire(error);
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": {
            "code": JSONRPC_INTERNAL_ERROR,
            "message": message,
            "data": { "kind": kind },
        },
    })
    .to_string()
}

/// Decode a JSON-RPC 2.0 response frame (success or error) for the client.
pub(super) fn decode_response(line: &str) -> TransportResult<ServerResponse> {
    let value = parse_value(line)?;
    expect_version(&value)?;
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let kind = error
            .get("data")
            .and_then(|data| data.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("remote")
            .to_string();
        return Err(TransportError::Remote {
            kind,
            message: required_string(error, "message")?,
        });
    }
    let result = required_value(&value, "result")?;
    decode_response_result(required_string(&value, "id")?, result)
}

/// A server-initiated JSON-RPC 2.0 notification: a `method`/`params` frame with
/// no `id`. ST2 defines the frame shape and proves it round-trips; ST4 wires
/// the event tail through it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Notification {
    pub(super) method: String,
    pub(super) params: Value,
}

/// Encode a server-initiated notification frame (no `id`).
pub(super) fn encode_notification(notification: &Notification) -> String {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": notification.method,
        "params": notification.params,
    })
    .to_string()
}

/// Decode a notification frame, rejecting any frame that carries an `id` (which
/// would make it a request/response, not a notification).
pub(super) fn decode_notification(line: &str) -> TransportResult<Notification> {
    let value = parse_value(line)?;
    expect_version(&value)?;
    if value.get("id").is_some() {
        return Err(TransportError::Protocol(
            "notification frame must not carry an id".to_string(),
        ));
    }
    Ok(Notification {
        method: required_string(&value, "method")?,
        params: required_value(&value, "params")?.clone(),
    })
}

fn expect_version(value: &Value) -> TransportResult<()> {
    match value.get("jsonrpc").and_then(Value::as_str) {
        Some(JSONRPC_VERSION) => Ok(()),
        other => Err(TransportError::Protocol(format!(
            "unsupported jsonrpc version: {}",
            other.unwrap_or("<missing>")
        ))),
    }
}

/// Lift the `type` discriminant out of an encoded command object to use as the
/// JSON-RPC method name.
fn method_of(command_value: &Value) -> String {
    command_value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}
