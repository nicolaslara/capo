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
    Fake }` (in-tree as of 2026-06-02: `Fake`/`Tailscale` -- `Tailscale` LANDED via
    CT3; `Ssh` is NOT in-tree and is explicitly DEFERRED by `connectivity-tunnel`,
    owned by `DT-pre-A` only if the DT track needs it); `RemoteProcessRunner` +
    `SshRemoteProcessRunner` (the latter LANDED via `remote-runtime` RR8, using a
    direct `SshRemoteConfig`; DT-pre-B verifies identity-proof + ordering, DT1/DT3
    own the `ConnectivityTunnel` endpoint-resolution seam); `ExposurePolicy`
    `loopback`/`private`/`public`; `RuntimeProcessRef.last_heartbeat_at` (NOW WRITTEN
    by CT5, no longer the unwritten column the original snapshot named);
    `auth_ref`/`identity_ref` are
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
  - Key facts: RR1-RR14 are COMPLETED. The original snapshot said "NO
    `SshRemoteProcessRunner`"; as of 2026-06-02 RR8 LANDED a real
    `SshRemoteProcessRunner` (`crates/capo-runtime/src/lib.rs:3830`) +
    `SshRemoteConfig` (`:3337`) + `FakeRemoteProcessRunner` for deterministic tests,
    alongside the loopback-delegating `RemoteProcessRunner`. RR8 resolves its target
    via a DIRECT `SshRemoteConfig`, NOT via `ConnectivityTunnel`; so DT3's
    `ConnectivityTunnel`-backed endpoint resolution is a NEW seam (DT1/DT3), while
    DT-pre-B's residual is verifying the identity-proof + append-first ordering. The
    loopback remote runner contract (refs/health/interrupt/terminate/recovery, RR1)
    is the shape DT3 reuses.
- `workpads/safety-gates` (the grant/permission lifecycle)
  - Key facts: the allow-grant model DT5's `activate_connectivity_exposure` matches
    against (`permission_scope` + subject); the write-lock that keeps the server the
    single writer. NOTE: not started per `TASKS.md`; it is a do-not-start gate
    signal for DT5.
- `workpads/connectivity-tunnel/` (CT0-CT10; CT2/CT5/CT6/CT7/CT8/CT9/CT10 `done`)
  - Key facts: at this workpad's AUTHORING this track did not exist, so DT-pre-A was
    created to own the variant + heartbeat + reconnect work. As of 2026-06-02 the
    track EXISTS and has LANDED most of that substrate: CT3 `ConnectivityTunnel::
    Tailscale` (`resolve_endpoint`/`check_reachability`/`open_channel`,
    `crates/capo-runtime/src/lib.rs:5403`); CT5 the heartbeat loop that WRITES
    `RuntimeProcessRef.last_heartbeat_at` + `connectivity.health_changed`
    (`EventKind::ConnectivityHealthChanged`); CT7/CT8 auditable+revocable exposure
    incl. the `channel_closed` payload boolean on `connectivity_exposure_revoked`;
    CT6 anti-sleep; CT9/CT10 the deterministic FakeTunnel suite + the opt-in live
    Tailscale smoke. What CT did NOT deliver (and DT-pre-A still owns if needed):
    a `ConnectivityTunnel::Ssh` reachability variant -- `connectivity-tunnel/
    knowledge.md` EXPLICITLY DEFERS `SshTunnel`; and discrete
    `ConnectivityChannelOpened`/`ConnectivityChannelClosed` event kinds (only a
    payload boolean exists today). The runner->server announce path remains DT1's.

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
    is STILL emitted by a LOCAL CLI command writing the LOCAL store -- NOT a remote
    runner announcing to a server over a connection (verified still true 2026-06-02).
    DT1 (per DT-D1) builds the runner->server announce over JSON-RPC; the draft's
    "already exists" papered over this gap. This is one of the genuinely still-open
    integration seams.
- `crates/capo-server/src/transport.rs`
  - Key facts: JSON-RPC 2.0 framing over a persistent connection; concurrent accept
    loop with per-connection timeouts + in-band `Cancel`/`interrupt`;
    loopback bind enforcement now routes through
    `ExposurePolicy::loopback_default().authorize_socket(bound_address.ip().is_loopback(),
    None)` on BOTH the bind and connect sides (CT1 form, `transport.rs:564-677`),
    NOT a hand-rolled unconditional `is_loopback()` (the original snapshot's claim is
    superseded). DT5 still owns the CONDITIONAL non-loopback bind: thread an active
    exposure grant through as `authorize_socket(.., Some(grant))`; the default
    (no grant) preserves today's hard rejection. DT1/DT6 preserve loopback as
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
    -- the shape RR8's `SshRemoteProcessRunner` (in-tree, `lib.rs:3830`) reuses;
    `ConnectivityTunnel` (as of 2026-06-02 `Fake`/`Tailscale` -- `Tailscale` LANDED
    via CT3 at `lib.rs:5403`; `Ssh` NOT in-tree, deferred by `connectivity-tunnel`),
    `ExposureScope` (`Loopback`/`Private`/`Public`), `ChannelKind`,
    `ExposureReport`, `resolve_endpoint`, `check_reachability`. NOTE (superseding the
    original snapshot): the CT5 heartbeat path NOW WRITES `last_heartbeat_at` and the
    `Tailscale` variant exists; `Ssh` reachability does not.
- `AGENTS.md`
  - Key facts: the SAFETY BOUNDARY this workpad treats as a first-class acceptance
    criterion -- never log API keys / subscription / OAuth tokens / cookies /
    session files / transcripts with secrets; treat subscription-backed agent access
    as a privileged connector; make remote-control capabilities auditable and
    revocable; and DO NOT claim a security property the deployment cannot enforce
    (the basis for scoping runner->server wire confidentiality to the tunnel
    transport rather than a Capo redaction guarantee).
- `TASKS.md`
  - Key facts: the active workpad is still `real-turn-loop`; `streaming-transport`
    and `safety-gates` remain unchecked/future at the top-level queue. The
    connectivity chain (`connectivity-tunnel -> remote-runtime -> distributed-topology`)
    is registered (2026-06-02) as highest-priority; the top-level
    `connectivity-tunnel` / `remote-runtime` checkboxes are still `[ ]`, but their
    LOAD-BEARING tasks have landed in-tree on this branch (CT3/CT5/CT7/CT8 etc. and
    RR8). So DT0's do-not-start gate is now MOSTLY satisfied; the remaining open
    items are `safety-gates`, the `ConnectivityTunnel::Ssh` variant (if needed), and
    the reconnect-leg event-kind decision -- see the Substrate Update in
    `tasks.md`/`knowledge.md`.

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
