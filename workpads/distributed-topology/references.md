# Distributed Topology References

## Objective

Record the in-repo files/modules/docs the `distributed-topology` workpad touches or
builds on, plus external references. This workpad is integration-heavy, so the
references are mostly the existing boundaries it composes -- with explicit notes
where a cited capability does NOT yet exist in-tree and is therefore owned by a
prerequisite task (`DT-pre-A` / `DT-pre-B`). Dated claims reflect 2026-06-02.

## Local Architecture Sources

- `workpads/architecture/runtime-tunnel.md`
  - Key facts: THE design this track implements. `RuntimeRunner` (process
    lifecycle) and `ConnectivityTunnel` (reachability) are separate boundaries;
    designed `ConnectivityTunnel` enum `{ LocalLoopback, Ssh, Tailscale, Reverse,
    Fake }` (in-tree TODAY only `Fake`/`LocalLoopback`/`EndpointStub`; `Ssh`/
    `Tailscale` are owned by `DT-pre-A`); `RemoteProcessRunner` / deferred
    `SshRemoteProcessRunner` (the latter owned by `DT-pre-B`); `ExposurePolicy`
    `loopback`/`private`/`public`; `RuntimeProcessRef.last_heartbeat_at` (an
    UNWRITTEN column today; written by `DT-pre-A`); `auth_ref`/`identity_ref` are
    HANDLES never raw credentials; append-first start sequence
    (`runtime.start_requested` before spawn) + recovery
    (`run.recovered`/`run.orphaned`/`run.exited`); `connectivity.*` event family
    (`endpoint_resolved`/`health_changed`/`channel_opened`/`channel_closed`/`exposure_changed`);
    `ResolvedEndpoint.expires_at`; tailnet ACLs are deployment security posture to
    review before remote dogfood; public/Funnel requires explicit permission +
    short-lived exposure + audit events.
- `workpads/architecture/boundaries.md`
  - Key facts: the controller/server owns the loop and is the authoritative writer;
    `AgentAdapter` and `RuntimeRunner` boundaries; the event log is authoritative;
    transport/connectivity stays below the adapter boundary and never owns
    orchestration state (the basis for the two-separate-health-planes decision and
    the single-authoritative-writer injected decision).
- `workpads/architecture/protocol-provider.md`
  - Key facts: Codex/Claude subscription connectors record auth mode only and never
    read credential material; scrub unrelated `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`;
    adapters never own process groups (the runtime does). NOTE: the scrub today
    lives where the adapter spawns the process (server/adapter-side); on a REMOTE
    runner the spawn is on the runner device, so DT5 + `DT-pre-B` add the scrub to
    the RUNNER-SIDE spawn path -- the existing server-side scrub does not cover it.

## Local Cross-Workpad Sources (the boundaries this workpad integrates)

- `workpads/streaming-transport/tasks.md` (+ `knowledge.md`, `references.md`)
  - Key facts: `Subscribe { session_id, from_sequence }` (ST4) over the JSON-RPC
    transport; `events_after(since_sequence)` + broadcast hub; the backlog-to-live
    seam is gap-free/dup-free via the per-stream watermark; ST5 multi-turn thread
    read model; ST7 redaction-on-emit at SERVER egress (NOT the runner hop -- see
    DT3); ST9 checked-in wire-snapshot contract; ST11 `subscribe_tcp`/`SubscribeStream`
    client + restart-resume; ST12 always-on E2E gate paired with an `#[ignore]`d
    live smoke behind `CAPO_SERVER_RUN_STREAMING_LIVE` (the single-`CAPO_SERVER_RUN_*`
    naming DT7 aligns to). DT4a reuses the resume cursor; DT6 reuses the contract
    snapshots; DT7 reuses the E2E-gate-paired-with-live-smoke pattern.
- `workpads/features/remote-runtime.md`
  - Key facts: RR1-RR14 are COMPLETED but deliver only the LOOPBACK-DELEGATING
    `RemoteProcessRunner` + exposure-status/inventory CLI; there is NO
    `SshRemoteProcessRunner` real transport (it was deferred per
    `runtime-tunnel.md`). Therefore DT3's "real transport" prerequisite is owned by
    `DT-pre-B` here, not by this completed track; the loopback remote runner
    contract test (refs/health/interrupt/terminate/recovery, RR1) is the shape
    `DT-pre-B` reuses.
- `workpads/safety-gates` (the grant/permission lifecycle)
  - Key facts: the allow-grant model DT5's `activate_connectivity_exposure` matches
    against (`permission_scope` + subject); the write-lock that keeps the server the
    single writer. NOTE: not started per `TASKS.md`; it is a do-not-start gate
    signal for DT5.
- `connectivity-tunnel` -- DOES NOT EXIST as a workpad
  - Key facts: the draft referenced this as a real prerequisite; `workpads/` has no
    such directory and `TASKS.md` has no such track. The variant + heartbeat +
    reconnect work it would have supplied is OWNED by `DT-pre-A` in this workpad (or
    by a dedicated upstream workpad `DT-pre-A` links to). This entry exists to
    prevent the plan from depending on a phantom.

## Local Implementation Sources

- `crates/capo-cli/src/connectivity.rs`
  - Key facts: the WORKING connectivity exposure lifecycle DT5 integrates (not a
    stub): `expose_connectivity_stub` (loopback/private/public, marks
    `blocked_pending_permission` for private/public),
    `request_connectivity_exposure_approval` (queues a `PermissionApproval`),
    `activate_connectivity_exposure` (requires a matching allow grant via
    `matching_connectivity_exposure_grant`), `revoke_connectivity_exposure`
    (status `revoked`, `reachable=false`), `connectivity_exposure_status`. Emits
    `ConnectivityExposureRequested`/`Changed`/`Revoked`. Owners `runtime_target` /
    `capo_server`; channels control/stdio/logs/dashboard/artifact.
- `crates/capo-cli/src/runtime_target.rs`
  - Key facts: `runtime.target_registered` (`EventKind::RuntimeTargetRegistered`)
    is emitted by a LOCAL CLI command writing the LOCAL store -- NOT a remote
    runner announcing to a server over a connection. DT1 (per DT-D1) builds the
    runner->server announce over JSON-RPC; the draft's "already exists" papered over
    this gap.
- `crates/capo-server/src/transport.rs`
  - Key facts: JSON-RPC 2.0 framing over a persistent connection; concurrent accept
    loop with per-connection timeouts + in-band `Cancel`/`interrupt`;
    loopback-only bind enforcement (`bound_address.ip().is_loopback()` at
    `transport.rs:563`, `connect_loopback`) -- UNCONDITIONAL today. DT5 adds the
    CONDITIONAL non-loopback bind (permit only with an active exposure grant; the
    default branch preserves today's hard rejection); DT1/DT6 preserve loopback as
    the default. `serve_tcp` / `send_tcp` / `subscribe_tcp` / `SubscribeStream`; the
    `RequestHandler::subscribe` seam and `start_event_tail`/`TailHandle` live pump.
- `crates/capo-server/src/event_tail.rs`
  - Key facts: `EventStream` -- the per-stream `delivered_through` watermark, the
    seam dedupe (`record.sequence <= delivered_through` dropped), `next_batch` /
    `recv_batch`, and `delivered_through()` -- the resume cursor DT4a reuses for
    reconnect. (Genuinely supports the watermark-resume half; it does NOT dedupe
    buffered-during-disconnect events -- that is the DT4b spool.)
- `crates/capo-server/src/types.rs`
  - Key facts: `ServerCommand::Subscribe { session_id, from_sequence }`,
    `ServerCommand::ReadThread { from_sequence }`, `SubscriptionBacklog`
    (`next_sequence`), `ServerEvent::from_record` / `redacted_for_egress` (the
    SERVER-EGRESS redaction funnel -- server->client, UPSTREAM of the runner->server
    hop; DT3 adds the runner-side redaction-before-transit on the leg this funnel
    does not cover), the read-only classification of `Subscribe` (no second writer).
- `crates/capo-runtime/src/lib.rs`
  - Key facts: `RuntimeRunner` enum (`LocalProcess`/`RemoteProcess`/`Fake`);
    `RemoteProcessRunner` / `RemoteProcessConfig` (LOOPBACK-DELEGATING today, with
    real `runtime_process_ref` rewriting, `remote_target_resolved` /
    `remote_process_started` / `remote_interrupt_sent` / `remote_terminate_sent`
    events, `health`, and `recover_orphan` -> `remote_recovered`/`remote_orphaned`)
    -- the shape `DT-pre-B`'s `SshRemoteProcessRunner` reuses; `ConnectivityTunnel`
    (today `Fake`/`local_loopback`/`endpoint_stub`; `Ssh`/`Tailscale` arrive via
    `DT-pre-A`), `ExposureScope` (`Loopback`/`Private`/`Public`), `ChannelKind`,
    `ExposureReport`, `resolve_endpoint`, `check_reachability`. NOTE: NO heartbeat
    emission and NO `Ssh`/`Tailscale` variant exist here today.
- `AGENTS.md`
  - Key facts: the SAFETY BOUNDARY this workpad treats as a first-class acceptance
    criterion -- never log API keys / subscription / OAuth tokens / cookies /
    session files / transcripts with secrets; treat subscription-backed agent access
    as a privileged connector; make remote-control capabilities auditable and
    revocable; and DO NOT claim a security property the deployment cannot enforce
    (the basis for scoping runner->server wire confidentiality to the tunnel
    transport rather than a Capo redaction guarantee).
- `TASKS.md`
  - Key facts: the active workpad is `real-turn-loop`; `streaming-transport`,
    `safety-gates`, and the entire connectivity track are unchecked/future. This
    capstone is authored far ahead of its substrate; DT0 records the concrete
    do-not-start completion signals derived from this state.

## External Sources

- Tailscale (tailnet identity, private endpoint resolution, ACLs; Funnel for public
  exposure)
  - Key facts: the reference private-connectivity path for the `Tailscale` tunnel
    variant (`DT-pre-A`); tailnet ACLs are deployment security posture reviewed
    before remote dogfood (per `runtime-tunnel.md`); Funnel/public exposure is out
    of default scope and requires explicit permission + short-lived exposure + audit.
    The live DT7 smoke gates on a generic reachable `Ssh`/`Tailscale` endpoint to
    avoid vendor lock-in.
- SSH (authenticated reachability / command transport for `SshRemoteProcessRunner`)
  - Key facts: host identity checks + key reference storage + failure/reconnect
    events + redacted logs (per `runtime-tunnel.md`); command execution still
    belongs to the runtime runner, not the tunnel; SSH/Tailscale transport
    ENCRYPTION is what provides runner->server wire confidentiality (a transport
    property), complementing Capo's runner-side redaction-before-transit (DT3).
