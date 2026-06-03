# Operator Guide: Running Capo Across Three Devices (DT8)

This is the operator-facing runbook for the distributed run path. It documents how
to start each of the three Capo roles, point them at each other over a tailnet,
audit and revoke remote control, and what changes (and what does NOT) when you go
distributed.

It is documentation only: every command, flag, and gate below is a surface that
already ships (DT1-DT5/DT7). Nothing here adds new behavior.

> All-local is the DEFAULT. If you run a single `capo` process with no `role` /
> distributed flags, none of this applies: you get server + local runner + client
> in one box over loopback, byte-for-byte as before (DT6). Read the
> "All-local is the default" section last if that is all you need.

## The three roles

A Capo deployment is a DEPLOYMENT TOPOLOGY over the existing single-process
boundaries, not a new distributed-consensus system. There are exactly three roles
(see `knowledge.md`):

- **server / controller** -- owns the turn loop, the authoritative event log
  (`SqliteStateStore`), and the broadcast hub. Binds a listener (loopback by
  default). It is the SINGLE authoritative writer; runner and client hold NO
  authoritative state.
- **remote runner** -- a device that owns agent process lifecycle behind the
  `RuntimeRunner` boundary (`RemoteProcessRunner` / `SshRemoteProcessRunner`). It
  holds no orchestration state; it announces itself to the server and reports
  runtime events + heartbeat.
- **client** -- a device running the CLI that submits commands and tails the event
  log (`subscribe_tcp` / `SubscribeStream`). It holds no authoritative state.

Cross-device resilience comes from the event log + the `Subscribe { from_sequence }`
resume cursor (plus the runner spool for buffered events), NOT from replicating
state.

## Endpoints are named by handle, never by raw address+credential

Every peer is named by a HANDLE:

- a loopback address (`--server-addr 127.0.0.1:7878`) for the all-local / test path, or
- a `connectivity_endpoint_id` (`--server-endpoint <id>` / `--runner-endpoint <id>` /
  `--endpoint <id>`) resolved through `ConnectivityTunnel::resolve_endpoint`
  (Tailscale on the live path; `FakeTunnel` in tests).

There is no flag that accepts an address bundled with a secret. `--server-addr`
with an inlined `user:pass@host` credential is hard-rejected up front
(`RoleConfigError::InlinedCredential`). Credentials are referenced by
`auth_ref` / `identity_ref` handles and never logged raw.

A `private` / `public` endpoint is reported `blocked_pending_permission` until the
DT5 grant path activates it; only a `loopback` endpoint is reachable with no grant.

## 1. Start the server / controller

Validate and resolve the server role config (confirms the topology before you bind):

```
capo role server [--server-addr 127.0.0.1:7878 | --server-endpoint <id>] \
    [--exposure loopback|private|public]
```

This prints the resolved bind, the reachability verdict, whether the keep-alive
planes would be `live` or `inert`, `all_local_default=<bool>`, and the
`next_action`. It does NOT start the long-lived listener -- it is a topology check.

Then start the actual listener:

```
capo server serve [--addr 127.0.0.1:7878] [--max-requests N]
```

**Non-loopback bind requires an active grant and is otherwise HARD-REFUSED.** The
default `server serve` path enforces loopback-only (`require_loopback_address` +
the `bound_address.ip().is_loopback()` guard); the bound transport routes through
`capo_runtime::authorize_server_bind(is_loopback, grant)`, so a non-loopback bind
is permitted ONLY when an ACTIVE `ConnectivityExposure` grant exists (built from an
`active` exposure's audited `capability_grant_id` + non-raw `auth_ref` handle). No
grant => the prior hard rejection, byte-for-byte (DT5). To expose the server
privately over the tailnet, follow the grant flow in section 5 BEFORE binding
non-loopback.

## 2. Start a remote runner on another device

On the runner device, register + ANNOUNCE the runtime target to the server over the
JSON-RPC command transport (the server, single writer, appends
`runtime.target_registered` -- this is NOT a local store write, so the runner can be
on a different device):

```
capo role runner \
    --target <runtime_target_id> --name <name> \
    --runner local-process|remote-process|container \
    --workspace <path> --artifacts <path> [--cwd <path>] \
    [--capability-profile <profile>] [--endpoint <runner_endpoint_id>] \
    (--server-addr <addr> | --server-endpoint <id>) [--exposure ...] \
    [--connect <tunnel-local-dial>]
```

Notes:

- A runner with no server control endpoint is rejected up front
  (`RoleConfigError::MissingPeer`), before any socket.
- `--server-addr` / `--server-endpoint` drive endpoint RESOLUTION and the
  reachability check; `--connect` is the tunnel-local DIAL address the announce
  socket actually opens. For a loopback peer they coincide and must not diverge (a
  mismatch is rejected). For a non-loopback (tunnel) peer there is no loopback
  default, so `--connect <tunnel-local-dial>` is REQUIRED.
- If the resolved server control endpoint is `blocked_pending_permission`, the
  runner refuses to announce until the DT5 grant is active (typed error, before any
  socket).
- A dead server fails LOUDLY (`ConnectionRefused` -> actionable error); the announce
  has no silent in-process fallback, so `announce_source=runner_jsonrpc` is never a
  lie.
- The server re-validates `runner_kind` / `status` against the closed vocabularies
  before appending, so a raw-TCP caller cannot inject an arbitrary string
  (`ServerError::InvalidRuntimeTargetField`).
- The announce is idempotent on `runtime-target:{project}:{target}`: re-announcing
  produces no duplicate event.

**Attach (DT3).** Once announced, the server drives the remote agent process group
through the existing `RuntimeRunner` boundary: the runner's runtime endpoint is
resolved via `ConnectivityTunnel::resolve_endpoint` + `open_channel`, the opened
reachability channel is bound to a `RemoteProcessRunner` (the live path binds RR8's
`SshRemoteChannel`; tests bind `FakeRemoteChannel`). Reachability resolution stays
separate from execution. The remote start is append-first
(`runtime.remote_start_requested` -> `runtime.remote_process_started`); an
unattachable start recovers as an orphan (`runtime.remote_run_orphaned`), never a
duplicate run. The runner proves its target identity by `identity_ref` handle before
launch, never logged raw.

## 3. Start the client on a third device

```
capo role client (--server-addr <addr> | --server-endpoint <id>) [--exposure ...]
```

This validates the client config and resolves the server endpoint into a tail
target. A client with no server endpoint is rejected up front. The client holds no
authoritative state; the actual tail is the existing operator-control path,
parameterized by the resolved endpoint:

```
capo control --connect <addr> [--session <session_id>]
```

The tail uses the `Subscribe { from_sequence }` event-tail contract and resumes from
the watermark the `SubscribeStream` last delivered (`EventStream::delivered_through()`).

**Streaming resume across a drop (DT4a).** On a client reconnect, the client resumes
by issuing a fresh `Subscribe { from_sequence = delivered_through }`. The resumed
tail re-delivers every committed event strictly after the watermark and NONE at or
below it -- no gap, no duplicate. This is durable across a server restart too: after
the server rebuilds read models from the log, a resume from `from_sequence` sees the
identical continuation. A `from_sequence` ahead of the log is rejected as invalid.

## 4. Keep-alive, health states, and reconnect (DT2)

There are TWO SEPARATE health planes, so a connectivity signal never pollutes the
authoritative log:

- **runner <-> server (LOGGED).** The runner heartbeat advances
  `RuntimeProcessRef.last_heartbeat_at`. Health transitions
  `available -> degraded -> unreachable` (matching `ConnectivityEndpoint.status`)
  are recorded as `connectivity.health_changed` events, BECAUSE runner liveness
  affects process truth and is legitimately auditable. The first confirmed miss
  records `degraded`; a continued miss escalates to `unreachable`.
- **client <-> server (EPHEMERAL, NOT LOGGED).** A missed client heartbeat
  transitions an in-memory, server-side connection state to `degraded` and back. It
  is NEVER an authoritative log entry -- client connectivity jitter cannot write into
  the truth log. Observe it via a status query, not the log.

When the runner leg recovers, the reconnect is auditable from the log via a
`connectivity.health_changed` event with `status=available` / `detail=reconnected`
(NO separate `channel_opened` kind -- a single named audit path), and the
`runtime-tunnel.md` recovery sequence is RE-RUN
(`recover_run` -> `run.recovered` / `run.exited`). Keep-alive never fabricates
process liveness: a returned leg over a gone process records `run.exited`.

Heartbeat interval + miss threshold are config with safe defaults. Heartbeat /
health payloads carry NO credential material and NO transcript content -- only
liveness/health + `runtime_process_ref` / `connectivity_endpoint_id` handles.

**Buffered-event reconciliation (DT4b).** Events a runner produced WHILE the
runner<->server leg was down are buffered in a bounded runner-side spool and replayed
on reattach over the SAME JSON-RPC transport (`ServerCommand::ReplayRunnerEvents`).
The single-writer server re-validates each frame and de-duplicates on
`(project_id, idempotency_key)`, so a tailing client sees each replayed event exactly
once, in order, with no duplicate run.

## 5. Audit and revoke remote control (DT5)

Every remote-control capability on the distributed path is a recorded, grant-backed,
revocable `ConnectivityExposure`. The lifecycle, end-to-end:

```
# 1. Declare the exposure (starts blocked_pending_permission):
capo connectivity expose-stub --endpoint <id> --owner-kind <kind> --owner-id <id> \
    --channel <channel> --exposure private|public [--auth-ref <handle>] \
    [--identity-ref <handle>]

# 2. Queue the approval:
capo connectivity request-approval --exposure <exposure_id> [--approval <approval_id>]

# 3. Grant it (the matching allow grant; allow_always is CLI-restricted to read scopes):
capo permission decide --approval <approval_id> --decision allow_once

# 4. Activate (requires the matching grant; moves status -> active):
capo connectivity activate-exposure --exposure <exposure_id>

# 5. Revoke remote control end-to-end:
capo connectivity revoke-exposure --exposure <exposure_id> [--reason <free-text>]
```

After `revoke-exposure` the exposure status is `revoked`, `reachable=false`,
re-activation is REFUSED, and a new control attempt on that channel is refused
(the RR6 `RemoteProcessRunner.revoke_control` / `ensure_control_granted` guarantee).

**Audit at any time:**

```
capo connectivity exposure-status --exposure <exposure_id>
capo connectivity exposure-status --latest [--owner-kind <k>] [--owner-id <id>] [--channel <c>]
```

Every state change is an event (`connectivity.exposure_requested` / `_changed` /
`_revoked`), so replaying the log reconstructs the full lifecycle
(requested -> active -> revoked) identically. No distributed remote control
activates without a grant.

A `public` exposure is high-risk and stays disabled by default: it requires the
explicit grant AND is short-lived/auditable (Funnel/`ReverseTunnel` is out of scope
beyond requiring the grant + audit).

## Safety posture (operator terms)

- A subscription-backed agent (Codex/Claude) on a remote runner is a
  **PRIVILEGED CONNECTOR**, never an ordinary API key. Its `auth_ref` /
  `identity_ref` is referenced by handle only.
- **Credentials are never logged and never cross the tunnel in the clear.** No API
  key, OAuth/subscription token, cookie, session file, or transcript-with-secrets is
  stored or logged raw. The runner-side privileged-connector env scrub
  (`ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` / `CAPO_CONNECTOR_TOKEN` + a
  value-shape net) runs in the RUNNER spawn path before launch, where the
  server-side scrub cannot reach; dropped NAMES (never values) are recorded as an
  audit event.
- **Two redaction seams, both hold.** Runner-side redaction-before-transit scrubs a
  seeded secret BEFORE it leaves the runner; the server-side egress redaction
  (`ServerEvent::from_record` -> `redacted_for_egress`) guards storage and the client
  tail. These are distinct seams.
- **Runner -> server wire confidentiality is a TRANSPORT property**
  (SSH / Tailscale encryption), NOT a Capo redaction guarantee, and is paired with
  runner-side redaction on the leg Capo controls. Capo does not claim a redaction
  guarantee on a leg it cannot enforce; it adds the runner-side pass to the leg it
  can.
- **Connectivity stays separate from execution and from controller state.**
  Reachability resolution (the tunnel) never owns process groups; the runner owns
  process groups; the server owns the authoritative log.
- **Tailnet ACLs are part of the deployment security posture and MUST be reviewed
  before remote dogfood** (per `workpads/architecture/runtime-tunnel.md`).

## All-local is the default (DT6)

With NO role flags and NO distributed config, a single `capo` process is byte-for-byte
equivalent to today: server + local runner + client over loopback, the existing turn
loop, the existing `Subscribe` / thread read model, and the checked-in
`streaming-transport` ST9 contract wire snapshots are UNCHANGED.

The distributed machinery is STRUCTURALLY INERT in the default path: the keep-alive
planes (DT2), the remote runner path (DT3), the buffered-event spool (DT4b), and the
exposure-gating machinery (DT5) are CONSTRUCTED ONLY when a distributed `RoleConfig`
(a non-loopback endpoint) is present. In the single-box path that code is not
entered, so no `connectivity.*` / heartbeat / exposure event type can be produced.
`capo role server` with no flags reports `keep_alive_planes=inert` and
`all_local_default=true`. The loopback-only bind enforcement still hard-refuses a
non-loopback bind absent a grant.

What changes when you go distributed: you add explicit `--server-endpoint` /
`--runner-endpoint` / `--endpoint` handles (or non-loopback exposures), which
constructs the keep-alive planes, the remote attach path, the spool, and the exposure
gating -- all of it grant-backed and auditable as above.

## Reproducing the gate (DT7)

The full path (role config -> remote attach -> stream -> reconnect-resume -> revoke)
is covered by an ALWAYS-ON deterministic three-process E2E gate
(`distributed_e2e_gate_runs_three_roles_over_loopback`, three OS processes over
loopback / `FakeTunnel`, injected fake-clock timing, deterministic drop seam,
per-step timeouts + process cleanup). It lives in the `capo-cli` `server_transport`
integration test, so the whole-workspace run catches it:

```
cargo test --workspace
```

The precise command for just the always-on gate (it is a `capo-cli` integration
test, NOT a `capo-server` test):

```
cargo test -p capo-cli --test server_transport \
    distributed_e2e_gate_runs_three_roles_over_loopback
```

A LIVE cross-device smoke (`live_distributed_smoke`, also in `capo-cli`) runs the
same flow over a real `Tailscale` endpoint across real devices. It is OPT-IN behind
an explicit env gate, `#[ignore]`d, and skips cleanly when the tailnet path is
unavailable. The live tunnel is a real `ConnectivityTunnel::Tailscale` backed by the
`LiveTailscaleStatusSource` (it shells out to `tailscale status --json`), NOT the
deterministic `FakeTunnel`; the optional preflight probes that REAL endpoint and
skips cleanly (never a hard failure) when no reachable peer exists. Because the live
test lives in `capo-cli`, target that crate's test binary:

```
CAPO_SERVER_RUN_DISTRIBUTED_LIVE=1 \
CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT=1 CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE=1 \
    cargo test -p capo-cli --test server_transport -- --ignored live_distributed_smoke
```

(`CAPO_SERVER_RUN_DISTRIBUTED_LIVE` matches the existing `CAPO_SERVER_RUN_*_LIVE`
family: `CAPO_SERVER_RUN_CODEX_LIVE`, `CAPO_SERVER_RUN_STREAMING_LIVE`. The
`CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT` + `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE`
pair is the in-tree defined live-Tailscale skip predicate `live_tailscale_smoke_decision`,
which probes the real `tailscale` binary; both must be `1` to attempt the live
tailnet.) The live smoke is PAIRED with the deterministic gate's shape assertion via
a shared helper, so completion is never operator-attested; its transcript is captured
with secrets stripped.
