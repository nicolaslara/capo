//! DT7: the cross-device end-to-end smoke (opt-in, gated) PAIRED with an always-on
//! deterministic three-process E2E gate.
//!
//! This is the capstone integration of the distributed-topology workpad. It proves
//! Capo runs as THREE roles -- server/controller, remote runner, client -- end to
//! end, composing the seams the prior DT tasks built:
//!   - DT1  role config + the JSON-RPC runner ANNOUNCE (three OS processes);
//!   - DT2  the runner<->server LOGGED keep-alive plane (degrade then recover,
//!     fake clock, no wall-clock sleep);
//!   - DT3  the tunnel-resolved remote attach + the runner-side redaction pass;
//!   - DT4a the watermark resume of already-committed events across a drop;
//!   - DT4b the runner-side spool + idempotent replay (exactly-once);
//!   - DT5  the auditable + REVOCABLE remote control (a revoked grant refuses new
//!     execution).
//!
//! The workpad-wide verification invariant (`knowledge.md`) is that no task
//! completes on operator self-attestation alone: every live smoke is PAIRED with a
//! deterministic assertion of the SAME shape via ONE shared helper
//! ([`assert_distributed_e2e_shape`]). DT7 honours that with two tests that share
//! that helper:
//!
//! 1. [`distributed_e2e_gate_runs_three_roles_over_loopback`] -- ALWAYS ON (no
//!    network, no live provider, no env mutation). It spawns the three roles as
//!    THREE SEPARATE OS PROCESSES over loopback (`capo role server` / `capo role
//!    runner` / `capo role client`) with an explicit PER-STEP TIMEOUT + KILL/REAP
//!    bound (so the gate can neither hang nor leak a process), drives the runner's
//!    JSON-RPC announce, tails it from the client, then composes the DT2/DT3/DT4a/
//!    DT4b/DT5 seams DETERMINISTICALLY (fake clock, deterministic drop seam) into
//!    one [`DistributedE2eShape`] and runs the shared shape assertion.
//!
//! 2. [`live_distributed_smoke`] -- `#[ignore]`d AND behind the explicit opt-in env
//!    gate [`LIVE_DISTRIBUTED_ENV`] (`CAPO_SERVER_RUN_DISTRIBUTED_LIVE`, mirroring
//!    the `CAPO_SERVER_RUN_*_LIVE` family). Its reachability preflight probes a REAL
//!    Tailscale endpoint via the in-tree CT10 predicate `live_tailscale_smoke_decision`
//!    (the `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT` + `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE`
//!    gates + the live `tailscale status --json` source) -- NOT `ConnectivityTunnel::fake()`,
//!    which always reports reachable. It skips CLEANLY when unset or when no reachable
//!    tailnet peer exists, so it never runs in ordinary test runs and is never the SOLE
//!    evidence. When opted in it resolves the DT3 attach over a REAL
//!    `ConnectivityTunnel::Tailscale` (with the DT5 grant a `Private` endpoint requires)
//!    and asserts the IDENTICAL shape via the shared helper.

use std::io::BufReader;
use std::process::Child;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use capo_runtime::{
    ChannelKind, ConnectivityClock, ConnectivityEndpointConfig, ConnectivityTunnel, EndpointOwner,
    ExposureBindGrant, ExposureScope, FakeRemoteChannel, FakeTunnelScript, HealthState,
    HeartbeatConfig, HeartbeatMonitor, LiveTailscaleSmokeDecision, LiveTailscaleStatusSource,
    LocalProcessRequest, RemoteChannel, RemoteRunnerAttach, RemoteStreamFinalReason,
    RunnerEventSpool, RunnerServerPlane, RuntimeError, SpoolAdmission, TailscaleStatusSource,
    live_tailscale_smoke_decision,
};
use capo_server::{
    CapoServer, RunnerReplayFrame, ServerCommand, ServerRequest, ServerResponsePayload,
    subscribe_tcp,
};
use capo_state::EventKind;

use super::support::*;

/// The explicit opt-in env gate for the live cross-device smoke, mirroring the
/// `CAPO_SERVER_RUN_STREAMING_LIVE` / `CAPO_SERVER_RUN_CODEX_LIVE` convention. The
/// smoke is `#[ignore]`d AND env-gated, so it never runs in ordinary test runs and
/// never stands as the only evidence for the task.
const LIVE_DISTRIBUTED_ENV: &str = "CAPO_SERVER_RUN_DISTRIBUTED_LIVE";

/// The endpoint id the live smoke resolves over the real tailnet. The CT10 live
/// predicate matches it against the peer's MagicDNS `DNSName` / `HostName`, so an
/// operator names their own tailnet runner host here (or via the env override).
const LIVE_TAILNET_ENDPOINT_DEFAULT: &str = "capo-runner";

/// Optional override for [`LIVE_TAILNET_ENDPOINT_DEFAULT`], so an operator points the
/// live smoke at their own tailnet host without editing the test.
const LIVE_TAILNET_ENDPOINT_ENV: &str = "CAPO_DISTRIBUTED_TAILNET_ENDPOINT";

/// A bounded per-step deadline so a hung subprocess fails the gate LOUDLY (with a
/// kill + reap) rather than blocking the suite forever. The deterministic gate must
/// never hang: every blocking step is wrapped by [`with_timeout`].
const STEP_DEADLINE: Duration = Duration::from_secs(20);

/// A seeded secret marker. The shared shape assertion proves it never crosses the
/// (fake) tunnel on the runner leg, nor reaches a spooled/replayed frame.
const FORBIDDEN_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

/// The reachability/attach transport the DT3 runner-side redaction step resolves
/// over. The deterministic gate uses [`E2eTunnel::Fake`] (loopback / `FakeTunnel`,
/// no network); the LIVE smoke uses [`E2eTunnel::Tailscale`] -- a REAL
/// `ConnectivityTunnel::Tailscale` resolved over the live `tailscale status --json`
/// source, NOT a fake (resolving findings 1 + 2: the live path must not be the same
/// `FakeTunnel` the always-on gate uses, and the preflight must probe a real
/// endpoint, not `fake()` -- which always returns `reachable: true`).
///
/// A real Tailscale endpoint resolves at `Private`, so it `permission_required`s an
/// active [`ExposureBindGrant`]; the live variant carries that grant so the attach
/// honours the DT5 gate exactly as a real non-loopback control channel would.
enum E2eTunnel {
    Fake,
    Tailscale {
        endpoint_id: String,
        grant: ExposureBindGrant,
    },
}

impl E2eTunnel {
    /// Build the `ConnectivityTunnel` this run resolves the DT3 attach over.
    fn tunnel(&self) -> ConnectivityTunnel {
        match self {
            // Deterministic gate: loopback / fake, no network.
            E2eTunnel::Fake => ConnectivityTunnel::fake(),
            // Live smoke: a REAL Tailscale tunnel backed by the live
            // `tailscale status --json` source (the gated CT10 surface), so the
            // attach crosses a real reachability boundary -- not the fake the gate
            // uses.
            E2eTunnel::Tailscale { endpoint_id, .. } => ConnectivityTunnel::tailscale(
                ConnectivityEndpointConfig::tailscale(endpoint_id.clone(), endpoint_id.clone()),
                TailscaleStatusSource::new(LiveTailscaleStatusSource::default()),
            ),
        }
    }

    /// The DT5 grant the attach needs (a real Tailscale `Private` endpoint is
    /// `permission_required`; the fake loopback resolve needs none).
    fn grant(&self) -> Option<&ExposureBindGrant> {
        match self {
            E2eTunnel::Fake => None,
            E2eTunnel::Tailscale { grant, .. } => Some(grant),
        }
    }

    /// Whether the resolved attach is expected to ride a loopback/fake transport
    /// (true for the deterministic gate) vs. a real machine boundary (false for the
    /// live Tailscale path). The honesty guard asserts the attach matches this.
    fn expects_loopback(&self) -> bool {
        matches!(self, E2eTunnel::Fake)
    }
}

/// A child process guard that KILLS + REAPS on drop, so a panicking step can never
/// leak the server subprocess (the "explicit cleanup" the DT7 AC names). The bounded
/// `--max-requests` accept budget is the primary reaper; this is the belt-and-braces
/// guarantee for the panic path.
struct ReapOnDrop {
    child: Option<Child>,
}

impl ReapOnDrop {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    /// Explicitly KILL + REAP the server subprocess after the gate's steps complete,
    /// then confirm it is reaped (no leaked process). We do not rely on an exact
    /// `--max-requests` accept budget here -- the `role client` reachability probe
    /// and the explicit `subscribe_tcp` tail open an implementation-detail number of
    /// connections -- so the gate's cleanup is an explicit kill, exactly as the DT7
    /// AC names ("an explicit TIMEOUT + CLEANUP (kill + reap) bound on every step so
    /// the gate cannot hang or leak processes"). Consumes the guard so Drop does not
    /// double-reap. Returns `true` once the process has been reaped.
    fn kill_and_reap(mut self) -> bool {
        match self.child.take() {
            Some(mut child) => {
                let _ = child.kill();
                child.wait().is_ok()
            }
            None => false,
        }
    }
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Run `op` on a scratch thread bounded by [`STEP_DEADLINE`]: if it does not finish
/// in time the gate FAILS LOUDLY (it does not hang). This is the per-step timeout the
/// DT7 AC requires on every blocking step of the three-process gate.
fn with_timeout<T, F>(label: &str, op: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = tx.send(op());
    });
    match rx.recv_timeout(STEP_DEADLINE) {
        Ok(value) => {
            let _ = worker.join();
            value
        }
        Err(_) => panic!(
            "DT7 gate step `{label}` exceeded the {STEP_DEADLINE:?} deadline \
             (a hang, not a pass): the step is killed and the gate fails loudly"
        ),
    }
}

/// The observable shape one distributed end-to-end run produces. Captured once by
/// the deterministic gate (and once by the live smoke) and asserted by the single
/// shared helper, so the always-on gate and the live cross-device smoke verify the
/// IDENTICAL contract -- the live evidence is never operator-attested.
struct DistributedE2eShape {
    /// The client tail observed the runner's SERVER-APPENDED
    /// `runtime.target_registered` (the DT1 three-process announce).
    observed_runtime_target_registered: bool,
    /// The runner leg's LOGGED health transitions, in order (DT2). Must contain a
    /// degrade-then-recover edge.
    runner_health_states: Vec<HealthState>,
    /// The runner-side redaction output forwarded across the (fake) tunnel (DT3).
    /// The secret MUST already be scrubbed here.
    runner_forwarded_output: String,
    /// The full committed sequence reconstructed from a pre-drop tail UNIONed with a
    /// post-resume `from_sequence` tail (DT4a). Must be strictly increasing +
    /// contiguous with no gap, no dupe.
    resumed_sequence: Vec<i64>,
    /// The per-event-id occurrence counts of the runner's SPOOLED + replayed events
    /// in the committed log (DT4b). Each must be exactly 1 (idempotent replay).
    replayed_event_occurrences: Vec<(String, usize)>,
    /// Whether a control attempt AFTER revoke was refused (DT5).
    revoked_control_refused: bool,
    /// Every wire/payload string the run produced that we must prove secret-free
    /// (the runner-forwarded output, the spooled+replayed payloads).
    secret_scanned_strings: Vec<String>,
}

/// The SINGLE shared shape assertion both the deterministic gate and the live smoke
/// call. It pins the DT7 end-to-end contract for one distributed run across all six
/// composed seams:
///
/// - DT1: the client tail observed the runner's server-appended target registration;
/// - DT2: the runner leg LOGGED a degrade-then-recover health path;
/// - DT3: the runner-side redaction scrubbed the seeded secret before it crossed the
///   tunnel;
/// - DT4a: the resumed sequence is strictly increasing + contiguous (no gap/dupe);
/// - DT4b: every spooled+replayed event appears EXACTLY ONCE in the committed log;
/// - DT5: a control attempt after revoke was refused;
/// - cross-cutting: no seeded secret appears in ANY produced wire/payload string.
fn assert_distributed_e2e_shape(shape: &DistributedE2eShape) {
    // DT1: three-process announce visible in the client tail.
    assert!(
        shape.observed_runtime_target_registered,
        "the client tail must observe the runner's server-appended runtime.target_registered (DT1)"
    );

    // DT2: a degrade-then-recover health path on the LOGGED runner leg. The path
    // must reach a non-available state and then return to available.
    assert!(
        shape
            .runner_health_states
            .iter()
            .any(|state| *state != HealthState::Available),
        "the runner leg must LOG a degrade away from available (DT2): {:?}",
        shape.runner_health_states
    );
    assert_eq!(
        shape.runner_health_states.last().copied(),
        Some(HealthState::Available),
        "the runner leg must RECOVER to available after the degrade (DT2): {:?}",
        shape.runner_health_states
    );

    // DT3: runner-side redaction-before-transit.
    assert!(
        !shape.runner_forwarded_output.is_empty(),
        "the remote runner must have streamed some output across the tunnel (DT3)"
    );
    assert!(
        !shape.runner_forwarded_output.contains(FORBIDDEN_SECRET),
        "the runner-side redaction must scrub the secret BEFORE it crosses the tunnel (DT3): {:?}",
        shape.runner_forwarded_output
    );

    // DT4a: watermark resume with no gap, no dupe, strictly increasing + contiguous.
    assert!(
        shape.resumed_sequence.len() >= 2,
        "the resumed sequence must cover the committed events (DT4a): {:?}",
        shape.resumed_sequence
    );
    for window in shape.resumed_sequence.windows(2) {
        assert_eq!(
            window[1],
            window[0] + 1,
            "the resumed sequence must be contiguous (no gap, no dupe) across the drop (DT4a): {:?}",
            shape.resumed_sequence
        );
    }

    // DT4b: idempotent replay -- every spooled+replayed event appears exactly once.
    assert!(
        !shape.replayed_event_occurrences.is_empty(),
        "the runner must have spooled+replayed at least one event (DT4b)"
    );
    for (event_id, count) in &shape.replayed_event_occurrences {
        assert_eq!(
            *count, 1,
            "the spooled event `{event_id}` must appear EXACTLY ONCE in the committed log (DT4b)"
        );
    }

    // DT5: a revoked grant refuses new execution.
    assert!(
        shape.revoked_control_refused,
        "a control attempt after revoke must be refused (DT5)"
    );

    // Cross-cutting safety: no seeded secret crosses any produced wire/payload.
    for scanned in &shape.secret_scanned_strings {
        assert!(
            !scanned.contains(FORBIDDEN_SECRET),
            "a seeded secret leaked into a produced wire/payload string: {scanned:?}"
        );
    }
}

/// Spawn the three role processes over loopback and drive the DT1 announce + tail,
/// returning whether the client tail saw the runner's server-appended
/// `runtime.target_registered`. THREE SEPARATE OS PROCESSES, each bounded by a
/// per-step timeout and reaped on drop, so the gate can neither hang nor leak.
fn drive_three_process_announce_and_tail() -> bool {
    let server_state = temp_root("dt7-gate-server-state");
    let runner_state = temp_root("dt7-gate-runner-state");

    // PROCESS 1: the server/controller. A generous accept budget keeps it alive
    // across the runner announce + the client role probe + the client tail; the
    // explicit kill+reap below is the gate's cleanup bound (no reliance on an exact
    // budget that an implementation-detail connection count could break).
    let mut server = spawn_server(&server_state, 1_000);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let server = ReapOnDrop::new(server);
    let address = with_timeout("read-server-address", move || {
        read_server_address(&mut reader)
    });
    let runner_state_str = runner_state.display().to_string();

    // PROCESS 2: the remote runner, announcing its runtime target over JSON-RPC to
    // the live server (DT1/DT-D1). A distinct, empty state root proves the announce
    // rode TCP, not a local store write.
    let announce_addr = address.clone();
    let announce_state = runner_state_str.clone();
    let announce = with_timeout("runner-announce", move || {
        capo([
            "role",
            "runner",
            "--target",
            "dt7-runner-target",
            "--name",
            "dt7 remote runner",
            "--runner",
            "remote-process",
            "--workspace",
            "/tmp/dt7-runner-ws",
            "--artifacts",
            "/tmp/dt7-runner-art",
            "--endpoint",
            "dt7-runner-endpoint",
            "--server-addr",
            &announce_addr,
            "--connect",
            &announce_addr,
            "--state",
            &announce_state,
        ])
    });
    assert!(
        announce.contains("runner_announced=true")
            && announce.contains("announce_source=runner_jsonrpc")
            && announce.contains("appended_by=server"),
        "the runner process must announce over JSON-RPC and the server must own the append:\n{announce}"
    );
    let sequence: i64 = output_value(&announce, "sequence")
        .parse()
        .expect("announce sequence");

    // PROCESS 3: the client, resolving the server tail from role config.
    let client_addr = address.clone();
    let client_state = runner_state_str.clone();
    let client_role = with_timeout("client-role", move || {
        capo([
            "role",
            "client",
            "--server-addr",
            &client_addr,
            "--state",
            &client_state,
        ])
    });
    assert!(
        client_role.contains("server_tail_reachability=reachable"),
        "the client role process must resolve the live server tail as reachable:\n{client_role}"
    );

    // The client tail (`subscribe_tcp`) must observe the runner's server-appended
    // event at the announce sequence.
    let tail_addr = address.clone();
    let observed = with_timeout("client-tail", move || {
        let (backlog, _stream) = subscribe_tcp(&tail_addr, None, 0).expect("client subscribe tail");
        backlog.events.iter().any(|event| {
            event.kind == "runtime.target_registered"
                && event.item_id.as_deref() == Some("dt7-runner-target")
                && event.sequence == sequence
        })
    });

    // Explicit cleanup: kill + reap the server subprocess (bounded by the per-step
    // timeout) so the gate proves no process leaked.
    assert!(
        with_timeout("server-kill-reap", move || server.kill_and_reap()),
        "the server process must be killed + reaped by the gate cleanup (no leak)"
    );

    observed
}

/// DT2: drive the runner<->server LOGGED keep-alive plane through a degrade and a
/// recover using a FAKE CLOCK (no wall-clock sleep), returning the ordered health
/// states. The scripted tunnel timeline is the deterministic drop seam.
fn drive_runner_health_degrade_recover() -> Vec<HealthState> {
    let clock = ConnectivityClock::manual(0);
    let tunnel = ConnectivityTunnel::fake_scripted(
        FakeTunnelScript::private_matching("dt7-endpoint", "dt7-runner")
            // available, miss (degraded), miss (unreachable), recover (available).
            .with_health_timeline(vec![true, false, false, true]),
    );
    let monitor = HeartbeatMonitor::new(tunnel, clock.clone(), HeartbeatConfig::default());
    let mut plane = RunnerServerPlane::new(monitor, "runtime-proc:dt7");

    let mut states = Vec::new();
    let mut recovered = false;
    for _ in 0..4 {
        let beat = plane.beat();
        states.push(plane.state());
        // DT2 wiring: a recovered leg flags that the recovery sequence must re-run.
        if beat.must_rerun_recovery {
            recovered = true;
        }
        clock.advance(15_000);
    }
    assert!(
        recovered,
        "the recovered runner leg must flag must_rerun_recovery (DT2 recovery re-run)"
    );
    states
}

/// DT3: build a tunnel-resolved remote attach over a fake channel that streams a
/// secret-laden output, then prove the runner-side redaction pass scrubs the secret
/// before it crosses the tunnel. Returns the forwarded (already-redacted) output.
/// The returned `RemoteRunnerAttach` is reused by the DT5 revoke seam.
fn drive_runner_side_redaction(
    e2e_tunnel: &E2eTunnel,
) -> (String, RemoteRunnerAttach, std::path::PathBuf) {
    let root = temp_root("dt7-redact");
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let raw = format!("remote step ok\nkey={FORBIDDEN_SECRET}\nremote step done");
    let tunnel = e2e_tunnel.tunnel();
    let owner = EndpointOwner::runtime_target("dt7-runner");
    let ws = workspace.clone();
    // The DT3 attach resolves the runner endpoint over the SELECTED tunnel (fake for
    // the gate; a real Tailscale tunnel for the live smoke) and binds the opened
    // reachability channel to the runner. A real Tailscale endpoint is
    // `permission_required`, so the live path threads its active DT5 grant.
    let attach = RemoteRunnerAttach::resolve_with_grant(
        &tunnel,
        owner,
        ChannelKind::Stdio,
        e2e_tunnel.grant(),
        move |channel| {
            RemoteChannel::Fake(
                FakeRemoteChannel::from_open_channel(channel, ws, artifacts)
                    .with_streamed_output(raw.into_bytes()),
            )
        },
    )
    .expect("the selected tunnel resolves an attach (loopback fake, or granted Tailscale)");
    assert_eq!(
        attach.is_loopback(),
        e2e_tunnel.expects_loopback(),
        "the attach must honestly report whether it rode a loopback/fake transport"
    );

    let outcome = attach
        .runner()
        .start_process(LocalProcessRequest {
            run_id: capo_core::RunId::new("dt7-redact"),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), "printf x".to_string()],
            cwd: workspace.clone(),
            env: std::collections::HashMap::new(),
        })
        .expect("remote start over the tunnel-resolved runner");
    let stream = attach.runner().stream_output(&outcome.process, 0);
    assert_eq!(stream.final_reason, RemoteStreamFinalReason::Eof);
    let forwarded: String = stream.deltas.iter().map(|d| d.text.clone()).collect();
    (forwarded, attach, workspace)
}

/// DT5: a revoked remote-control grant refuses NEW execution. Reuses the DT3 attach;
/// after `revoke_control`, a fresh `start_process` must fail with
/// [`RuntimeError::RemoteControlRevoked`]. Returns whether the refusal held.
fn drive_revoke_refuses_control(attach: &RemoteRunnerAttach, workspace: &std::path::Path) -> bool {
    attach.runner().revoke_control("dt7 operator revoke", None);
    let re_start = attach.runner().start_process(LocalProcessRequest {
        run_id: capo_core::RunId::new("dt7-after-revoke"),
        turn_id: None,
        program: "/bin/sh".to_string(),
        argv: vec!["-c".to_string(), "printf nope".to_string()],
        cwd: workspace.to_path_buf(),
        env: std::collections::HashMap::new(),
    });
    matches!(re_start, Err(RuntimeError::RemoteControlRevoked { .. }))
}

/// DT4b: a runner buffers `runtime.*` events while its server leg is DOWN, then on
/// reattach replays them through the PRODUCTION seam
/// (`ServerCommand::ReplayRunnerEvents` -> `CapoServer::handle()`). Replaying TWICE
/// (a retried reattach) must be idempotent. Returns the per-event-id occurrence
/// counts in the committed log + the spooled payload strings (for the secret scan).
fn drive_spool_replay_exactly_once() -> (Vec<(String, usize)>, Vec<String>) {
    let root = temp_root("dt7-spool");
    let project = capo_core::ProjectId::new("project-capo");
    let session = capo_core::SessionId::new("session-dt7-spool");
    let server = CapoServer::open(project, &root).expect("server");

    let mut spool = RunnerEventSpool::new(16);
    spool.mark_disconnected();
    let mut spooled_payloads = Vec::new();
    for i in 0..3 {
        // Seed a secret into the payload to prove the spool scrubs it before replay.
        let admission = spool.offer(
            format!("dt7-runner-evt-{i}"),
            EventKind::RuntimeRemoteOutputDelta,
            session.clone(),
            format!("runtime.remote_output_delta:run-dt7:{i}"),
            &format!("{{\"offset\":{i},\"text\":\"delta {i} token={FORBIDDEN_SECRET}\"}}"),
        );
        assert_eq!(admission, Some(SpoolAdmission::Buffered));
    }

    let drained = spool.drain_for_replay();
    assert_eq!(drained.len(), 3);
    for frame in &drained {
        spooled_payloads.push(frame.payload_json.clone());
    }

    let command =
        |drained: &[capo_runtime::SpooledRuntimeEvent]| ServerCommand::ReplayRunnerEvents {
            frames: drained.iter().map(RunnerReplayFrame::from).collect(),
        };
    let replay = |cmd: ServerCommand| -> Vec<i64> {
        let response = server
            .handle(ServerRequest::cli(cmd))
            .expect("replay handled");
        let ServerResponsePayload::RunnerEventsReplayed(summary) = response.payload else {
            panic!("expected RunnerEventsReplayed response");
        };
        summary.appended_sequences
    };

    // First replay appends; a SECOND replay of the same drained frames (a retried
    // reattach) is a no-op via the single writer's idempotency-key dedupe.
    let first = replay(command(&drained));
    let second = replay(command(&drained));
    assert_eq!(first.len(), 3);
    assert_eq!(
        first, second,
        "re-replaying already-appended frames must return their existing sequences (DT4b no-op)"
    );

    // Reopen the store and count occurrences of each spooled event id.
    let store = capo_state::SqliteStateStore::open(&root).expect("reopen store");
    let committed = store
        .events_after_for_session(&session, 0, 100_000)
        .expect("events");
    let occurrences = (0..3)
        .map(|i| {
            let event_id = format!("dt7-runner-evt-{i}");
            let count = committed
                .iter()
                .filter(|event| event.event_id == event_id)
                .count();
            (event_id, count)
        })
        .collect();
    (occurrences, spooled_payloads)
}

/// DT4a: a client tail is force-dropped mid-stream and resumes from
/// `delivered_through`; the union of the pre-drop backlog + the post-resume tail
/// equals the full committed sequence with NO gap and NO dupe. Returns that resumed
/// sequence (strictly increasing + contiguous). The drop is the DETERMINISTIC
/// `drop(stream)` seam -- no wall-clock.
fn drive_watermark_resume_no_gap_no_dupe() -> Vec<i64> {
    let root = temp_root("dt7-resume");
    let project = capo_core::ProjectId::new("project-capo");
    let session = capo_core::SessionId::new("session-dt7-resume");
    let server = CapoServer::open(project, &root).expect("server");

    // Commit a handful of events the client will tail across a drop.
    let mut spool = RunnerEventSpool::new(16);
    spool.mark_disconnected();
    for i in 0..6 {
        spool.offer(
            format!("dt7-resume-evt-{i}"),
            EventKind::RuntimeRemoteOutputDelta,
            session.clone(),
            format!("runtime.remote_output_delta:run-resume:{i}"),
            &format!("{{\"offset\":{i}}}"),
        );
    }
    let drained = spool.drain_for_replay();
    let response = server
        .handle(ServerRequest::cli(ServerCommand::ReplayRunnerEvents {
            frames: drained.iter().map(RunnerReplayFrame::from).collect(),
        }))
        .expect("replay handled");
    assert!(matches!(
        response.payload,
        ServerResponsePayload::RunnerEventsReplayed(_)
    ));

    // First tail: read the first half, capture the watermark, then DROP it.
    let (backlog, stream) = server
        .subscribe(Some(session.as_str().to_string()), 0)
        .expect("subscribe");
    let all: Vec<i64> = backlog.events.iter().map(|e| e.sequence).collect();
    assert!(all.len() >= 6, "expected the committed events: {all:?}");
    let split = all.len() / 2;
    let pre_drop: Vec<i64> = all[..split].to_vec();
    let watermark = *pre_drop.last().expect("a pre-drop watermark");
    drop(stream); // the deterministic drop seam.

    // Resume from the watermark: re-delivers strictly after it, none at/below.
    let (resumed_backlog, resumed_stream) = server
        .subscribe(Some(session.as_str().to_string()), watermark)
        .expect("resume subscribe");
    drop(resumed_stream);
    let post_resume: Vec<i64> = resumed_backlog.events.iter().map(|e| e.sequence).collect();
    assert!(
        post_resume.iter().all(|seq| *seq > watermark),
        "resume must deliver only events strictly after the watermark (DT4a): {post_resume:?}"
    );

    // The union (pre-drop + post-resume) is the full sequence with no gap, no dupe.
    let mut resumed = pre_drop;
    resumed.extend(post_resume);
    resumed
}

/// Compose all six seams into one [`DistributedE2eShape`]. Shared by the always-on
/// gate and (its deterministic half of) the live smoke pairing.
fn capture_distributed_e2e_shape(e2e_tunnel: &E2eTunnel) -> DistributedE2eShape {
    let observed_runtime_target_registered = drive_three_process_announce_and_tail();
    let runner_health_states = drive_runner_health_degrade_recover();
    let (runner_forwarded_output, attach, workspace) = drive_runner_side_redaction(e2e_tunnel);
    let revoked_control_refused = drive_revoke_refuses_control(&attach, &workspace);
    let resumed_sequence = drive_watermark_resume_no_gap_no_dupe();
    let (replayed_event_occurrences, spooled_payloads) = drive_spool_replay_exactly_once();

    let mut secret_scanned_strings = vec![runner_forwarded_output.clone()];
    secret_scanned_strings.extend(spooled_payloads);

    DistributedE2eShape {
        observed_runtime_target_registered,
        runner_health_states,
        runner_forwarded_output,
        resumed_sequence,
        replayed_event_occurrences,
        revoked_control_refused,
        secret_scanned_strings,
    }
}

/// DT7 always-on deterministic three-process E2E gate. Spawns the three roles as
/// THREE SEPARATE OS PROCESSES over loopback (DT1) with per-step timeouts + kill/reap
/// cleanup, then composes the DT2/DT3/DT4a/DT4b/DT5 seams deterministically (fake
/// clock + deterministic drop seam, no wall-clock sleep) and runs the SAME shape
/// assertion the live smoke uses. This is the deterministic pairing that keeps DT7
/// completion from being operator-attested.
#[test]
fn distributed_e2e_gate_runs_three_roles_over_loopback() {
    // The always-on gate resolves the DT3 attach over the deterministic fake tunnel
    // (loopback, no network).
    let shape = capture_distributed_e2e_shape(&E2eTunnel::Fake);
    assert_distributed_e2e_shape(&shape);
}

/// DT7 live cross-device smoke. `#[ignore]`d AND gated behind the explicit opt-in
/// env var [`LIVE_DISTRIBUTED_ENV`] (it also skips CLEANLY when unset, so an operator
/// can exercise the tailnet path without failing for everyone else). Its reachability
/// preflight probes a REAL Tailscale endpoint through the in-tree CT10 predicate
/// `live_tailscale_smoke_decision`; an unreachable tailnet (binary absent / not
/// logged in / no reachable peer) skips cleanly rather than fails.
///
/// Run it with:
///   `CAPO_SERVER_RUN_DISTRIBUTED_LIVE=1 \`
///   `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT=1 CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE=1 \`
///   `  cargo test -p capo-cli --test server_transport -- --ignored live_distributed_smoke`
/// (point it at your own tailnet host with `CAPO_DISTRIBUTED_TAILNET_ENDPOINT=<host>`.)
///
/// It drives the three roles end to end and resolves the DT3 attach over a REAL
/// `ConnectivityTunnel::Tailscale` (backed by the live `tailscale status --json`
/// source, NOT a fake), then asserts the IDENTICAL shape via
/// [`assert_distributed_e2e_shape`] -- the deterministic pairing. A real Tailscale
/// `Private` endpoint is `permission_required`, so the live attach carries the DT5
/// grant a granted non-loopback control channel would. The always-on gate over the
/// fake tunnel remains the authoritative deterministic evidence; this smoke proves
/// the SAME shape holds when the attach crosses a real reachability boundary.
#[test]
#[ignore = "live distributed smoke: set CAPO_SERVER_RUN_DISTRIBUTED_LIVE=1"]
fn live_distributed_smoke() {
    if std::env::var(LIVE_DISTRIBUTED_ENV).as_deref() != Ok("1") {
        // Not opted in: skip cleanly. The always-on
        // `distributed_e2e_gate_runs_three_roles_over_loopback` test is the paired
        // deterministic assertion of the same shape.
        eprintln!(
            "skipping live distributed smoke: set {LIVE_DISTRIBUTED_ENV}=1 to run it \
             (the reachability preflight uses the CT10 live-Tailscale gates \
              CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT + CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE)"
        );
        return;
    }

    // REACHABILITY PREFLIGHT over a REAL Tailscale endpoint (resolving findings 1 +
    // 2). We do NOT probe `ConnectivityTunnel::fake()` (which hard-returns
    // `reachable: true`, so its skip guard is dead code). Instead we use the in-tree
    // DEFINED CT10 predicate `live_tailscale_smoke_decision`, which reads the live
    // Tailscale env gates and probes the real `tailscale status --json` binary. When
    // the tailnet path is unavailable (binary absent / not logged in / no reachable
    // peer / gate unset) it returns a recorded, secret-free `Skip` reason and we skip
    // CLEANLY rather than fail.
    let endpoint_id = std::env::var(LIVE_TAILNET_ENDPOINT_ENV)
        .unwrap_or_else(|_| LIVE_TAILNET_ENDPOINT_DEFAULT.to_string());
    let live_source = TailscaleStatusSource::new(LiveTailscaleStatusSource::default());
    let peer = match live_tailscale_smoke_decision(&live_source, &endpoint_id) {
        LiveTailscaleSmokeDecision::Run(peer) => peer,
        LiveTailscaleSmokeDecision::Skip { reason } => {
            eprintln!(
                "skipping live distributed smoke: tailnet path unavailable for \
                 endpoint `{endpoint_id}` ({reason})"
            );
            return;
        }
    };
    eprintln!(
        "live distributed smoke: reachable tailnet peer for `{endpoint_id}` \
         (device={}, reachable={})",
        peer.observed_device_id, peer.reachable
    );

    // Build the LIVE attach tunnel: a REAL `ConnectivityTunnel::Tailscale` backed by
    // the live status source (NOT a fake). A Tailscale endpoint resolves at
    // `Private`, so it is `permission_required` -- the live attach carries an active
    // DT5 `ExposureBindGrant` exactly as a real granted non-loopback control channel
    // would (without the grant the attach is `blocked_pending_permission`).
    let grant = ExposureBindGrant::from_active_exposure(
        format!("exposure:dt7-live:{endpoint_id}"),
        "active",
        Some("grant:dt7-live"),
        "connectivity.tailnet.private",
        Some("keychain:capo/dt7-live-tailnet-authkey"),
        ExposureScope::Private,
    )
    .expect("an active Private exposure grant builds for the live attach");
    let e2e_tunnel = E2eTunnel::Tailscale { endpoint_id, grant };

    // Drive the same end-to-end flow over the REAL tunnel and assert the IDENTICAL
    // shape via the shared helper -- the live smoke is never the sole evidence, and
    // the cross-cutting invariants (no secret crosses the leg, exposures are
    // revocable, the server is the single writer) are pinned by the same assertion.
    let shape = capture_distributed_e2e_shape(&e2e_tunnel);
    assert_distributed_e2e_shape(&shape);

    // A secrets-stripped evidence line an operator can attach: every scanned string
    // is already proven secret-free by the shared assertion above.
    eprintln!("--- live distributed smoke (secrets stripped) ---");
    eprintln!(
        "runtime_target_registered_observed={}",
        shape.observed_runtime_target_registered
    );
    eprintln!("runner_health_path={:?}", shape.runner_health_states);
    eprintln!("resumed_sequence={:?}", shape.resumed_sequence);
    eprintln!(
        "replayed_event_occurrences={:?}",
        shape.replayed_event_occurrences
    );
    eprintln!("revoked_control_refused={}", shape.revoked_control_refused);
}
