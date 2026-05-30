use super::*;

#[test]
fn tcp_transport_rejects_oversized_frames_before_json_decode() {
    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(1),
        )
        .expect("serve tcp")
    });

    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .write_all(&vec![b'x'; 400 * 1024])
        .and_then(|_| stream.write_all(b"\n"))
        .expect("write oversized frame");
    stream.shutdown(Shutdown::Write).expect("shutdown write");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    // The oversized frame is rejected before JSON decode, so the server
    // replies with a JSON-RPC 2.0 error frame (no recoverable id) carrying the
    // bounded-frame protocol failure, not a success `result`.
    let frame: serde_json::Value =
        serde_json::from_str(response.trim_end()).expect("oversized-frame response is JSON-RPC");
    assert_eq!(
        frame.get("jsonrpc").and_then(serde_json::Value::as_str),
        Some("2.0")
    );
    assert!(
        frame.get("result").is_none(),
        "expected an error frame, got: {response}"
    );
    assert_eq!(frame.get("id"), Some(&serde_json::Value::Null));
    let error = frame.get("error").expect("error member");
    assert_eq!(
        error
            .get("data")
            .and_then(|data| data.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("protocol"),
    );
    assert!(
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("request frame is too large")),
        "unexpected error message: {response}"
    );
    assert_eq!(server_thread.join().expect("server thread"), 1);
}

#[test]
fn tcp_transport_rejects_non_loopback_listener_at_server_boundary() {
    let root = temp_root();
    let listener = TcpListener::bind("0.0.0.0:0").expect("listener");
    let error = serve_tcp(listener, ProjectId::new("project-capo"), &root, Some(1))
        .expect_err("server transport must reject non-loopback listener");
    assert!(
        format!("{error:?}").contains("loopback"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn tcp_transport_rejects_non_loopback_connect_at_server_boundary() {
    let request = ServerRequest::local_cli("connect-public", ServerCommand::ListAgents);
    let error = send_tcp("0.0.0.0:1", &request)
        .expect_err("server transport client must reject non-loopback addresses before connect");
    assert!(
        format!("{error:?}").contains("loopback"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn tcp_transport_round_trips_server_requests_and_recovers_state() {
    let root = temp_root();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server_root = root.clone();
    let server_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            server_root,
            Some(3),
        )
        .expect("serve tcp")
    });

    let registered = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-register-mock-codex",
            ServerCommand::RegisterAgent {
                name: "mock-codex".to_string(),
            },
        ),
    )
    .expect("register over tcp");
    assert_agent_registered(&registered, "mock-codex");

    let sent = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-send-mock-codex",
            ServerCommand::SendTask {
                agent_name: "mock-codex".to_string(),
                goal: "Prove TCP transport".to_string(),
                scenario: "default".to_string(),
            },
        ),
    )
    .expect("send over tcp");
    let ServerResponsePayload::TaskSent(run) = sent.payload else {
        panic!("expected task sent");
    };
    assert_eq!(run.session_id.as_str(), "session-mock-codex");

    let dashboard = send_tcp(
        address,
        &ServerRequest::local_cli(
            "tcp-dashboard",
            ServerCommand::Dashboard {
                recent_event_limit: 8,
            },
        ),
    )
    .expect("dashboard over tcp");
    let ServerResponsePayload::Dashboard(snapshot) = dashboard.payload else {
        panic!("expected dashboard");
    };
    assert_eq!(snapshot.agent_count, 1);
    assert_eq!(snapshot.active_session_count, 1);

    assert_eq!(server_thread.join().expect("server thread"), 3);

    let listener = TcpListener::bind("127.0.0.1:0").expect("restart listener");
    let restart_address = listener.local_addr().expect("restart address");
    let restart_root = root.clone();
    let restart_thread = thread::spawn(move || {
        serve_tcp(
            listener,
            ProjectId::new("project-capo"),
            restart_root,
            Some(2),
        )
        .expect("serve restarted tcp")
    });

    let recovered = send_tcp(
        restart_address,
        &ServerRequest::local_cli("tcp-recover", ServerCommand::Recover),
    )
    .expect("recover after restart over tcp");
    let ServerResponsePayload::Recovery(recovery) = recovered.payload else {
        panic!("expected recovery");
    };
    assert_eq!(recovery.recovered_run_count, 1);

    let restarted_dashboard = send_tcp(
        restart_address,
        &ServerRequest::local_cli(
            "tcp-dashboard-after-restart",
            ServerCommand::Dashboard {
                recent_event_limit: 8,
            },
        ),
    )
    .expect("dashboard after restart over tcp");
    let ServerResponsePayload::Dashboard(snapshot) = restarted_dashboard.payload else {
        panic!("expected restarted dashboard");
    };
    assert_eq!(snapshot.agent_count, 1);
    assert_eq!(snapshot.active_session_count, 0);
    assert_eq!(restart_thread.join().expect("restart thread"), 2);

    let reopened = CapoServer::open(ProjectId::new("project-capo"), root).expect("reopen");
    let dashboard = reopened.dashboard_snapshot().expect("dashboard");
    assert_eq!(dashboard.agent_count, 1);
    assert_eq!(dashboard.active_session_count, 0);
}

// --- ST3: concurrent accept loop, idle timeout, in-band Cancel ---------------

use std::io::{BufRead, BufReader};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::AgentSummary;
use crate::transport::{
    CancellationToken, ConnectionGate, RequestHandler, ServeConfig, TransportError,
    serve_tcp_with_handler,
};

/// A releasable latch a `noncoop-` request waits on while *ignoring* the cancel
/// token, modeling a genuinely non-cooperative (production-shaped) handler. The
/// test releases it at the end so the orphaned detached worker exits cleanly
/// instead of leaking a thread that runs until process exit.
#[derive(Clone, Default)]
struct ReleaseLatch {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ReleaseLatch {
    fn wait(&self) {
        let (lock, cvar) = &*self.inner;
        let mut released = lock.lock().expect("release latch lock");
        while !*released {
            released = cvar.wait(released).expect("release latch wait");
        }
    }

    fn release(&self) {
        let (lock, cvar) = &*self.inner;
        *lock.lock().expect("release latch lock") = true;
        cvar.notify_all();
    }
}

/// A scripted handler driving the ST3 transport tests deterministically (no
/// live provider, no real turn). Behavior is selected by the request id:
///
/// - `barrier-*`: wait on a shared [`Barrier`] before replying. If the accept
///   loop were serial, two such requests on two connections could never both
///   reach the barrier, so reaching it proves genuine concurrency.
/// - `block-*`: spin until the in-band cancel token fires, then return. This
///   holds a request in flight (cooperatively) so the test can cancel it.
/// - `noncoop-*`: block on the [`ReleaseLatch`] *ignoring* the cancel token,
///   modeling a non-cooperative handler. The connection must still be reclaimed
///   (its worker is detached) even though this worker keeps running.
/// - anything else: reply immediately.
///
/// Every reply echoes the request id back as a single-agent `Agents` payload so
/// the client can assert each connection received *its own* response.
struct ScriptedHandler {
    barrier: Arc<Barrier>,
    noncoop_latch: ReleaseLatch,
}

impl ScriptedHandler {
    fn new(barrier: Arc<Barrier>) -> Self {
        Self {
            barrier,
            noncoop_latch: ReleaseLatch::default(),
        }
    }

    /// A clone of the latch a `noncoop-` request blocks on, so a test can
    /// release the orphaned worker after asserting the connection was reclaimed.
    fn noncoop_latch_for_test(&self) -> ReleaseLatch {
        self.noncoop_latch.clone()
    }
}

impl RequestHandler for ScriptedHandler {
    fn handle(
        &self,
        request: ServerRequest,
        cancel: &CancellationToken,
    ) -> Result<ServerResponse, TransportError> {
        if request.request_id.starts_with("barrier-") {
            self.barrier.wait();
        } else if request.request_id.starts_with("noncoop-") {
            // Non-cooperative: block until explicitly released, never consulting
            // the cancel token. A cancel/disconnect must reclaim the connection
            // regardless, because the worker runs detached from it.
            self.noncoop_latch.wait();
        } else if request.request_id.starts_with("block-") {
            // Cooperative cancellation: hold the request in flight until the
            // in-band cancel fires. A short sleep keeps the spin cheap.
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(echo_response(&request))
    }
}

/// Echo the request id back as a single-agent `Agents` payload so two
/// connections can be told apart by their responses.
fn echo_response(request: &ServerRequest) -> ServerResponse {
    ServerResponse {
        request_id: request.request_id.clone(),
        client_id: request.origin.client_id.clone(),
        actor_id: request.origin.actor_id.clone(),
        input_origin: request.origin.input_origin,
        payload: ServerResponsePayload::Agents(vec![AgentSummary {
            agent_id: capo_core::AgentId::new(format!("agent-{}", request.request_id)),
            name: request.request_id.clone(),
            status: "available".to_string(),
            current_session_id: None,
            session: None,
        }]),
    }
}

/// Build a JSON-RPC 2.0 request wire frame for `ListAgents` with the given id.
/// Built by hand (not via the internal codec) so the test also pins the wire
/// shape the server must accept.
fn jsonrpc_list_agents_frame(request_id: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "list_agents",
        "params": {
            "origin": {
                "client_id": "local-cli",
                "actor_id": "local-user",
                "input_origin": "cli",
            }
        }
    })
    .to_string()
}

/// Build an in-band `cancel` notification frame (no `id`) naming the request to
/// abort. It is a JSON-RPC notification, distinct from closing the socket.
fn jsonrpc_cancel_frame(request_id: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "cancel",
        "params": { "request_id": request_id },
    })
    .to_string()
}

fn write_frame_line(stream: &mut TcpStream, frame: &str) {
    stream
        .write_all(frame.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .expect("write frame");
}

/// Read exactly one newline-terminated response frame from a connection, with a
/// read timeout so a hung server fails the test rather than blocking forever.
fn read_frame_line(stream: &TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response line");
    line
}

fn response_agent_name(frame: &str) -> String {
    let value: serde_json::Value =
        serde_json::from_str(frame.trim_end()).expect("response is JSON-RPC");
    value
        .get("result")
        .and_then(|result| result.get("payload"))
        .and_then(|payload| payload.get("agents"))
        .and_then(|agents| agents.get(0))
        .and_then(|agent| agent.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("expected agents payload, got: {frame}"))
        .to_string()
}

#[test]
fn two_concurrent_connections_receive_independent_responses() {
    // The barrier requires BOTH connections' requests to be handled at once. A
    // serial accept loop could never satisfy it, so completing proves the
    // accept loop serves connections concurrently.
    let barrier = Arc::new(Barrier::new(2));
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::clone(&barrier)));
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(listener, handler, Some(2), ServeConfig::default())
            .expect("serve tcp with scripted handler")
    });

    let client = |request_id: &'static str| {
        thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            write_frame_line(&mut stream, &jsonrpc_list_agents_frame(request_id));
            let response = read_frame_line(&stream);
            response_agent_name(&response)
        })
    };

    let first = client("barrier-conn-one");
    let second = client("barrier-conn-two");

    assert_eq!(first.join().expect("first client"), "barrier-conn-one");
    assert_eq!(second.join().expect("second client"), "barrier-conn-two");
    assert_eq!(server_thread.join().expect("server thread"), 2);
}

#[test]
fn idle_connection_is_closed_after_the_read_timeout() {
    // A client that connects but never sends a complete frame must not hold a
    // connection thread forever; the per-connection idle timeout reaps it.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::new(Barrier::new(1))));
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(
            listener,
            handler,
            Some(1),
            ServeConfig::with_idle_timeout(Duration::from_millis(150)),
        )
        .expect("serve tcp with scripted handler")
    });

    // Connect and send nothing (an idle/stalled client). The server must close
    // the connection once the idle timeout elapses; reading then returns EOF.
    let stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    let read = reader.read_to_end(&mut buffer).expect("read to eof");
    assert_eq!(
        read, 0,
        "idle connection should be closed with no bytes sent"
    );
    assert!(buffer.is_empty());

    // The accept loop drained its one connection and returned cleanly.
    assert_eq!(server_thread.join().expect("server thread"), 1);
}

#[test]
fn in_band_cancel_aborts_in_flight_request_without_dropping_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::new(Barrier::new(1))));
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(listener, handler, Some(1), ServeConfig::default())
            .expect("serve tcp with scripted handler")
    });

    let mut stream = TcpStream::connect(address).expect("connect");

    // Send a request that blocks in the handler until cancelled.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("block-in-flight"));
    // Then cancel it in-band on the SAME open connection.
    write_frame_line(&mut stream, &jsonrpc_cancel_frame("block-in-flight"));

    // The cancelled request gets a typed `cancelled` error frame, not a result.
    let cancelled = read_frame_line(&stream);
    let cancelled_value: serde_json::Value =
        serde_json::from_str(cancelled.trim_end()).expect("cancel response is JSON-RPC");
    assert_eq!(
        cancelled_value
            .get("id")
            .and_then(serde_json::Value::as_str),
        Some("block-in-flight"),
    );
    assert!(
        cancelled_value.get("result").is_none(),
        "expected an error frame for the cancelled request, got: {cancelled}"
    );
    assert_eq!(
        cancelled_value
            .get("error")
            .and_then(|error| error.get("data"))
            .and_then(|data| data.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("cancelled"),
        "cancelled request must carry the typed `cancelled` error kind: {cancelled}"
    );

    // The connection is NOT dropped: a follow-up request on it still succeeds.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("after-cancel"));
    let follow_up = read_frame_line(&stream);
    assert_eq!(response_agent_name(&follow_up), "after-cancel");

    // Close the connection so the accept loop drains and returns.
    stream.shutdown(Shutdown::Both).expect("shutdown");
    assert_eq!(server_thread.join().expect("server thread"), 1);
}

#[test]
fn second_request_while_one_is_in_flight_is_rejected_and_connection_recovers() {
    // ST3 admits one request at a time per connection: a second request sent
    // while the first is still in flight is rejected with a `protocol` error and
    // the connection keeps serving. This is the load-bearing single-writer
    // safety branch, exercised here end to end.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::new(Barrier::new(1))));
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(listener, handler, Some(1), ServeConfig::default())
            .expect("serve tcp with scripted handler")
    });

    let mut stream = TcpStream::connect(address).expect("connect");

    // First request blocks in the handler (cooperatively) until cancelled.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("block-first"));
    // A second request arrives while the first is still in flight: it must be
    // rejected, not admitted as a concurrent second writer.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("second-while-busy"));

    let rejected = read_frame_line(&stream);
    let rejected_value: serde_json::Value =
        serde_json::from_str(rejected.trim_end()).expect("rejection is JSON-RPC");
    assert_eq!(
        rejected_value.get("id").and_then(serde_json::Value::as_str),
        Some("second-while-busy"),
        "the rejection must name the second request, not the in-flight one: {rejected}"
    );
    assert!(
        rejected_value.get("result").is_none(),
        "the second request must get an error frame, not a result: {rejected}"
    );
    let error = rejected_value.get("error").expect("error member");
    assert_eq!(
        error
            .get("data")
            .and_then(|data| data.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("protocol"),
        "already-in-flight rejection must be a protocol error: {rejected}"
    );
    assert!(
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("already in flight")),
        "unexpected rejection message: {rejected}"
    );

    // The first request is unaffected: cancel it in-band and observe its typed
    // `cancelled` frame.
    write_frame_line(&mut stream, &jsonrpc_cancel_frame("block-first"));
    let cancelled = read_frame_line(&stream);
    let cancelled_value: serde_json::Value =
        serde_json::from_str(cancelled.trim_end()).expect("cancel response is JSON-RPC");
    assert_eq!(
        cancelled_value
            .get("error")
            .and_then(|error| error.get("data"))
            .and_then(|data| data.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("cancelled"),
        "the in-flight request should still cancel cleanly: {cancelled}"
    );

    // After clearing the in-flight slot the connection serves a third request.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("after-rejection"));
    let third = read_frame_line(&stream);
    assert_eq!(response_agent_name(&third), "after-rejection");

    stream.shutdown(Shutdown::Both).expect("shutdown");
    assert_eq!(server_thread.join().expect("server thread"), 1);
}

#[test]
fn stalled_connection_with_noncooperative_in_flight_work_is_reclaimed() {
    // The strong claim is that a stalled/disconnected connection is reclaimed
    // even when its in-flight handler is non-cooperative (ignores the cancel
    // token), because the worker runs detached from the connection thread. If
    // the worker were scoped, the accept loop's drain would block here until the
    // handler returned -- which it never would on its own.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::new(Barrier::new(1))));
    // Keep a handle to the latch so we can release the orphaned worker at the
    // end (so it does not leak a thread running until process exit).
    let latch = handler.noncoop_latch_for_test();
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(listener, handler, Some(1), ServeConfig::default())
            .expect("serve tcp with scripted handler")
    });

    let mut stream = TcpStream::connect(address).expect("connect");
    // Send a non-cooperative request that blocks ignoring the cancel token.
    write_frame_line(&mut stream, &jsonrpc_list_agents_frame("noncoop-stuck"));
    // Give the worker a moment to actually enter the handler and block.
    thread::sleep(Duration::from_millis(50));
    // Abruptly disconnect, simulating a stalled/abandoned client.
    stream.shutdown(Shutdown::Both).expect("shutdown");
    drop(stream);

    // The connection thread (and thus the bounded accept loop's drain) must
    // return promptly despite the still-running non-cooperative worker. We join
    // on a helper thread with a deadline so a regression (scoped worker pinning
    // the connection) fails the test rather than hanging the suite forever.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let served = server_thread.join().expect("server thread");
        let _ = done_tx.send(served);
    });
    let served = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("accept loop must drain a stalled connection with non-cooperative in-flight work");
    assert_eq!(served, 1);

    // Release the orphaned worker so its detached thread exits cleanly.
    latch.release();
}

#[test]
fn accept_loop_honors_the_concurrent_connection_ceiling() {
    // End-to-end DoS bound: with a ceiling of one, a second connection cannot be
    // served until the first is torn down, so a flood cannot spawn unbounded
    // connection threads. We hold the first connection in flight, prove the
    // second gets no response meanwhile, then free the first and watch the
    // second complete.
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let handler = Arc::new(ScriptedHandler::new(Arc::new(Barrier::new(1))));
    let server_thread = thread::spawn(move || {
        serve_tcp_with_handler(
            listener,
            handler,
            Some(2),
            ServeConfig::with_max_concurrent_connections(1),
        )
        .expect("serve tcp with scripted handler")
    });

    // Connection one holds a cooperatively-blocking request in flight, keeping
    // its connection thread (and thus the single permit) alive.
    let mut first = TcpStream::connect(address).expect("connect first");
    write_frame_line(&mut first, &jsonrpc_list_agents_frame("block-hold"));

    // Connection two connects and sends a request, but must NOT be served while
    // the ceiling is saturated by connection one.
    let mut second = TcpStream::connect(address).expect("connect second");
    write_frame_line(&mut second, &jsonrpc_list_agents_frame("waiting-for-slot"));
    second
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("set read timeout");
    let mut probe = [0_u8; 1];
    let blocked = second.read(&mut probe);
    assert!(
        matches!(&blocked, Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
        "second connection must not be served while the ceiling is saturated, got: {blocked:?}"
    );

    // Free connection one: cancel its in-flight request and close it, releasing
    // the permit so the accept loop can finally serve connection two.
    write_frame_line(&mut first, &jsonrpc_cancel_frame("block-hold"));
    let _ = read_frame_line(&first); // drain the `cancelled` frame
    first.shutdown(Shutdown::Both).expect("shutdown first");
    drop(first);

    // Connection two is now served and gets its own response.
    let response = read_frame_line(&second);
    assert_eq!(response_agent_name(&response), "waiting-for-slot");
    second.shutdown(Shutdown::Both).expect("shutdown second");
    assert_eq!(server_thread.join().expect("server thread"), 2);
}

#[test]
fn connection_gate_blocks_at_capacity_and_reclaims_on_release() {
    // The accept loop's DoS bound: the gate must never let more than `capacity`
    // permits out, and must hand out a fresh permit the instant one is released.
    let gate = ConnectionGate::new(2);
    let first = gate.acquire();
    let second = gate.acquire();
    assert_eq!(gate.live_count(), 2, "two permits are out at capacity");

    // A third acquire must block until a permit is released. Prove it blocks by
    // checking it has NOT completed after a grace period, then release one and
    // confirm it completes.
    let gate_for_third = Arc::clone(&gate);
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let third = thread::spawn(move || {
        let permit = gate_for_third.acquire();
        let _ = acquired_tx.send(());
        // Hold the permit until the test drops the channel sender side.
        permit
    });

    assert!(
        acquired_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err(),
        "the gate must block a third acquire while at capacity"
    );

    // Releasing one permit lets the blocked acquire proceed promptly.
    let release_at = Instant::now();
    drop(first);
    acquired_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("releasing a permit must unblock the waiting acquire");
    assert!(
        release_at.elapsed() < Duration::from_secs(2),
        "the waiting acquire should be woken by the release, not by polling"
    );
    assert_eq!(gate.live_count(), 2, "still two permits out after the swap");

    drop(second);
    let third_permit = third.join().expect("third acquire thread");
    drop(third_permit);
    assert_eq!(gate.live_count(), 0, "all permits released");
}
