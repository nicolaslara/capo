//! DT1 deterministic three-process-over-loopback test: a server process, a
//! runner process that ANNOUNCES a runtime target over the JSON-RPC command
//! transport (DT-D1), and a client that tails the event log via `subscribe_tcp`
//! and sees the runner's SERVER-APPENDED `runtime.target_registered` event.
//!
//! This proves the DT-D1 decision end-to-end: the runner is a special client
//! that owns processes and reuses the existing transport (no second bridge); the
//! server -- the single authoritative writer -- owns the append; and a client
//! holding no authoritative state sees that append in its tail. It also pins the
//! up-front role-config validation (a runner/client with no server endpoint is
//! rejected before any socket) and the `blocked_pending_permission` verdict for a
//! non-loopback exposure.

use std::io::BufReader;

use capo_server::subscribe_tcp;

use super::support::*;

#[test]
fn runner_announces_runtime_target_over_jsonrpc_and_client_tail_sees_server_append() {
    // The SERVER process owns the authoritative event log under its OWN state
    // root. The RUNNER subprocess uses a DISTINCT, EMPTY state root: this is what
    // proves the announce travelled over TCP rather than an in-process store
    // write (review finding 2). With the in-process fallback removed (finding 1),
    // the only path by which `runtime.target_registered` can land in the SERVER's
    // log -- the log the client tails -- is the JSON-RPC announce over the socket.
    // The runner's own state root is asserted EMPTY afterward, so no local write
    // could have masqueraded as the announce.
    let server_state = temp_root("role-topology-announce-server-state");
    let runner_state = temp_root("role-topology-announce-runner-state");
    let mut server = spawn_server(&server_state, 1_000);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let runner_state_str = runner_state.display().to_string();

    // The RUNNER role, in its own process, announces over the JSON-RPC transport
    // pointing at the server's loopback control endpoint. The server appends
    // `runtime.target_registered`; the runner gets back the server's sequence.
    let announce = capo([
        "role",
        "runner",
        "--target",
        "runner-target-1",
        "--name",
        "remote runner one",
        "--runner",
        "remote-process",
        "--workspace",
        "/tmp/runner-ws",
        "--artifacts",
        "/tmp/runner-art",
        "--endpoint",
        "runner-endpoint-1",
        "--server-addr",
        &address,
        "--connect",
        &address,
        "--state",
        &runner_state_str,
    ]);
    assert!(
        announce.contains("role=runner"),
        "expected role banner:\n{announce}"
    );
    assert!(
        announce.contains("runner_announced=true"),
        "expected runner announce:\n{announce}"
    );
    assert!(
        announce.contains("announce_source=runner_jsonrpc"),
        "expected the JSON-RPC announce source (DT-D1), not a local store write:\n{announce}"
    );
    assert!(
        announce.contains("appended_by=server"),
        "expected the SERVER (single writer) to own the append:\n{announce}"
    );
    assert!(
        announce.contains("runtime_target=runner-target-1"),
        "expected the announced target id:\n{announce}"
    );
    // The server control endpoint resolved as a reachable loopback peer.
    assert!(
        announce.contains("server_control_reachability=reachable"),
        "expected the loopback control endpoint reachable:\n{announce}"
    );
    let sequence: i64 = output_value(&announce, "sequence")
        .parse()
        .expect("announce sequence");
    assert!(sequence > 0, "expected a real append sequence:\n{announce}");

    // Finding 2: the runner's OWN state root must hold no authoritative log
    // (no local store write happened); the event exists ONLY in the server's
    // log, reachable solely via the TCP tail below.
    let runner_db = runner_state.join("capo.sqlite");
    assert!(
        !runner_db.exists(),
        "the runner must not have written an authoritative store locally; the announce must travel over TCP. found: {}",
        runner_db.display()
    );

    // Finding 3: the CLIENT role also starts from role config, in its own
    // process, against the live server address, and reports a reachable tail.
    let client_role = capo([
        "role",
        "client",
        "--server-addr",
        &address,
        "--state",
        &runner_state_str,
    ]);
    assert!(
        client_role.contains("role=client"),
        "expected client role banner:\n{client_role}"
    );
    assert!(
        client_role.contains("server_tail_reachability=reachable"),
        "the client role must resolve the live server tail as reachable:\n{client_role}"
    );

    // The CLIENT tail (the `subscribe_tcp` seam the `role client` resolves to)
    // reads from sequence 0 and must see the server-appended
    // `runtime.target_registered` event. The client holds no authoritative state;
    // it resumes purely by sequence cursor against the SERVER's log.
    let (backlog, _stream) = subscribe_tcp(&address, None, 0).expect("client subscribe tail");
    let registered = backlog
        .events
        .iter()
        .find(|event| event.kind == "runtime.target_registered")
        .expect("client tail must contain the server-appended runtime.target_registered");
    assert_eq!(
        registered.item_id.as_deref(),
        Some("runner-target-1"),
        "the tailed event must be the announced target"
    );
    assert_eq!(
        registered.sequence, sequence,
        "the tailed event sequence must equal the server-reported append sequence"
    );
    // Auditability + no-credential invariant: the announce carries the endpoint by
    // HANDLE only and records the announce source; no credential material is in the
    // payload.
    assert!(
        registered
            .payload_json
            .contains("\"announce_source\":\"runner_jsonrpc\""),
        "the appended event must record the announce source:\n{}",
        registered.payload_json
    );
    assert!(
        registered.payload_json.contains("runner-endpoint-1"),
        "the appended event must reference the endpoint by handle:\n{}",
        registered.payload_json
    );
    for marker in ["password", "authkey", "secret", "api_key", "token", "@"] {
        assert!(
            !registered
                .payload_json
                .to_ascii_lowercase()
                .contains(marker),
            "runtime.target_registered payload must carry no credential marker `{marker}`:\n{}",
            registered.payload_json
        );
    }

    let _ = server.kill();
    let _ = server.wait();
}

/// Finding 1: with the in-process fallback removed, a runner announce against a
/// dead server (no listener at the resolved loopback address) FAILS loudly --
/// the announce is genuinely over the JSON-RPC transport and cannot silently
/// succeed in-process. This is the negative half of the proof that the announce
/// rides the socket.
#[test]
fn runner_announce_against_dead_server_fails_loudly_no_inprocess_fallback() {
    let runner_state = temp_root("role-topology-dead-server-state");
    let runner_state_str = runner_state.display().to_string();
    // A loopback address with no server bound to it.
    let dead_addr = unused_loopback_address();

    let output = capo_failure([
        "role",
        "runner",
        "--target",
        "runner-target-dead",
        "--name",
        "remote runner dead",
        "--runner",
        "remote-process",
        "--workspace",
        "/tmp/runner-ws",
        "--artifacts",
        "/tmp/runner-art",
        "--endpoint",
        "runner-endpoint-dead",
        "--server-addr",
        &dead_addr,
        "--connect",
        &dead_addr,
        "--state",
        &runner_state_str,
    ]);
    // The announce must FAIL loudly (non-zero exit, captured by `capo_failure`)
    // because there is no live server and no in-process fallback. We do not pin
    // the exact transport errno (a dropped listener may surface as
    // ConnectionRefused or ConnectionReset depending on the OS); the load-bearing
    // facts are (a) it failed and (b) it did NOT silently write in-process.
    assert!(
        !output.contains("runner_announced=true"),
        "a dead server must NOT report a successful announce:\n{output}"
    );
    // No local store write happened either: the only writer is the server, over
    // the socket, which was never reached.
    assert!(
        !runner_state.join("capo.sqlite").exists(),
        "a failed announce must not have written a local authoritative store"
    );
}

/// Finding 6: `--connect` and a loopback `--server-addr` that DISAGREE are
/// rejected up front, before any socket -- two flags silently naming "the
/// server address" with different values is an operator footgun.
#[test]
fn runner_rejects_connect_disagreeing_with_loopback_server_addr() {
    let runner_state = temp_root("role-topology-connect-mismatch-state");
    let runner_state_str = runner_state.display().to_string();
    let output = capo_failure([
        "role",
        "runner",
        "--target",
        "runner-target-mismatch",
        "--name",
        "remote runner mismatch",
        "--runner",
        "remote-process",
        "--workspace",
        "/tmp/runner-ws",
        "--artifacts",
        "/tmp/runner-art",
        "--server-addr",
        "127.0.0.1:7878",
        "--connect",
        "127.0.0.1:8888",
        "--state",
        &runner_state_str,
    ]);
    assert!(
        output.contains("disagrees with the resolved loopback server endpoint"),
        "a --connect that diverges from a loopback --server-addr must be rejected:\n{output}"
    );
}

/// Finding 4: idempotency regression. Announcing the SAME `runtime_target_id`
/// twice returns the SAME server sequence (the state store keys on the
/// idempotency key and does not re-insert), and the client tail sees the
/// `runtime.target_registered` event for that target EXACTLY ONCE.
#[test]
fn re_announcing_same_runtime_target_is_idempotent_on_sequence_and_tail() {
    let server_state = temp_root("role-topology-idempotent-server-state");
    let runner_state = temp_root("role-topology-idempotent-runner-state");
    let mut server = spawn_server(&server_state, 1_000);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let runner_state_str = runner_state.display().to_string();

    let announce_once = || {
        capo([
            "role",
            "runner",
            "--target",
            "runner-target-idem",
            "--name",
            "remote runner idem",
            "--runner",
            "remote-process",
            "--workspace",
            "/tmp/runner-ws",
            "--artifacts",
            "/tmp/runner-art",
            "--endpoint",
            "runner-endpoint-idem",
            "--server-addr",
            &address,
            "--connect",
            &address,
            "--state",
            &runner_state_str,
        ])
    };

    let first = announce_once();
    let second = announce_once();
    let first_sequence: i64 = output_value(&first, "sequence")
        .parse()
        .expect("first announce sequence");
    let second_sequence: i64 = output_value(&second, "sequence")
        .parse()
        .expect("second announce sequence");
    assert_eq!(
        first_sequence, second_sequence,
        "a re-announce on the same target id must return the same server sequence (idempotent):\nfirst:\n{first}\nsecond:\n{second}"
    );

    // The client tail must contain the target's `runtime.target_registered`
    // exactly once, not duplicated by the re-announce.
    let (backlog, _stream) = subscribe_tcp(&address, None, 0).expect("client subscribe tail");
    let occurrences = backlog
        .events
        .iter()
        .filter(|event| {
            event.kind == "runtime.target_registered"
                && event.item_id.as_deref() == Some("runner-target-idem")
        })
        .count();
    assert_eq!(
        occurrences, 1,
        "the re-announce must not duplicate the runtime.target_registered event in the tail"
    );

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn server_role_resolves_loopback_bind_and_marks_private_blocked_pending_permission() {
    let state_root = temp_root("role-topology-server-state");
    let state = state_root.display().to_string();

    // Default server role: loopback bind, reachable, all-local default true.
    let loopback = capo(["role", "server", "--state", &state]);
    assert!(loopback.contains("role=server"), "{loopback}");
    assert!(
        loopback.contains("server_bind_reachability=reachable"),
        "loopback bind must be reachable:\n{loopback}"
    );
    assert!(
        loopback.contains("all_local_default=true"),
        "the no-flags server role is the all-local default:\n{loopback}"
    );

    // A private exposure is blocked_pending_permission until the DT5 grant path.
    let private = capo([
        "role",
        "server",
        "--server-endpoint",
        "server-private-ep",
        "--exposure",
        "private",
        "--state",
        &state,
    ]);
    assert!(
        private.contains("server_bind_reachability=blocked_pending_permission"),
        "a private exposure must be blocked_pending_permission:\n{private}"
    );
    assert!(
        private.contains("server_bind_exposure=private"),
        "the resolved exposure must be private:\n{private}"
    );
}

#[test]
fn client_role_resolves_server_tail_and_rejects_missing_endpoint() {
    let state_root = temp_root("role-topology-client-state");
    let state = state_root.display().to_string();

    let client = capo([
        "role",
        "client",
        "--server-addr",
        "127.0.0.1:7878",
        "--state",
        &state,
    ]);
    assert!(client.contains("role=client"), "{client}");
    assert!(
        client.contains("server_tail_reachability=reachable"),
        "loopback server tail must be reachable:\n{client}"
    );
    assert!(
        client.contains("holds_authoritative_state=false"),
        "a client holds no authoritative state:\n{client}"
    );

    // A client with NO server endpoint is rejected up front (typed error), before
    // any socket. `capo` (the success-asserting helper) is not used here because we
    // expect a non-zero exit.
    let output = capo_failure(["role", "client", "--state", &state]);
    assert!(
        output.contains("requires --server-endpoint"),
        "missing server endpoint must be rejected up front:\n{output}"
    );
}

#[test]
fn runner_role_rejects_missing_server_endpoint_up_front() {
    let state_root = temp_root("role-topology-runner-reject-state");
    let state = state_root.display().to_string();
    let output = capo_failure([
        "role",
        "runner",
        "--target",
        "t1",
        "--name",
        "n1",
        "--runner",
        "remote-process",
        "--workspace",
        "/tmp/ws",
        "--artifacts",
        "/tmp/art",
        "--state",
        &state,
    ]);
    assert!(
        output.contains("requires --server-endpoint"),
        "a runner with no server endpoint must be rejected before any socket:\n{output}"
    );
}
