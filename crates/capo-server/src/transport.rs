//! capo-server transport: JSON-RPC 2.0 framing over a persistent loopback
//! connection (ST2), served by a concurrent, task-per-connection accept loop
//! with per-connection idle timeouts and an in-band typed `Cancel` (ST3).
//!
//! The wire format is a **JSON-RPC 2.0** request/response pair plus a
//! server-initiated notification variant (see [`jsonrpc`]). It deliberately
//! lives *below* the `AgentAdapter`/`CapoServer` boundary: it is the codec layer
//! that serializes the typed [`ServerRequest`]/[`ServerResponse`] domain
//! surface onto the socket, and it never becomes the domain model. Callers keep
//! using the typed [`send_tcp`]/[`serve_tcp`] API; only the bytes on the wire
//! are JSON-RPC.
//!
//! ## Concurrency model (ST3)
//!
//! `serve_tcp` accepts connections and hands each one to its own OS thread, so
//! many persistent connections are served at once instead of one-at-a-time.
//! Each connection runs a persistent read loop: it reads framed JSON-RPC
//! requests until EOF or the per-connection idle timeout, replying to each on
//! the same socket.
//!
//! ### Single-writer constraint (documented, not silently interleaved)
//!
//! Concurrency here is about serving many *connections*, not about admitting
//! concurrent *writers* to the event log. The append path is not yet guarded by
//! the `safety-gates` single-writer workspace lock, so concurrent writers are
//! **unsupported**: each request runs through the same [`CapoServer::handle`]
//! handler, and the handler is the serialization point. ST3 does not add a
//! second write path; the in-band `Cancel` aborts the *transport's* view of an
//! in-flight request (it stops waiting and frees the connection) rather than
//! racing a second writer into the store.
//!
//! ### In-band `Cancel`
//!
//! A client can abort an in-flight request *without closing the socket* by
//! sending a JSON-RPC `cancel` notification (no `id`) naming the `request_id`
//! to abort. The connection emits a `cancelled` error frame for that request
//! and stays open for subsequent requests. The handler is handed a
//! [`CancellationToken`] it can observe so cooperative work can stop early; the
//! transport never blocks the connection on a cancelled request's eventual
//! completion.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use capo_core::ProjectId;
use serde_json::Value;

mod codec;
mod jsonrpc;
mod wire;

use crate::{CapoServer, ServerError, ServerRequest, ServerResponse};

const MAX_TRANSPORT_FRAME_BYTES: usize = 384 * 1024;

/// Default per-connection idle timeout: a connection that sends no bytes for
/// this long is closed so a stalled or abandoned client cannot hold a
/// connection thread indefinitely. A legitimately-long-lived subscriber (ST4)
/// keeps the connection alive by reading; the read side never trips this. The
/// value is generous enough for interactive request/response round-trips.
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// The JSON-RPC method name for the in-band cancel notification.
const CANCEL_METHOD: &str = "cancel";

/// A cooperative cancellation flag handed to the request handler so in-flight
/// work can observe an in-band `Cancel` and stop early. The transport never
/// *requires* the handler to honor it: a cancelled request frees the connection
/// regardless, and any later result the handler produces is discarded.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether an in-band `Cancel` has been observed for the in-flight request.
    /// Handlers observe this to stop cooperative work early; the production
    /// [`CapoServerHandler`] does not yet (its calls are short), so today this
    /// is exercised by the deterministic in-band-cancel test's scripted handler.
    #[cfg(test)]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The per-request handler the connection loop drives. Production serves this
/// from a shared [`CapoServer`] ([`CapoServerHandler`]); deterministic tests
/// inject a scripted handler to hold a request in-flight while they send an
/// in-band `Cancel`.
pub(crate) trait RequestHandler: Send + Sync + 'static {
    fn handle(
        &self,
        request: ServerRequest,
        cancel: &CancellationToken,
    ) -> TransportResult<ServerResponse>;
}

/// The production handler: each request runs through the shared
/// [`CapoServer::handle`], the single serialization point for writes. It does
/// not currently observe the cancellation token (handler calls are short), but
/// the seam is in place for cooperative cancellation of longer work.
struct CapoServerHandler {
    server: CapoServer,
}

impl RequestHandler for CapoServerHandler {
    fn handle(
        &self,
        request: ServerRequest,
        _cancel: &CancellationToken,
    ) -> TransportResult<ServerResponse> {
        self.server.handle(request).map_err(TransportError::Server)
    }
}

/// Per-connection serving configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ServeConfig {
    /// Idle timeout: a connection idle (no bytes) for this long is closed.
    idle_timeout: Duration,
}

impl ServeConfig {
    /// Build a config with an explicit idle timeout (used by the deterministic
    /// idle-timeout test to keep the assertion fast).
    #[cfg(test)]
    pub(crate) fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self { idle_timeout }
    }
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

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
    let server = CapoServer::open(project_id, state_root).map_err(TransportError::Server)?;
    serve_tcp_with_handler(
        listener,
        Arc::new(CapoServerHandler { server }),
        max_requests,
        ServeConfig::default(),
    )
}

/// Concurrent accept loop: accept up to `max_connections` persistent
/// connections (or unbounded when `None`), serving each on its own thread.
///
/// `max_connections` keeps the historical meaning of the old `max_requests`
/// argument: it bounds how many connections are accepted before the loop
/// returns, which is how the deterministic round-trip tests size the server.
/// Returns the number of connections accepted once all connection threads have
/// finished, so callers (and tests) observe a fully-drained server.
pub(crate) fn serve_tcp_with_handler<H: RequestHandler>(
    listener: TcpListener,
    handler: Arc<H>,
    max_connections: Option<usize>,
    config: ServeConfig,
) -> TransportResult<usize> {
    let bound_address = listener.local_addr().map_err(TransportError::Io)?;
    if !bound_address.ip().is_loopback() {
        return Err(TransportError::Protocol(format!(
            "server listener must be loopback, got {bound_address}"
        )));
    }
    let mut accepted = 0;
    let mut connection_threads = Vec::new();
    while max_connections.map(|max| accepted < max).unwrap_or(true) {
        let (stream, _) = listener.accept().map_err(TransportError::Io)?;
        accepted += 1;
        let handler = Arc::clone(&handler);
        connection_threads.push(thread::spawn(move || {
            // A per-connection error (a peer that dropped, a malformed frame
            // mid-stream, an idle timeout) tears down only that connection; it
            // never poisons the accept loop or sibling connections.
            let _ = handle_connection(handler.as_ref(), stream, config);
        }));
    }
    for connection_thread in connection_threads {
        connection_thread
            .join()
            .map_err(|_| TransportError::Protocol("connection thread panicked".to_string()))?;
    }
    Ok(accepted)
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
    Remote {
        kind: String,
        message: String,
    },
    /// An in-flight request was aborted by an in-band `Cancel` (ST3). It is
    /// distinct from a transport failure: the connection stays open and the
    /// client can issue further requests.
    Cancelled {
        request_id: String,
    },
}

/// One framed line read off a persistent connection, classified by intent.
enum Frame {
    /// A JSON-RPC request to dispatch.
    Request(Box<ServerRequest>),
    /// An in-band `cancel` notification naming the `request_id` to abort.
    Cancel { request_id: Option<String> },
    /// A frame that parsed as JSON-RPC but is neither a dispatchable request
    /// nor a recognized notification, or one that failed bounded/utf-8/JSON
    /// decoding. `id` is the recoverable request id when one was present.
    Invalid {
        id: Option<String>,
        error: TransportError,
    },
}

/// A demuxed connection event the per-connection loop blocks on: either an
/// inbound frame from the read side or the in-flight request's handler result.
/// Folding both onto one channel lets the loop wait on `recv` (no polling) yet
/// react the instant *either* a cancel arrives or the handler finishes.
enum ConnEvent {
    /// An inbound frame read off the socket.
    Incoming(Frame),
    /// The read side reached EOF, idle-timed-out, or hit a hard I/O error.
    Closed,
    /// The in-flight request's handler completed (tagged with the generation it
    /// belongs to so a result for an already-cancelled request is discarded).
    Result {
        generation: u64,
        result: Box<TransportResult<ServerResponse>>,
    },
}

/// Persistent per-connection read loop (ST3): read framed JSON-RPC requests
/// until EOF or the idle timeout, replying to each on the same socket. The read
/// side runs on its own thread and feeds [`ConnEvent`]s to the main loop, so an
/// in-band `cancel` notification can abort the matching in-flight request
/// without dropping the connection, and a stalled peer is reaped by the idle
/// timeout.
fn handle_connection<H: RequestHandler>(
    handler: &H,
    stream: TcpStream,
    config: ServeConfig,
) -> TransportResult<()> {
    // The idle timeout is enforced as a read timeout: a read that blocks longer
    // than this returns `WouldBlock`/`TimedOut`, which we fold into `Closed` so
    // a stalled or abandoned client cannot hold a connection thread.
    stream
        .set_read_timeout(Some(config.idle_timeout))
        .map_err(TransportError::Io)?;
    let mut write_half = stream.try_clone().map_err(TransportError::Io)?;
    let read_half = stream;

    let (event_tx, event_rx) = mpsc::channel::<ConnEvent>();

    thread::scope(|scope| {
        // Read side: classify frames and forward them; a clean EOF, idle
        // timeout, or hard error all terminate the connection via `Closed`.
        let reader_tx = event_tx.clone();
        scope.spawn(move || {
            let mut reader = BufReader::new(read_half);
            loop {
                match read_frame(&mut reader) {
                    Ok(Some(frame)) => {
                        if reader_tx.send(ConnEvent::Incoming(frame)).is_err() {
                            return;
                        }
                    }
                    Ok(None) | Err(_) => {
                        let _ = reader_tx.send(ConnEvent::Closed);
                        return;
                    }
                }
            }
        });

        // Main loop: at most one request in flight. `in_flight` carries the
        // current request id, its cancellation token, and a generation so a
        // late handler result for an already-cancelled request is dropped.
        let mut in_flight: Option<(String, CancellationToken, u64)> = None;
        let mut generation: u64 = 0;
        while let Ok(event) = event_rx.recv() {
            match event {
                ConnEvent::Closed => {
                    if let Some((_, cancel, _)) = &in_flight {
                        cancel.cancel();
                    }
                    return Ok(());
                }
                ConnEvent::Incoming(Frame::Request(request)) => {
                    if in_flight.is_some() {
                        // One request at a time per connection: admitting a
                        // second concurrently would risk a second writer into
                        // the store before the safety-gates write lock lands.
                        let error = TransportError::Protocol(
                            "a request is already in flight on this connection".to_string(),
                        );
                        write_frame(
                            &mut write_half,
                            &jsonrpc::encode_error_response(Some(&request.request_id), &error),
                        )?;
                        continue;
                    }
                    generation += 1;
                    let this_generation = generation;
                    let request_id = request.request_id.clone();
                    let cancel = CancellationToken::new();
                    let worker_cancel = cancel.clone();
                    let worker_tx = event_tx.clone();
                    scope.spawn(move || {
                        let result = handler.handle(*request, &worker_cancel);
                        let _ = worker_tx.send(ConnEvent::Result {
                            generation: this_generation,
                            result: Box::new(result),
                        });
                    });
                    in_flight = Some((request_id, cancel, this_generation));
                }
                ConnEvent::Incoming(Frame::Cancel { request_id: target }) => {
                    if let Some((request_id, cancel, _)) = &in_flight {
                        let matches = target.as_deref().map(|id| id == request_id).unwrap_or(true);
                        if matches {
                            // Abort: signal the worker, emit a `cancelled`
                            // frame, and keep the connection open. The worker's
                            // eventual `Result` is discarded by the generation
                            // check below.
                            cancel.cancel();
                            let error = TransportError::Cancelled {
                                request_id: request_id.clone(),
                            };
                            let frame = jsonrpc::encode_error_response(Some(request_id), &error);
                            write_frame(&mut write_half, &frame)?;
                            in_flight = None;
                        }
                    }
                    // A cancel with nothing matching in flight is a no-op
                    // notification (no response frame is owed).
                }
                ConnEvent::Incoming(Frame::Invalid { id, error }) => {
                    write_frame(
                        &mut write_half,
                        &jsonrpc::encode_error_response(id.as_deref(), &error),
                    )?;
                }
                ConnEvent::Result { generation, result } => {
                    // Drop a result whose request was already cancelled (or
                    // superseded): only the current in-flight generation owes a
                    // response.
                    let owed = in_flight
                        .as_ref()
                        .map(|(_, _, current)| *current == generation)
                        .unwrap_or(false);
                    if !owed {
                        continue;
                    }
                    let (request_id, _, _) = in_flight.take().expect("owed implies in flight");
                    let frame = match *result {
                        Ok(response) => jsonrpc::encode_success_response(&response),
                        Err(error) => jsonrpc::encode_error_response(Some(&request_id), &error),
                    };
                    write_frame(&mut write_half, &frame)?;
                }
            }
        }
        Ok(())
    })
}

/// Read and classify one framed line from the connection. Returns `Ok(None)` on
/// a clean EOF or an idle-timeout read (both close the connection) and surfaces
/// a bounded-frame/utf-8 violation as an [`Frame::Invalid`] error frame so the
/// client still gets a reply before the connection ends.
fn read_frame(reader: &mut BufReader<TcpStream>) -> TransportResult<Option<Frame>> {
    let mut line = Vec::new();
    match read_bounded_line(reader, &mut line) {
        Ok(true) => {}
        // Clean EOF with no buffered bytes: the peer closed the connection.
        Ok(false) => return Ok(None),
        // An idle/stalled read (the per-connection timeout) closes the
        // connection; a hard I/O error does too. Either way there is no frame.
        Err(TransportError::Io(error)) if is_timeout(&error) => return Ok(None),
        Err(TransportError::Io(error)) => return Err(TransportError::Io(error)),
        // A bounded-frame protocol violation is still reported as an error frame
        // (matching the ST2 pre-JSON-decode rejection contract) before close.
        Err(error) => return Ok(Some(Frame::Invalid { id: None, error })),
    }
    let decoded = String::from_utf8(line)
        .map_err(|_| TransportError::Protocol("request frame is not valid utf-8".to_string()))
        .and_then(|line| classify_frame(&line));
    match decoded {
        Ok(frame) => Ok(Some(frame)),
        Err(error) => Ok(Some(Frame::Invalid { id: None, error })),
    }
}

/// Classify a decoded line as a dispatchable request, an in-band cancel, or an
/// invalid frame. Cancel is recognized first because it is a notification (no
/// `id`) and must not be misread as a request.
fn classify_frame(line: &str) -> TransportResult<Frame> {
    let value: Value = serde_json::from_str(line).map_err(TransportError::Json)?;
    let is_notification = value.get("id").is_none();
    let method = value.get("method").and_then(Value::as_str);
    if is_notification && method == Some(CANCEL_METHOD) {
        let request_id = value
            .get("params")
            .and_then(|params| params.get("request_id"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        return Ok(Frame::Cancel { request_id });
    }
    let request = jsonrpc::decode_request(line)?;
    Ok(Frame::Request(Box::new(request)))
}

fn write_frame(stream: &mut TcpStream, frame: &str) -> TransportResult<()> {
    stream
        .write_all(frame.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)
}

/// Read one bounded, newline-terminated frame. Returns `Ok(true)` when a frame
/// was read (with or without a trailing newline at EOF), `Ok(false)` on a clean
/// EOF before any bytes, and an error for an oversized frame or I/O failure.
fn read_bounded_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> TransportResult<bool> {
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            // An idle-timeout read surfaces as `WouldBlock`/`TimedOut`; treat it
            // as the connection going idle so the caller tears it down. A
            // partial line already buffered is abandoned with the connection.
            Err(error) if is_timeout(&error) => return Err(TransportError::Io(error)),
            Err(error) => return Err(TransportError::Io(error)),
        };
        if available.is_empty() {
            // EOF: a frame without a trailing newline is still a frame.
            return Ok(!line.is_empty());
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
            return Ok(true);
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

/// Whether an I/O error is a per-connection read timeout (an idle connection),
/// as opposed to a hard failure. Platforms surface this as either `WouldBlock`
/// or `TimedOut` depending on the socket timeout implementation.
fn is_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}
