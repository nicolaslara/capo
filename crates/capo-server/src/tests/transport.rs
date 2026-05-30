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
use std::sync::{Arc, Barrier};
use std::time::Duration;

use crate::AgentSummary;
use crate::transport::{
    CancellationToken, RequestHandler, ServeConfig, TransportError, serve_tcp_with_handler,
};

/// A scripted handler driving the ST3 transport tests deterministically (no
/// live provider, no real turn). Behavior is selected by the request id:
///
/// - `barrier-*`: wait on a shared [`Barrier`] before replying. If the accept
///   loop were serial, two such requests on two connections could never both
///   reach the barrier, so reaching it proves genuine concurrency.
/// - `block-*`: spin until the in-band cancel token fires, then return. This
///   holds a request in flight so the test can cancel it.
/// - anything else: reply immediately.
///
/// Every reply echoes the request id back as a single-agent `Agents` payload so
/// the client can assert each connection received *its own* response.
struct ScriptedHandler {
    barrier: Arc<Barrier>,
}

impl RequestHandler for ScriptedHandler {
    fn handle(
        &self,
        request: ServerRequest,
        cancel: &CancellationToken,
    ) -> Result<ServerResponse, TransportError> {
        if request.request_id.starts_with("barrier-") {
            self.barrier.wait();
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
    let handler = Arc::new(ScriptedHandler {
        barrier: Arc::clone(&barrier),
    });
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
    let handler = Arc::new(ScriptedHandler {
        barrier: Arc::new(Barrier::new(1)),
    });
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
    let handler = Arc::new(ScriptedHandler {
        barrier: Arc::new(Barrier::new(1)),
    });
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
