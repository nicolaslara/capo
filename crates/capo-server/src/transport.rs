//! capo-server transport: JSON-RPC 2.0 framing over a persistent loopback
//! connection.
//!
//! The wire format is a **JSON-RPC 2.0** request/response pair plus a
//! server-initiated notification variant (see [`jsonrpc`]). It deliberately
//! lives *below* the `AgentAdapter`/`CapoServer` boundary: it is the codec layer
//! that serializes the typed [`ServerRequest`]/[`ServerResponse`] domain
//! surface onto the socket, and it never becomes the domain model. Callers keep
//! using the typed [`send_tcp`]/[`serve_tcp`] API; only the bytes on the wire
//! are JSON-RPC.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;

use capo_core::ProjectId;
use serde_json::Value;

mod codec;
mod jsonrpc;
mod wire;

use crate::{CapoServer, ServerError, ServerRequest, ServerResponse};

const MAX_TRANSPORT_FRAME_BYTES: usize = 384 * 1024;

/// A server-initiated JSON-RPC 2.0 notification: a `method`/`params` frame with
/// no `id`, pushed to a connected client without a prior request.
///
/// ST2 defines and round-trips this frame; the event tail (ST4) and the
/// `capo-web` SSE bridge (ST8) push committed events through it. It is the
/// server-to-client half of the persistent bidirectional connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventNotification {
    pub method: String,
    pub params: Value,
}

impl EventNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }

    /// Serialize this notification to a single JSON-RPC 2.0 wire frame.
    pub fn to_wire_frame(&self) -> String {
        jsonrpc::encode_notification(&jsonrpc::Notification {
            method: self.method.clone(),
            params: self.params.clone(),
        })
    }

    /// Parse a JSON-RPC 2.0 notification wire frame. Errors if the frame
    /// carries an `id` (which would make it a request/response, not a
    /// notification) or is not JSON-RPC 2.0.
    pub fn from_wire_frame(frame: &str) -> TransportResult<Self> {
        let notification = jsonrpc::decode_notification(frame)?;
        Ok(Self {
            method: notification.method,
            params: notification.params,
        })
    }
}

pub fn serve_tcp(
    listener: TcpListener,
    project_id: ProjectId,
    state_root: impl AsRef<Path>,
    max_requests: Option<usize>,
) -> TransportResult<usize> {
    let bound_address = listener.local_addr().map_err(TransportError::Io)?;
    if !bound_address.ip().is_loopback() {
        return Err(TransportError::Protocol(format!(
            "server listener must be loopback, got {bound_address}"
        )));
    }
    let server = CapoServer::open(project_id, state_root).map_err(TransportError::Server)?;
    let mut served = 0;
    while max_requests.map(|max| served < max).unwrap_or(true) {
        let (stream, _) = listener.accept().map_err(TransportError::Io)?;
        handle_stream(&server, stream)?;
        served += 1;
    }
    Ok(served)
}

pub fn send_tcp(
    address: impl ToSocketAddrs,
    request: &ServerRequest,
) -> TransportResult<ServerResponse> {
    let resolved = address
        .to_socket_addrs()
        .map_err(TransportError::Io)?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(TransportError::Protocol(
            "server address resolved to no endpoints".to_string(),
        ));
    }
    if !resolved.iter().all(|address| address.ip().is_loopback()) {
        return Err(TransportError::Protocol(format!(
            "server connect address must resolve only to loopback addresses, got {resolved:?}"
        )));
    }
    let mut stream = TcpStream::connect(resolved.as_slice()).map_err(TransportError::Io)?;
    let request_json = jsonrpc::encode_request(request);
    stream
        .write_all(request_json.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(TransportError::Io)?;
    jsonrpc::decode_response(&line)
}

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Server(ServerError),
    Remote { kind: String, message: String },
}

fn handle_stream(server: &CapoServer, mut stream: TcpStream) -> TransportResult<()> {
    let mut line = Vec::new();
    let read_result = {
        let mut reader = BufReader::new(&mut stream);
        read_bounded_line(&mut reader, &mut line)
    };
    let decoded = read_result
        .and_then(|_| {
            String::from_utf8(line).map_err(|_| {
                TransportError::Protocol("request frame is not valid utf-8".to_string())
            })
        })
        .and_then(|line| jsonrpc::decode_request(&line));
    let response_line = match decoded {
        Ok(request) => {
            // Echo the JSON-RPC `id` (= request_id) so request-identity
            // idempotency is observable on both success and error.
            let request_id = request.request_id.clone();
            match server.handle(request).map_err(TransportError::Server) {
                Ok(response) => jsonrpc::encode_success_response(&response),
                Err(error) => jsonrpc::encode_error_response(Some(&request_id), &error),
            }
        }
        // A frame we could not even parse has no recoverable id.
        Err(error) => jsonrpc::encode_error_response(None, &error),
    };
    stream
        .write_all(response_line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)
}

fn read_bounded_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> TransportResult<()> {
    loop {
        let available = reader.fill_buf().map_err(TransportError::Io)?;
        if available.is_empty() {
            return Ok(());
        }
        let consumed = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None => available.len(),
        };
        if line.len() + consumed > MAX_TRANSPORT_FRAME_BYTES {
            reader.consume(consumed);
            drain_to_line_end(reader)?;
            return Err(TransportError::Protocol(format!(
                "request frame is too large: > {MAX_TRANSPORT_FRAME_BYTES} bytes"
            )));
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if line.ends_with(b"\n") {
            return Ok(());
        }
    }
}

fn drain_to_line_end<R: BufRead>(reader: &mut R) -> TransportResult<()> {
    loop {
        let available = reader.fill_buf().map_err(TransportError::Io)?;
        if available.is_empty() {
            return Ok(());
        }
        let consumed = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => index + 1,
            None => available.len(),
        };
        let has_newline = available[..consumed].contains(&b'\n');
        reader.consume(consumed);
        if has_newline {
            return Ok(());
        }
    }
}
