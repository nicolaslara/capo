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
//! many persistent connections are served at once instead of one-at-a-time. The
//! accept loop is bounded by a configurable concurrency ceiling
//! ([`ServeConfig::max_concurrent_connections`]): a counting gate blocks the
//! accept loop once that many connections are live, so a loopback
//! connection-flood (or a buggy reconnect loop) cannot spawn unbounded threads
//! and file descriptors. Each connection runs a persistent read loop: it reads
//! framed JSON-RPC requests until EOF or the per-connection idle timeout,
//! replying to each on the same socket.
//!
//! ### Single-writer constraint (enforced, not just documented)
//!
//! Concurrency here is about serving many *connections*, not about admitting
//! concurrent *writers* to the event log. The append path is not yet guarded by
//! the `safety-gates` single-writer workspace lock, so until that lands the
//! transport enforces single-writer semantics itself: every *write-bearing*
//! command (anything other than the read-only `ListAgents` / `AgentStatus` /
//! `Dashboard`, see [`crate::ServerCommand::is_read_only`]) runs through
//! [`RequestHandler::handle`] while holding a process-wide write lock
//! ([`WriteSerializer`]), so at most one writer into the store executes at any
//! instant across all connections. That makes the handler a *real* serialization
//! point for writes rather than an aspirational one. Read-only commands skip the
//! lock, so the concurrency ST3 advertises is genuine for them while writes
//! never interleave. As defense in depth the SQLite store also runs in WAL mode
//! with a `busy_timeout`, so a reader overlapping a write (or any future second
//! writer) blocks-and-retries rather than racing to a `SQLITE_BUSY` error. ST3
//! does not add a second logical write path.
//!
//! ### In-band `Cancel`
//!
//! A client can abort an in-flight request *without closing the socket* by
//! sending a JSON-RPC `cancel` notification (no `id`) naming the `request_id`
//! to abort. The connection emits a `cancelled` error frame for that request
//! and immediately resumes serving subsequent requests. The handler is handed a
//! [`CancellationToken`] it can observe so cooperative work stops early; a
//! production handler that observes it (via [`CancellationToken::is_cancelled`],
//! available in all builds) returns promptly.
//!
//! ### In-band `Interrupt` (typed mid-turn interrupt, ST6)
//!
//! Distinct from `cancel` (which aborts one request by `id`) and from the
//! coarse `StopAgent` domain command, an `interrupt` notification names a
//! `session_id` and a `reason`: it is the typed *mid-turn* interrupt a client
//! (the CLI Ctrl-C) sends on the open connection to abort the live
//! generation/run for a session. When it matches the in-flight request the
//! transport (1) signals that request's [`CancellationToken`] as *interrupted*
//! (carrying the reason) so the running turn cooperatively reaps its runtime
//! process group, (2) invokes [`RequestHandler::interrupt`] so the server
//! records the typed turn-aborted event (`session.interrupted`) the thread
//! projection renders, and (3) emits a typed `interrupted` frame and keeps the
//! connection open. The interrupt drives the runtime's process-group kill (ST1)
//! so descendants are reaped, leaving no surviving runtime process group.
//!
//! The request worker runs on a *detached* thread, not a scoped one, so the
//! connection's read loop never blocks waiting for a cancelled (or
//! idle-timed-out) request's worker to finish. A genuinely long, *non*-
//! cooperative handler call keeps running to natural completion on its detached
//! thread -- but it no longer pins the connection thread, the accept loop, or
//! server drain. While that orphaned worker runs it still holds the write lock,
//! so the single-writer guarantee above is never violated; the only residual is
//! that a stuck non-cooperative handler delays the *next* write until it
//! returns, which is exactly the serialization the write lock promises.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use capo_core::ProjectId;
use capo_runtime::ExposurePolicy;
use serde_json::Value;

mod codec;
pub mod contract;
mod jsonrpc;
mod wire;

use crate::event_tail::TailRecvError;
use crate::{
    CapoServer, ServerCommand, ServerError, ServerRequest, ServerResponse, ServerResponsePayload,
};

const MAX_TRANSPORT_FRAME_BYTES: usize = 384 * 1024;

/// Default per-connection idle timeout: a connection that sends no bytes for
/// this long is closed so a stalled or abandoned client cannot hold a
/// connection thread indefinitely. A legitimately-long-lived subscriber (ST4)
/// keeps the connection alive by reading; the read side never trips this. The
/// value is generous enough for interactive request/response round-trips.
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Default poll interval for a live event-tail pump (ST4/ST11): how long it
/// blocks on the next committed event before waking to re-check its stop flag. A
/// committed event wakes the pump immediately (the wait is interruptible by a
/// send), so this only bounds how soon a tail notices its connection closed.
const DEFAULT_TAIL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The JSON-RPC method name for the in-band cancel notification.
const CANCEL_METHOD: &str = "cancel";

/// The JSON-RPC method name for the typed mid-turn interrupt notification (ST6).
/// Its `params` carry `session_id` and `reason`; it is distinct from `cancel`
/// (request-id-scoped) and from the coarse `StopAgent` domain command.
const INTERRUPT_METHOD: &str = "interrupt";

/// A cooperative cancellation flag handed to the request handler so in-flight
/// work can observe an in-band `Cancel` *or* a typed mid-turn `Interrupt` (ST6)
/// and stop early. The transport never *requires* the handler to honor it: a
/// cancelled/interrupted request frees the connection regardless, and any later
/// result the handler produces is discarded.
///
/// Cancel and interrupt share the stop flag ([`Self::is_cancelled`] is `true`
/// for both) so an existing cooperative handler honoring cancel also honors an
/// interrupt without change. An *interrupt* additionally records its reason, so
/// a handler that drives a runtime process-group kill on interrupt can label the
/// turn-aborted event with it.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    stopped: Arc<AtomicBool>,
    /// `Some(reason)` once a typed mid-turn `Interrupt` has been observed for
    /// the in-flight request; `None` for a plain `Cancel` (or while still live).
    interrupt_reason: Arc<Mutex<Option<String>>>,
}

impl CancellationToken {
    fn new() -> Self {
        Self::default()
    }

    fn cancel(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Signal a typed mid-turn interrupt with its reason. Sets the shared stop
    /// flag (so a cancel-aware handler also stops) and records the reason so the
    /// handler can label the runtime process-group kill / turn-aborted event.
    fn interrupt(&self, reason: &str) {
        *self
            .interrupt_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason.to_string());
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Whether an in-band `Cancel` *or* `Interrupt` has been observed for the
    /// in-flight request. Handlers observe this to stop cooperative work early.
    /// It is available in all builds so a production handler can poll it during
    /// long work; the current production [`CapoServerHandler`] does not need to
    /// for its short request calls, but the cooperative path is real, not
    /// test-only (the ST6 mid-turn-interrupt test's scripted turn polls it to
    /// drive a runtime process-group kill).
    pub fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// The interrupt reason once a typed mid-turn `Interrupt` (not a plain
    /// `Cancel`) has been observed, else `None`. A handler running a live turn
    /// reads this to label the runtime process-group kill and the turn-aborted
    /// event it records.
    pub fn interrupt_reason(&self) -> Option<String> {
        self.interrupt_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// A process-wide write-serialization point. Holding this around a handler call
/// guarantees at most one *write-bearing* handler call -- and therefore at most
/// one writer into the event log -- runs at any instant across every
/// connection, which is the single-writer guarantee ST3 documents. The
/// connection loop wraps only write-bearing commands in it (read-only commands
/// skip it and run concurrently). It is a placeholder for the `safety-gates`
/// workspace write lock: when that lands it can subsume this.
///
/// It is intentionally a plain `Mutex<()>` (not a `RwLock`): readers are handled
/// by *not taking the lock at all* (gated by the command kind plus SQLite WAL),
/// so there is no in-lock read/write split to model, and serializing writes is
/// the simplest thing that makes the documented guarantee true.
#[derive(Clone, Debug, Default)]
pub(crate) struct WriteSerializer(Arc<Mutex<()>>);

impl WriteSerializer {
    /// Run `f` while holding the write lock, so it is serialized against every
    /// other write-bearing handler call. A poisoned lock (a previous handler
    /// panicked) is recovered: the lock guards no data, so the next writer
    /// proceeds.
    fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        let _guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f()
    }
}

/// A counting gate that bounds how many connections are served concurrently.
/// The accept loop acquires a permit before spawning a connection thread and
/// blocks once `capacity` permits are out, so a connection flood cannot spawn
/// unbounded threads/file descriptors. Each connection releases its permit on
/// teardown (including detached-worker teardown), so capacity is reclaimed as
/// connections finish without retaining their `JoinHandle`s.
#[derive(Debug)]
pub(crate) struct ConnectionGate {
    capacity: usize,
    state: Mutex<usize>,
    released: Condvar,
}

impl ConnectionGate {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            state: Mutex::new(0),
            released: Condvar::new(),
        })
    }

    /// Block until a permit is free, then claim it. Returned guard releases the
    /// permit on drop, so a panicking connection thread still frees its slot.
    pub(crate) fn acquire(self: &Arc<Self>) -> ConnectionPermit {
        let mut live = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while *live >= self.capacity {
            live = self.released.wait(live).unwrap_or_else(|p| p.into_inner());
        }
        *live += 1;
        ConnectionPermit {
            gate: Arc::clone(self),
        }
    }

    /// The number of permits currently out (live connections). Used by the
    /// deterministic connection-cap test to assert the gate enforces its bound.
    #[cfg(test)]
    pub(crate) fn live_count(&self) -> usize {
        *self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// RAII permit: releasing it (on drop) decrements the live-connection count and
/// wakes the accept loop if it was blocked at the ceiling.
pub(crate) struct ConnectionPermit {
    gate: Arc<ConnectionGate>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut live = self.gate.state.lock().unwrap_or_else(|p| p.into_inner());
        *live = live.saturating_sub(1);
        self.gate.released.notify_one();
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

    /// Open a live event tail (ST4/ST11) for a `Subscribe` request: the catch-up
    /// backlog plus a live [`EventStream`] the connection pumps as server-initiated
    /// `event` notifications. Returning `Some` puts the connection into tailing
    /// mode after the `Subscribed` response; returning `None` (the default) means
    /// this handler does not tail, and `Subscribe` is served as a one-shot backlog
    /// response via [`Self::handle`]. The production [`CapoServerHandler`] overrides
    /// it to delegate to [`CapoServer::subscribe`].
    fn subscribe(
        &self,
        _session_id: Option<String>,
        _from_sequence: i64,
    ) -> TransportResult<(crate::SubscriptionBacklog, crate::EventStream)> {
        Err(TransportError::Protocol(
            "this handler does not support live subscription".to_string(),
        ))
    }

    /// Whether [`Self::subscribe`] yields a live tail (so the connection enters
    /// tailing mode after a `Subscribe`). The default handler does not tail; the
    /// production [`CapoServerHandler`] does. Kept separate from `subscribe` so the
    /// connection loop can decide to tail without opening (and then discarding) a
    /// broadcast subscription for a non-tailing handler.
    fn supports_subscription(&self) -> bool {
        false
    }

    /// Handle a typed mid-turn `Interrupt` (ST6) for `session_id`: record the
    /// turn-aborted event for the session so the thread projection renders it.
    /// The transport calls this when an in-band `interrupt` frame matches the
    /// in-flight request, in addition to signaling that request's
    /// [`CancellationToken`] (which drives the running turn's runtime
    /// process-group kill). The default is a no-op so a handler that does not
    /// own session abort semantics is unaffected; the production
    /// [`CapoServerHandler`] overrides it to drive
    /// [`CapoServer::interrupt_session`]. An error is logged-and-dropped: a
    /// failed abort record must not poison the connection, which still emits the
    /// typed `interrupted` frame and stays open.
    fn interrupt(&self, _session_id: &str, _reason: &str) {}
}

/// The production handler: each request runs through the shared
/// [`CapoServer::handle`]. The connection loop wraps this call in the transport's
/// [`WriteSerializer`] for write-bearing commands, so writes are a real
/// single-writer point rather than an assumed one (reads run unlocked). Beyond
/// the queued-cancel short-circuit below it does not poll the token mid-call
/// (handler calls are short), but the seam is in place for cooperative
/// cancellation of longer work.
struct CapoServerHandler {
    server: CapoServer,
}

impl RequestHandler for CapoServerHandler {
    fn handle(
        &self,
        request: ServerRequest,
        cancel: &CancellationToken,
    ) -> TransportResult<ServerResponse> {
        // Cooperative cancellation, the production path the ST3 review asked for.
        // A request can sit queued behind the write lock while the client
        // cancels it in-band; observing the token here means a request cancelled
        // before its write begins short-circuits instead of writing into the
        // store. The connection loop still discards this result by generation,
        // so the client only ever sees the `cancelled` frame.
        if cancel.is_cancelled() {
            return Err(TransportError::Cancelled {
                request_id: request.request_id,
            });
        }
        self.server.handle(request).map_err(TransportError::Server)
    }

    fn subscribe(
        &self,
        session_id: Option<String>,
        from_sequence: i64,
    ) -> TransportResult<(crate::SubscriptionBacklog, crate::EventStream)> {
        self.server
            .subscribe(session_id, from_sequence)
            .map_err(TransportError::Server)
    }

    fn supports_subscription(&self) -> bool {
        true
    }

    fn interrupt(&self, session_id: &str, reason: &str) {
        // Record the typed turn-aborted event through the single
        // `CapoServer::interrupt_session` serialization point. A failure (e.g. an
        // unknown/already-ended session) is dropped rather than propagated: the
        // connection must stay open and still emit the typed `interrupted` frame.
        if let Err(error) = self.server.interrupt_session(session_id, reason) {
            // The transport has no structured logger here; surface to stderr so
            // an interrupt that could not be recorded is observable without
            // crashing the connection.
            eprintln!("capo-server: interrupt for session {session_id} failed: {error:?}");
        }
    }
}

/// Default ceiling on concurrently-served connections. The accept loop blocks
/// (rather than spawning) once this many connections are live, bounding thread
/// and file-descriptor use against a loopback connection-flood. It is generous
/// for interactive use yet finite, so a misbehaving local client cannot exhaust
/// the daemon's resources.
const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 256;

/// Per-connection serving configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ServeConfig {
    /// Idle timeout: a connection idle (no bytes) for this long is closed.
    idle_timeout: Duration,
    /// Ceiling on connections served at once. The accept loop blocks once this
    /// many are live so a connection flood cannot spawn unbounded threads.
    max_concurrent_connections: usize,
    /// How long a live event-tail pump (ST4/ST11) blocks waiting for the next
    /// committed event before waking to re-check its stop flag. It bounds how
    /// long a `Subscribe` tail lingers after its connection closes; it is not a
    /// delivery latency floor (a committed event wakes the pump immediately).
    tail_poll_interval: Duration,
}

impl ServeConfig {
    /// Build a config with an explicit idle timeout (used by the deterministic
    /// idle-timeout test to keep the assertion fast).
    #[cfg(test)]
    pub(crate) fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self {
            idle_timeout,
            ..Self::default()
        }
    }

    /// Build a config with an explicit concurrent-connection ceiling (used by
    /// the deterministic connection-cap test).
    #[cfg(test)]
    pub(crate) fn with_max_concurrent_connections(max_concurrent_connections: usize) -> Self {
        Self {
            max_concurrent_connections,
            ..Self::default()
        }
    }
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_concurrent_connections: DEFAULT_MAX_CONCURRENT_CONNECTIONS,
            tail_poll_interval: DEFAULT_TAIL_POLL_INTERVAL,
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

/// The JSON-RPC method name for a live event-tail notification (ST4). The
/// server pushes one of these per newly-committed event to a subscribed client
/// on the persistent connection; `params` is the same event shape the
/// `subscribed` catch-up backlog carries, so a client decodes a backlog event
/// and a live event identically.
pub const EVENT_TAIL_METHOD: &str = "event";

impl EventNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }

    /// Build the live event-tail notification frame for a committed event (ST4).
    /// The frame is `{"jsonrpc":"2.0","method":"event","params":{"event":{...}}}`
    /// where the inner object is the same shape as a `subscribed` backlog entry.
    pub fn for_event(event: &crate::ServerEvent) -> Self {
        Self {
            method: EVENT_TAIL_METHOD.to_string(),
            params: serde_json::json!({ "event": codec::encode_event(event) }),
        }
    }

    /// Decode the [`crate::ServerEvent`] carried by a live event-tail
    /// notification frame, the inverse of [`Self::for_event`].
    pub fn decode_event(&self) -> TransportResult<crate::ServerEvent> {
        if self.method != EVENT_TAIL_METHOD {
            return Err(TransportError::Protocol(format!(
                "not an event-tail notification: method={}",
                self.method
            )));
        }
        let event = self
            .params
            .get("event")
            .ok_or_else(|| TransportError::Protocol("missing params.event".to_string()))?;
        codec::decode_event(event)
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

/// Round-trip a [`ServerRequest`] through the JSON-RPC 2.0 request codec
/// (encode then decode), for deterministic wire-shape tests. Exercises the same
/// encode/decode path `send_tcp` and the connection read loop use.
#[cfg(test)]
pub(crate) fn jsonrpc_request_roundtrip(request: &ServerRequest) -> ServerRequest {
    jsonrpc::decode_request(&jsonrpc::encode_request(request)).expect("request round-trips")
}

/// Round-trip a [`ServerResponse`] through the JSON-RPC 2.0 success-response
/// codec (encode then decode), for deterministic wire-shape tests.
#[cfg(test)]
pub(crate) fn jsonrpc_response_roundtrip(response: &ServerResponse) -> ServerResponse {
    jsonrpc::decode_response(&jsonrpc::encode_success_response(response))
        .expect("response round-trips")
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
/// argument: it bounds how many connections are *accepted* before the loop
/// returns, which is how the deterministic round-trip tests size the server.
/// That is orthogonal to [`ServeConfig::max_concurrent_connections`], which
/// bounds how many connections are *live at once* (the accept loop blocks at
/// that ceiling so a flood cannot spawn unbounded threads).
///
/// In the bounded mode (`Some(n)`, the tests' sizing) the connection threads
/// are joined before returning, so callers observe a fully-drained server and
/// the returned count is exact. In the production unbounded mode (`None`) the
/// connection threads are detached: their `JoinHandle`s are *not* retained, so
/// a long-running daemon servicing churned connections does not accumulate
/// handles for the process lifetime. Liveness is tracked by the connection gate
/// (via [`ConnectionPermit`]) instead, so resource use stays bounded without a
/// growing handle vector.
pub(crate) fn serve_tcp_with_handler<H: RequestHandler>(
    listener: TcpListener,
    handler: Arc<H>,
    max_connections: Option<usize>,
    config: ServeConfig,
) -> TransportResult<usize> {
    serve_tcp_with_handler_and_grant(listener, handler, max_connections, config, None)
}

/// DT5 conditional-bind entry point: like [`serve_tcp_with_handler`] but accepts an
/// optional [`capo_runtime::ExposureBindGrant`]. With `None` (the all-local
/// default) the bind is loopback-only (hard rejection of a non-loopback address,
/// byte-for-byte the prior behavior). With an ACTIVE grant a non-loopback bind is
/// permitted under the recorded, grant-backed exposure (review finding 12).
pub(crate) fn serve_tcp_with_handler_and_grant<H: RequestHandler>(
    listener: TcpListener,
    handler: Arc<H>,
    max_connections: Option<usize>,
    config: ServeConfig,
    bind_grant: Option<capo_runtime::ExposureBindGrant>,
) -> TransportResult<usize> {
    let bound_address = listener.local_addr().map_err(TransportError::Io)?;
    // CT1/DT5: the bind side consults the connectivity policy rather than a
    // hand-rolled loopback check. With no grant (`config.bind_grant == None`, the
    // all-local default) this is the loopback-only policy's HARD rejection,
    // byte-for-byte the prior behavior — loopback passes, a non-loopback bind fails
    // closed. DT5's conditional bind (review finding 12) supplies an ACTIVE
    // `ExposureBindGrant` so a non-loopback bind is permitted ONLY under a recorded,
    // grant-backed exposure (promoted ceiling + `auth_ref` handle). The listener and
    // connect sides share one policy so loosening one side cannot open an
    // asymmetric hole.
    capo_runtime::authorize_server_bind(bound_address.ip().is_loopback(), bind_grant.as_ref())
        .map_err(|error| {
            TransportError::Protocol(format!(
                "server listener must be loopback, got {bound_address}: {error}"
            ))
        })?;
    // One process-wide write lock shared by every connection: it makes the
    // handler the real single-writer serialization point the module doc claims.
    let write_serializer = WriteSerializer::default();
    let gate = ConnectionGate::new(config.max_concurrent_connections);
    let mut accepted = 0;
    let mut connection_threads = Vec::new();
    while max_connections.map(|max| accepted < max).unwrap_or(true) {
        // Block here once the live-connection ceiling is reached, so the accept
        // loop never spawns more than `max_concurrent_connections` threads.
        let permit = gate.acquire();
        let (stream, _) = listener.accept().map_err(TransportError::Io)?;
        accepted += 1;
        let handler = Arc::clone(&handler);
        let write_serializer = write_serializer.clone();
        let connection_thread = thread::spawn(move || {
            // The permit is moved in and dropped when this thread returns, so it
            // releases its slot back to the gate (even on a panic), reclaiming
            // capacity without the accept loop retaining the handle.
            let _permit = permit;
            // A per-connection error (a peer that dropped, a malformed frame
            // mid-stream, an idle timeout) tears down only that connection; it
            // never poisons the accept loop or sibling connections.
            let _ = handle_connection(handler, stream, &write_serializer, config);
        });
        if max_connections.is_some() {
            // Bounded mode: retain handles so we can drain and return an exact
            // count. The bound is small (test sizing), so no unbounded growth.
            connection_threads.push(connection_thread);
        }
        // Unbounded (production) mode: `connection_thread` is dropped here,
        // detaching it. We never retain its handle, so no per-connection
        // memory accrues for the process lifetime.
    }
    for connection_thread in connection_threads {
        connection_thread
            .join()
            .map_err(|_| TransportError::Protocol("connection thread panicked".to_string()))?;
    }
    Ok(accepted)
}

/// Build the typed mid-turn `interrupt` notification wire frame (ST6).
///
/// Shape: `{"jsonrpc":"2.0","method":"interrupt","params":{"session_id":..,"reason":..}}`.
/// It carries no `id` (it is a notification, not a request), so it is the
/// server-to-nothing half a client pushes on an open connection to abort the
/// live turn for a session, distinct from `cancel` (request-id-scoped) and from
/// the coarse `StopAgent` domain command. The CLI Ctrl-C handler sends this
/// frame on the connection rather than killing the client process.
pub fn interrupt_frame(session_id: &str, reason: &str) -> String {
    EventNotification::new(
        INTERRUPT_METHOD,
        serde_json::json!({ "session_id": session_id, "reason": reason }),
    )
    .to_wire_frame()
}

/// Send a typed mid-turn `interrupt` (ST6) for `session_id` over a fresh
/// loopback connection. This is the smallest client seam the CLI Ctrl-C path
/// uses: it opens the connection, writes the `interrupt` notification frame, and
/// returns. The server aborts the matching in-flight turn (signaling its
/// cancellation token to reap the runtime process group) and records the typed
/// turn-aborted event for the session. Because an interrupt is a notification,
/// no response frame is owed, so this does not read one back.
pub fn send_interrupt(
    address: impl ToSocketAddrs,
    session_id: &str,
    reason: &str,
) -> TransportResult<()> {
    let mut stream = connect_loopback(address)?;
    let frame = interrupt_frame(session_id, reason);
    stream
        .write_all(frame.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)
}

/// Resolve and connect to a loopback-only address, enforcing the same
/// loopback constraint [`send_tcp`] does.
fn connect_loopback(address: impl ToSocketAddrs) -> TransportResult<TcpStream> {
    let resolved = address
        .to_socket_addrs()
        .map_err(TransportError::Io)?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(TransportError::Protocol(
            "server address resolved to no endpoints".to_string(),
        ));
    }
    // CT1: the connect side consults the SAME `ExposurePolicy` as the listener
    // guard (symmetric: loosening only one side is an asymmetric hole). Under the
    // default loopback-only policy every resolved address must be loopback; a
    // non-loopback connect fails closed with the same fail-closed semantics. A
    // non-loopback connect requires an explicitly promoted policy + auth_ref
    // handle (CT2), threaded by the client build path.
    let policy = ExposurePolicy::loopback_default();
    let all_loopback = resolved.iter().all(|address| address.ip().is_loopback());
    policy
        .authorize_socket(all_loopback, None)
        .map_err(|error| {
            TransportError::Protocol(format!(
                "server connect address must resolve only to loopback addresses, got {resolved:?}: {error}"
            ))
        })?;
    TcpStream::connect(resolved.as_slice()).map_err(TransportError::Io)
}

pub fn send_tcp(
    address: impl ToSocketAddrs,
    request: &ServerRequest,
) -> TransportResult<ServerResponse> {
    let mut stream = connect_loopback(address)?;
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

/// Open a live event-tail subscription over a persistent loopback connection
/// (ST4/ST11), the client half of the server's `Subscribe` tail.
///
/// This is the incremental client seam the CLI uses instead of snapshot-polling:
/// it opens one connection, sends a `Subscribe { session_id, from_sequence }`,
/// reads the catch-up [`SubscriptionBacklog`] as the typed response, and returns
/// a [`SubscribeStream`] that yields each subsequent committed event from the
/// live `event` notifications on the *same* connection. There is no gap and no
/// duplicate at the backlog-to-live seam (the server subscribes to the broadcast
/// before snapshotting the backlog, and the live tail resumes strictly after the
/// backlog watermark).
pub fn subscribe_tcp(
    address: impl ToSocketAddrs,
    session_id: Option<String>,
    from_sequence: i64,
) -> TransportResult<(crate::SubscriptionBacklog, SubscribeStream)> {
    let stream = connect_loopback(address)?;
    let request = ServerRequest::cli(ServerCommand::Subscribe {
        session_id,
        from_sequence,
    });
    let mut write_half = stream.try_clone().map_err(TransportError::Io)?;
    let request_json = jsonrpc::encode_request(&request);
    write_half
        .write_all(request_json.as_bytes())
        .and_then(|_| write_half.write_all(b"\n"))
        .and_then(|_| write_half.flush())
        .map_err(TransportError::Io)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(TransportError::Io)?;
    let response = jsonrpc::decode_response(&line)?;
    let ServerResponsePayload::Subscribed(backlog) = response.payload else {
        return Err(TransportError::Protocol(
            "subscribe did not return a Subscribed backlog".to_string(),
        ));
    };
    Ok((
        backlog,
        SubscribeStream {
            reader,
            _write_half: write_half,
        },
    ))
}

/// The client side of a live event tail (ST4/ST11): a persistent connection
/// reading the server's live `event` notification frames after the catch-up
/// backlog. Each [`Self::next_event`] reads one committed event in sequence
/// order. Dropping the stream closes the connection, which stops the server's
/// tail pump.
pub struct SubscribeStream {
    reader: BufReader<TcpStream>,
    // Held so the write half stays open for the connection's lifetime; the client
    // could later use it to send a `cancel`/`interrupt` on the same connection.
    _write_half: TcpStream,
}

impl SubscribeStream {
    /// Read the next committed event off the live tail, blocking until one
    /// arrives. Returns `Ok(None)` when the connection closed cleanly (the server
    /// tail ended), and an error for a malformed frame or an I/O failure. A read
    /// timeout can be installed on the underlying stream via
    /// [`Self::set_read_timeout`] so a caller can poll with a deadline rather than
    /// block forever.
    pub fn next_event(&mut self) -> TransportResult<Option<crate::ServerEvent>> {
        Ok(self.next_event_frame()?.map(|(_, event)| event))
    }

    /// Like [`Self::next_event`] but also returns the verbatim wire frame line
    /// (newline stripped) the server emitted, so a deterministic snapshot test can
    /// assert the exact JSON-RPC notification bytes, not a client re-encoding.
    pub fn next_event_frame(&mut self) -> TransportResult<Option<(String, crate::ServerEvent)>> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(TransportError::Io)?;
        if read == 0 {
            // Clean EOF: the server closed the tail.
            return Ok(None);
        }
        let frame = line.trim_end().to_string();
        let event = EventNotification::from_wire_frame(&frame)
            .and_then(|notification| notification.decode_event())?;
        Ok(Some((frame, event)))
    }

    /// Install a read timeout on the underlying connection so [`Self::next_event`]
    /// returns a `WouldBlock`/`TimedOut` I/O error instead of blocking forever
    /// when no live event arrives. A deterministic tail test uses this to bound
    /// its wait.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> TransportResult<()> {
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(TransportError::Io)
    }
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
    /// An in-flight turn was aborted by a typed mid-turn `Interrupt` (ST6),
    /// naming the session and reason. Distinct from [`Self::Cancelled`] (a
    /// request-id abort): the connection stays open and the server records a
    /// turn-aborted event for the session.
    Interrupted {
        session_id: String,
        reason: String,
    },
}

/// One framed line read off a persistent connection, classified by intent.
enum Frame {
    /// A JSON-RPC request to dispatch.
    Request(Box<ServerRequest>),
    /// An in-band `cancel` notification naming the `request_id` to abort.
    Cancel { request_id: Option<String> },
    /// A typed mid-turn `interrupt` notification (ST6) naming the `session_id`
    /// to abort and a `reason`. Distinct from `Cancel` and from `StopAgent`.
    Interrupt {
        session_id: Option<String>,
        reason: String,
    },
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
/// side runs on its own (detached) thread and feeds [`ConnEvent`]s to the main
/// loop, so an in-band `cancel` notification can abort the matching in-flight
/// request without dropping the connection, and a stalled peer is reaped by the
/// idle timeout.
///
/// Both the read side and each request worker run on *detached* threads rather
/// than scoped ones: that is deliberate. A scoped worker would pin this
/// function (and thus the connection thread and server drain) until a slow,
/// non-cooperative handler returned, defeating the resource bound -- the read
/// loop could mark a request cancelled or the connection closed, yet still be
/// stuck joining the worker. Detaching lets the read loop return the instant the
/// connection closes or a request is cancelled; an orphaned worker drains on its
/// own. It still holds the [`WriteSerializer`] while it runs, so it cannot race
/// a second writer into the store; it just delays the next write until it ends.
fn handle_connection<H: RequestHandler>(
    handler: Arc<H>,
    stream: TcpStream,
    write_serializer: &WriteSerializer,
    config: ServeConfig,
) -> TransportResult<()> {
    // The idle timeout is enforced as a read timeout: a read that blocks longer
    // than this returns `WouldBlock`/`TimedOut`, which we fold into `Closed` so
    // a stalled or abandoned client cannot hold a connection thread.
    stream
        .set_read_timeout(Some(config.idle_timeout))
        .map_err(TransportError::Io)?;
    // The write half is shared between this loop and the live event-tail pump (a
    // `Subscribe` puts the connection into tailing mode, ST4/ST11). A `Mutex`
    // serializes the two writers so a tail `event` notification can never
    // interleave bytes with a response frame on the same socket.
    let write_half = Arc::new(Mutex::new(stream.try_clone().map_err(TransportError::Io)?));
    let read_half = stream;

    let (event_tx, event_rx) = mpsc::channel::<ConnEvent>();

    // Read side: classify frames and forward them; a clean EOF, idle timeout, or
    // hard error all terminate the connection via `Closed`. Detached: when this
    // function returns and drops `event_rx`, the reader's next `send` fails and
    // it exits on its own, so we never block the connection on it.
    let reader_tx = event_tx.clone();
    thread::spawn(move || {
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
    // At most one live event tail per connection. It is started when a
    // `Subscribe` request is served and stopped when the connection closes (or a
    // new request supersedes it), so a tailing connection never leaks its pump.
    let mut tail: Option<TailHandle> = None;
    let mut generation: u64 = 0;
    while let Ok(event) = event_rx.recv() {
        match event {
            ConnEvent::Closed => {
                if let Some((_, cancel, _)) = &in_flight {
                    cancel.cancel();
                }
                // Stopping the tail (its `Drop`) flips the pump's stop flag so the
                // detached pump thread exits on its next poll wakeup.
                drop(tail);
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
                    write_shared_frame(
                        &write_half,
                        &jsonrpc::encode_error_response(Some(&request.request_id), &error),
                    )?;
                    continue;
                }
                // A `Subscribe` opens a live event tail (ST4/ST11): the connection
                // serves the catch-up backlog as the response, then pumps live
                // `event` notifications on the same socket. A fresh `Subscribe`
                // supersedes any prior tail on this connection.
                if let ServerCommand::Subscribe {
                    session_id,
                    from_sequence,
                } = &request.command
                    && handler.supports_subscription()
                {
                    drop(tail.take());
                    match start_event_tail(
                        &handler,
                        &write_half,
                        &request,
                        session_id.clone(),
                        *from_sequence,
                        config,
                    ) {
                        Ok(handle) => tail = Some(handle),
                        Err(error) => {
                            // A subscribe that could not open its tail gets a typed
                            // error frame and the connection stays open.
                            write_shared_frame(
                                &write_half,
                                &jsonrpc::encode_error_response(Some(&request.request_id), &error),
                            )?;
                        }
                    }
                    continue;
                }
                generation += 1;
                let this_generation = generation;
                let request_id = request.request_id.clone();
                let cancel = CancellationToken::new();
                let worker_cancel = cancel.clone();
                let worker_tx = event_tx.clone();
                let worker_handler = Arc::clone(&handler);
                let worker_serializer = write_serializer.clone();
                // Serialize write-bearing commands behind the process-wide write
                // lock so only one writer runs at a time across every connection
                // (the documented single-writer point). Read-only commands skip
                // the lock so they can be served concurrently -- the concurrency
                // ST3 promises is real for reads, while writes never interleave.
                let serialize_writes = !request.command.is_read_only();
                // Detached worker: if the request is cancelled or the connection
                // closes first, this worker is orphaned and drains on its own;
                // its `Result` send below fails harmlessly once `event_rx` is
                // gone. A write-bearing worker holds the write lock while it
                // runs, so even orphaned it cannot race a second writer.
                thread::spawn(move || {
                    let dispatch = || worker_handler.handle(*request, &worker_cancel);
                    let result = if serialize_writes {
                        worker_serializer.run(dispatch)
                    } else {
                        dispatch()
                    };
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
                        write_shared_frame(&write_half, &frame)?;
                        in_flight = None;
                    }
                } else if tail.is_some() {
                    // A cancel against a live event tail (no request in flight)
                    // stops the tail and emits the typed `cancelled` frame so the
                    // client knows the tail ended; the connection stays open.
                    drop(tail.take());
                    let error = TransportError::Cancelled {
                        request_id: target.unwrap_or_default(),
                    };
                    write_shared_frame(&write_half, &jsonrpc::encode_error_response(None, &error))?;
                }
                // A cancel with nothing matching in flight is a no-op
                // notification (no response frame is owed).
            }
            ConnEvent::Incoming(Frame::Interrupt { session_id, reason }) => {
                // Typed mid-turn interrupt (ST6): abort the in-flight turn on
                // this connection. A connection serves one turn at a time, so
                // the interrupt targets whatever is in flight; the `session_id`
                // (when present) names the session the turn-aborted event is
                // recorded against.
                if let Some((request_id, cancel, _)) = &in_flight {
                    // 1) Signal the worker as INTERRUPTED with the reason so the
                    //    running turn cooperatively reaps its runtime process
                    //    group (and can label the abort with the reason).
                    cancel.interrupt(&reason);
                    // 2) Record the typed turn-aborted event for the session so
                    //    the thread projection renders it. A `None` session id
                    //    only signals the in-flight worker (the handler has no
                    //    session to record against).
                    if let Some(session_id) = session_id.as_deref() {
                        handler.interrupt(session_id, &reason);
                    }
                    // 3) Emit the typed `interrupted` frame and keep the
                    //    connection open; the worker's eventual `Result` is
                    //    discarded by the generation check below.
                    let error = TransportError::Interrupted {
                        session_id: session_id.unwrap_or_default(),
                        reason,
                    };
                    let frame = jsonrpc::encode_error_response(Some(request_id), &error);
                    write_shared_frame(&write_half, &frame)?;
                    in_flight = None;
                } else if let Some(session_id) = session_id.as_deref() {
                    // No request in flight (e.g. while tailing): still record the
                    // typed turn-aborted event so the thread projection renders the
                    // interrupt. This is the connection a CLI Ctrl-C lands on when
                    // the live turn runs under a separate connection but the tail is
                    // here.
                    handler.interrupt(session_id, &reason);
                }
                // An interrupt with nothing in flight is otherwise a no-op.
            }
            ConnEvent::Incoming(Frame::Invalid { id, error }) => {
                write_shared_frame(
                    &write_half,
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
                write_shared_frame(&write_half, &frame)?;
            }
        }
    }
    drop(tail);
    Ok(())
}

/// A running live event tail on a connection (ST4/ST11). Holds the stop flag the
/// detached pump thread polls; dropping the handle flips the flag so the pump
/// exits on its next poll wakeup (it never has to be joined, matching the
/// detached-worker discipline of the rest of the connection loop).
struct TailHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for TailHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Open a live event tail for a `Subscribe` (ST4/ST11): write the catch-up
/// `Subscribed` backlog as the response, then spawn a detached pump that streams
/// live `event` notifications on the same socket until the connection closes.
///
/// The backlog snapshot and the live `EventStream` come from a single
/// `RequestHandler::subscribe`, which subscribes to the broadcast *before*
/// reading the backlog -- so the tail is gap-free and duplicate-free at the
/// backlog-to-live seam (the same guarantee `CapoServer::subscribe` documents).
/// The pump polls the stream with a bounded timeout so it wakes to notice the
/// stop flag promptly even when no events arrive.
fn start_event_tail<H: RequestHandler>(
    handler: &Arc<H>,
    write_half: &Arc<Mutex<TcpStream>>,
    request: &ServerRequest,
    session_id: Option<String>,
    from_sequence: i64,
    config: ServeConfig,
) -> TransportResult<TailHandle> {
    let (backlog, mut stream) = handler.subscribe(session_id, from_sequence)?;
    // The catch-up backlog is the response to the `Subscribe` request itself, so a
    // client reads it exactly like any other typed response before the live frames.
    let response = ServerResponse {
        request_id: request.request_id.clone(),
        client_id: request.origin.client_id.clone(),
        actor_id: request.origin.actor_id.clone(),
        input_origin: request.origin.input_origin,
        payload: ServerResponsePayload::Subscribed(backlog),
    };
    write_shared_frame(write_half, &jsonrpc::encode_success_response(&response))?;

    let stop = Arc::new(AtomicBool::new(false));
    let pump_stop = Arc::clone(&stop);
    let pump_writer = Arc::clone(write_half);
    let poll_interval = config.tail_poll_interval;
    thread::spawn(move || {
        while !pump_stop.load(Ordering::SeqCst) {
            // Block (bounded) on the next committed event so the pump is event-
            // driven, not a busy spin, yet still wakes to re-check the stop flag.
            match stream.recv_batch(poll_interval) {
                Ok(events) => {
                    for event in events {
                        let frame = EventNotification::for_event(&event).to_wire_frame();
                        if write_shared_frame(&pump_writer, &frame).is_err() {
                            // The socket is gone (peer closed); end the pump.
                            return;
                        }
                    }
                }
                // A poll with no event is normal: loop to re-check the stop flag.
                Err(TailRecvError::Timeout) => {}
                // The broadcast hub was torn down (server shutting down): end.
                Err(TailRecvError::Disconnected) => return,
            }
        }
    });
    Ok(TailHandle { stop })
}

/// Write a framed line to a connection's shared write half (used by both the main
/// loop and the live event-tail pump). Locks for the duration of one frame so the
/// two writers never interleave bytes.
fn write_shared_frame(write_half: &Arc<Mutex<TcpStream>>, frame: &str) -> TransportResult<()> {
    let mut stream = write_half
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_frame(&mut stream, frame)
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
    if is_notification && method == Some(INTERRUPT_METHOD) {
        let params = value.get("params");
        let session_id = params
            .and_then(|params| params.get("session_id"))
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let reason = params
            .and_then(|params| params.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("interrupt requested")
            .to_string();
        return Ok(Frame::Interrupt { session_id, reason });
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
