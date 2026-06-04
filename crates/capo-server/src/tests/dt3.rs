//! DT3 (distributed-topology) capo-server remote-attach tests.
//!
//! DT3 wires the server's turn loop to drive an agent process on a REMOTE runner
//! through `RemoteProcessRunner` resolved over a `ConnectivityTunnel`, keeping
//! runtime ownership on the runner and orchestration state on the server. The
//! runner-level start/recovery/redaction matrix is proven exhaustively in
//! `capo-runtime`'s deterministic fake-channel suite (the `dt3_*` tests there);
//! these tests exercise the DT3 seam from the `capo-server` crate (the
//! `-p capo-server` verification gate the DT3 section names) so the server-side
//! seam is covered, not solely self-attested inside `capo-runtime`:
//!
//! - the server resolves the runner's runtime endpoint via the tunnel and drives a
//!   remote process through the EXISTING runner surface (append-first start
//!   sequence + streamed output), with NO loop change;
//! - the two redaction seams are DISTINCT and BOTH hold: the runner-side pass
//!   scrubs a seeded secret BEFORE it crosses the (fake) tunnel, and the server's
//!   egress backstop (`ServerEvent::from_record` via `subscribe`) scrubs a secret
//!   that slipped into a `safe`-labeled payload before it reaches the client tail.
//!
//! All deterministic: the tunnel is a `FakeTunnel` (resolves at loopback exposure,
//! no grant) and the transport is the in-memory `FakeRemoteChannel` (NO network,
//! NO real SSH).

use capo_core::{ProjectId, RunId, SessionId};
use capo_runtime::{
    ChannelKind, ConnectivityTunnel, EndpointOwner, FakeRemoteChannel, LocalProcessRequest,
    RemoteChannel, RemoteRunnerAttach, RemoteStreamFinalReason,
};
use capo_state::{EventKind, NewEvent, RedactionState};

use super::temp_root;
use crate::{CapoServer, EventNotification};

/// DT3: build a `RemoteRunnerAttach` over a fake tunnel, binding a fake-channel
/// transport (optionally scripted) keyed to the channel the TUNNEL opened. This is
/// the DT1/DT3 seam: reachability is resolved on the connectivity boundary; the
/// runner that comes back owns only the process group.
fn attach_over_tunnel(
    name: &str,
    script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
) -> (RemoteRunnerAttach, capo_tmptest::TempRoot) {
    // `workspace` is the guard returned to (and kept alive by) the caller; the
    // artifacts dir lives under it so both are cleaned up when the caller drops it.
    let workspace = temp_root();
    let artifacts = workspace.join(format!("artifacts-{name}"));
    std::fs::create_dir_all(&workspace).expect("workspace");

    let tunnel = ConnectivityTunnel::fake();
    let owner = EndpointOwner::runtime_target(format!("runner-{name}"));
    let ws = workspace.to_path_buf();
    let attach = RemoteRunnerAttach::resolve(&tunnel, owner, ChannelKind::Stdio, |channel| {
        RemoteChannel::Fake(script(FakeRemoteChannel::from_open_channel(
            channel, ws, artifacts,
        )))
    })
    .expect("fake tunnel resolves a loopback attach without a grant");
    (attach, workspace)
}

#[test]
fn server_drives_remote_process_through_tunnel_resolved_runner() {
    // The server resolves the runner endpoint over the tunnel and dispatches a turn
    // to the remote runner: the append-first start sequence
    // (`start_requested` -> `remote_process_started`) is recorded and output streams
    // back through the EXISTING runner surface, with no loop change.
    let (attach, workspace) = attach_over_tunnel("dt3-srv-drive", |c| {
        c.with_streamed_output(b"remote turn output".to_vec())
    });
    assert!(
        attach.is_loopback(),
        "the deterministic attach must honestly report a loopback transport"
    );

    let outcome = attach
        .runner()
        .start_process(LocalProcessRequest {
            run_id: RunId::new("dt3-srv-drive"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "printf x".to_string()],
            cwd: workspace.to_path_buf(),
            env: std::collections::HashMap::new(),
        })
        .expect("remote start over the tunnel-resolved runner");

    let kinds: Vec<&str> = outcome.events.iter().map(|e| e.kind.as_str()).collect();
    let req = kinds
        .iter()
        .position(|k| *k == "runtime.remote_start_requested")
        .expect("start_requested present");
    let started = kinds
        .iter()
        .position(|k| *k == "runtime.remote_process_started")
        .expect("remote_process_started present");
    assert!(req < started, "append-first ordering must hold: {kinds:?}");

    let stream = attach.runner().stream_output(&outcome.process, 0);
    assert_eq!(stream.final_reason, RemoteStreamFinalReason::Eof);
    let streamed: String = stream.deltas.iter().map(|d| d.text.clone()).collect();
    assert_eq!(streamed, "remote turn output");
}

#[test]
fn dt3_two_redaction_seams_both_hold_runner_side_and_server_egress() {
    // The TWO distinct seams, asserted separately.
    let secret = "AKIAIOSFODNN7EXAMPLE";

    // Seam 1 — RUNNER-SIDE before transit: a seeded secret in remote output is
    // scrubbed by the runner's redaction pass before the delta becomes an
    // event/artifact crossing the (fake) tunnel.
    let raw = format!("step ok\nkey={secret}\nstep done");
    let (attach, workspace) = attach_over_tunnel("dt3-redact", move |c| {
        c.with_streamed_output(raw.into_bytes())
    });
    let outcome = attach
        .runner()
        .start_process(LocalProcessRequest {
            run_id: RunId::new("dt3-redact"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "printf x".to_string()],
            cwd: workspace.to_path_buf(),
            env: std::collections::HashMap::new(),
        })
        .expect("remote start");
    let stream = attach.runner().stream_output(&outcome.process, 0);
    let forwarded: String = stream.deltas.iter().map(|d| d.text.clone()).collect();
    assert!(
        !forwarded.contains(secret),
        "runner-side seam must scrub the secret before transit: {forwarded:?}"
    );
    assert_eq!(stream.redaction_state, "redacted");

    // Seam 2 — SERVER-SIDE egress backstop: a secret that slipped into a
    // `safe`-labeled payload is scrubbed by the server's subscription egress
    // (`ServerEvent::from_record` -> `redacted_for_egress`) before it reaches the
    // client tail. This is UPSTREAM of the runner leg and a DISTINCT mechanism.
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let mut event = NewEvent::new(
        "dt3-egress-leak",
        EventKind::SessionSummaryUpdated,
        "controller",
    );
    event.project_id = Some(ProjectId::new("project-capo"));
    event.session_id = Some(SessionId::new("session-dt3"));
    event.payload_json = format!("{{\"note\":\"{secret}\"}}");
    // Labeled SAFE on purpose — the egress backstop must still catch it.
    event.redaction_state = RedactionState::Safe;
    server
        .state_for_test()
        .append_event(event, &[])
        .expect("append seeded event");

    let (backlog, _stream) = server.subscribe(None, 0).expect("subscribe from 0");
    let egress = backlog
        .events
        .iter()
        .find(|e| e.event_id == "dt3-egress-leak")
        .expect("seeded event in backlog");
    let wire = EventNotification::for_event(egress).to_wire_frame();
    assert!(
        !wire.contains(secret),
        "server-side egress seam must scrub the secret before the client tail: {wire}"
    );
    assert_eq!(
        egress.redaction_state, "redacted",
        "the egress backstop must upgrade the classification when it scrubs"
    );
}
