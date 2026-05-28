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
    assert!(response.contains("\"ok\":false"));
    assert!(response.contains("request frame is too large"));
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
