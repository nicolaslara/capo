# Distributed Topology Tasks

## Objective

Prove Capo runs as THREE roles -- **server/controller**, **remote runner**, and
**client** -- on DIFFERENT DEVICES end-to-end over a tailnet, while keeping the
all-local single-box path the DEFAULT and a protected regression. This is the
capstone/integration workpad for the connectivity track: it does NOT re-architect
the turn loop, the streaming transport, the permission/grant model, or the goal
model. It integrates the boundaries the other workpads built --
`RuntimeRunner` / `RemoteProcessRunner` and `ConnectivityTunnel` from
`runtime-tunnel.md`, the connectivity exposure lifecycle in
`crates/capo-cli/src/connectivity.rs`, and the `Subscribe { session_id,
from_sequence }` event tail + `EventStream` watermark + `subscribe_tcp` /
`SubscribeStream` resume cursor from `streaming-transport` -- and wires them into a
coherent three-role deployment with keep-alive, resumable streaming, an end-to-end
cross-device smoke, operator docs, and auditable/revocable remote control.

The integration is real but the cross-device proof is deterministic-first: the
three roles are exercised as three separate PROCESSES over loopback / `FakeTunnel`
in the always-on suite, with the real-network (tailnet/SSH) path opt-in behind an
explicit env gate that skips cleanly when unavailable.

## Status

Planned. Capstone workpad of the connectivity track, authored ahead of its
substrate. It MUST NOT START until its substrate exists; DT0 records a hard
do-not-start gate with concrete completion signals (see below). All tasks pending.
`DT0` defines role configuration, routing, scope, the substrate gate, the two
load-bearing mechanism decisions (runner<->server channel, runner-reconnect
reconciliation), and the verification + safety invariant; no DT task begins before
its named prerequisite has actually LANDED in-tree (not merely been named).

### Substrate Reality (ORIGINAL snapshot at authoring; SUPERSEDED -- see Substrate Update below)

NOTE: the bullets in this subsection are the snapshot taken WHEN THIS WORKPAD WAS
AUTHORED. They are retained as a historical record of why DT-pre-A / DT-pre-B were
created. Most of them are NO LONGER TRUE: the `connectivity-tunnel` track (CT0-CT10)
and the `remote-runtime` RR8 SSH transport have since landed. Read the "Substrate
Update" subsection immediately after this list for the CURRENT in-tree state; the
gate and the DT-pre-A / DT-pre-B scopes are governed by that update, not by the
stale bullets here.

The named prerequisites this workpad composes were NOT all present at authoring
time. This was recorded so the plan did not assert capabilities the tree lacked:

- There is NO `connectivity-tunnel` workpad in `workpads/`; the
  `ConnectivityTunnel` enum in `crates/capo-runtime/src/lib.rs` has only
  `Fake`/`LocalLoopback`/`EndpointStub` -- no `Ssh`/`Tailscale` variant.
- There is NO heartbeat emission anywhere; `last_heartbeat_at` exists only as a
  SQLite column in `runtime_process_refs`, never written.
- There is NO `SshRemoteProcessRunner` type; `remote-runtime.md`'s RR1-RR14 are
  completed but deliver only the LOOPBACK-DELEGATING `RemoteProcessRunner`.
- `runtime.target_registered` is emitted by a LOCAL CLI command
  (`crates/capo-cli/src/runtime_target.rs`) writing the local store, NOT by a
  remote runner announcing itself to a server over a connection. No runner->server
  announce path exists in-tree.
- The egress redaction funnel (`ServerEvent::from_record` / `redacted_for_egress`
  in `crates/capo-server/src/types.rs`) runs at the SERVER's subscription egress
  (server->client), NOT on the runner->server hop.
- `transport.rs` enforces loopback on the bound address unconditionally
  (`bound_address.ip().is_loopback()`) and `connect_loopback` on the connect side;
  there is no conditional non-loopback bind path.

Accordingly, the workpad adds explicit UPSTREAM PREREQUISITE TASKS (`DT-pre-A`,
`DT-pre-B`) that must be planned and landed before the integration tasks, and each
DT task's DETERMINISTIC tier is scoped to what the tree (or the named, planned
prerequisite) actually supports; live SSH/tailnet work sits behind the DT7 opt-in
gate.

### Substrate Update (re-verified in-tree, 2026-06-02 after CT/RR landed)

The original snapshot above is superseded. The connectivity + remote-runtime
substrate has LANDED on this branch (off main). Verified in-tree today:

- The `connectivity-tunnel` workpad EXISTS (`workpads/connectivity-tunnel/`) with
  CT2/CT5/CT6/CT7/CT8/CT9/CT10 marked `done` (CT0/CT1/CT3/CT4 still `pending`, but
  the load-bearing tunnel + health + exposure surface they front is in-tree).
- `ConnectivityTunnel::Tailscale(TailscaleTunnel)` IS in-tree
  (`crates/capo-runtime/src/lib.rs:5403`), with `resolve_endpoint` /
  `check_reachability` / `open_channel` / `close_channel`. The enum is now
  `Fake` / `Tailscale` (plus the loopback/endpoint-stub fakes).
  IMPORTANT: `ConnectivityTunnel::Ssh` is NOT in-tree and is NOT owned by any CT
  task -- `connectivity-tunnel/knowledge.md` EXPLICITLY DEFERS `SshTunnel`
  ("This scope DOES NOT add `SshTunnel`..."). The SSH that DID land is the
  `remote-runtime` RR8 `SshRemoteProcessRunner` (an execution-runner transport via
  `SshRemoteConfig`), which is a DIFFERENT boundary from a `ConnectivityTunnel::Ssh`
  reachability variant. See the revised gate and DT-pre-A below.
- Heartbeat health LANDED (CT5): `RuntimeProcessRef.last_heartbeat_at` IS written
  (`crates/capo-state/src/apply.rs`, `schema.rs`, `projections.rs`) and
  `EventKind::ConnectivityHealthChanged` ("connectivity.health_changed") IS emitted
  (`crates/capo-state/src/event.rs:25,283`).
- The reconnect "channel" signal exists ONLY as a `channel_closed` BOOLEAN payload
  field on the `connectivity_exposure_revoked` event
  (`crates/capo-cli/src/connectivity.rs:1152`), NOT as discrete
  `EventKind::ConnectivityChannelOpened` / `ConnectivityChannelClosed` event kinds
  (those do not exist in `event.rs`). DT-pre-A's acceptance criterion is corrected
  accordingly below.
- `SshRemoteProcessRunner` IS in-tree (`crates/capo-runtime/src/lib.rs:3830`, RR8),
  with `SshRemoteConfig` (`:3337`) and the loopback-parity `FakeRemoteProcessRunner`.
- `transport.rs` now binds via `ExposurePolicy::loopback_default().authorize_socket(
  bound_address.ip().is_loopback(), None)` on both the bind and connect sides
  (CT1 form, `crates/capo-server/src/transport.rs:564-677`), NOT a hand-rolled
  unconditional `is_loopback()` rejection. The CONDITIONAL non-loopback bind that
  DT5 still owns is the grant-gated `authorize_socket(..., Some(grant))` path; the
  `ExposurePolicy` seam to thread it through already exists.

What is STILL missing (and therefore still owned by DT/DT-pre tasks):
- `ConnectivityTunnel::Ssh` reachability variant -- unowned upstream; see DT-pre-A.
- The runner->server ANNOUNCE path (`runtime.target_registered` is still only a
  LOCAL CLI store write per `crates/capo-cli/src/runtime_target.rs`) -- DT1.
- Runner-side redaction-before-transit and runner-side env scrub -- DT3 / DT5.
- The buffered-event runner SPOOL + idempotent replay -- DT4b.
- `ConnectivityTunnel`-backed endpoint resolution wiring into the runner attach --
  DT1 / DT3 (RR8 uses a direct `SshRemoteConfig`, not a `ConnectivityTunnel`).

## Feature Set

- Three named roles -- server/controller, remote runner, client -- each with an
  explicit CLI surface to start it and point it at the others over the tailnet
  (`role configuration`), reusing the existing loopback CLI/server seam and the
  JSON-RPC command transport (no second bridge).
- A keep-alive plane spanning the WHOLE path, with TWO SEPARATE health planes:
  runner<->server (a runtime/connectivity event family, because it affects process
  truth) and client<->server (EPHEMERAL server-side connection state, never an
  authoritative log event).
- Streaming resilience: the `Subscribe { from_sequence }` event tail RESUMES
  across a client OR runner CONNECTIVITY DROP with no gap and no duplicate, reusing
  the `EventStream` per-stream watermark (`delivered_through`) and the
  `subscribe_tcp` / `SubscribeStream` resume cursor. Reconciliation of events a
  runner BUFFERED while disconnected is a SEPARATE, explicitly-built mechanism
  (DT4b), not assumed.
- An end-to-end cross-device smoke (opt-in, gated, deterministic-timed) that runs
  the three roles as three processes, plus an all-local DEFAULT regression proving
  the single-box path is byte-for-byte unchanged AND structurally inert.
- Operator docs for the distributed run path (how to start each role, point them
  at each other, audit, and revoke).
- Auditable + revocable remote control end-to-end: every remote-control capability
  is a recorded, grant-backed exposure that can be revoked, and a subscription-backed
  agent (Codex/Claude) is treated as a PRIVILEGED CONNECTOR, never an ordinary API
  key; no credential material is ever logged or crosses the tunnel in the clear.

## DT0 - Workpad, Roles, Routing, Scope, Substrate Gate, Mechanism Decisions, And The Verification + Safety Invariant

Status: done. The three deliverable files (`tasks.md`, `knowledge.md`,
`references.md`) exist, follow the house format, and record every required element:
the three roles, the integration boundary, the hard do-not-start substrate gate, the
differentiated per-task prerequisites, the two resolved load-bearing mechanism
decisions (DT-D1, DT-D2, each with a falsification condition), the verification
invariant (deterministic-first, controllable timing, env gate
`CAPO_SERVER_RUN_DISTRIBUTED_LIVE`, `#[ignore]`, skip-clean, paired), the safety
invariant, and the injected topology-over-boundaries decision. The substrate
snapshot has been re-verified against the tree (see "Substrate Update" below).
Markdown only; `git diff --check` clean.

Scope:

- Name the three roles and their boundaries; record what this workpad integrates
  vs. what earlier workpads own; pin the substrate do-not-start gate, the
  per-task prerequisites, the two load-bearing mechanism decisions, and the
  verification + safety invariant. Markdown only; no code.

Acceptance criteria:

- Define the THREE roles precisely and record them in `knowledge.md`:
  - **server/controller** -- owns the turn loop, the authoritative event log
    (`SqliteStateStore`), and the broadcast hub; binds a listener (loopback by
    default).
  - **remote runner** -- a device that owns agent process lifecycle via
    `RemoteProcessRunner` / `SshRemoteProcessRunner` behind the `RuntimeRunner`
    boundary; holds NO orchestration state; reports runtime events / heartbeat to
    the server.
  - **client** -- a device running the CLI (`subscribe_tcp` / `SubscribeStream`)
    that submits commands and tails the event log; holds NO authoritative state.
- Record the integration boundary: this workpad does NOT change the turn loop
  (`real-turn-loop`), the streaming transport / `Subscribe` contract
  (`streaming-transport`), the permission/grant lifecycle (`safety-gates`), the
  goal model (`goal-autonomy`), the adapters (`depth`), or the web client (web
  agent). It composes `RuntimeRunner`/`ConnectivityTunnel` (`runtime-tunnel.md`),
  the connectivity exposure lifecycle (`crates/capo-cli/src/connectivity.rs`), and
  the event-tail resume cursor into a three-role deployment.
- Record the HARD do-not-start substrate gate (a checkable list of concrete
  completion signals, not soft "prerequisite landed" prose). No DT integration
  task (DT1+) begins until ALL of these are true in-tree:
  - `safety-gates` grant model is merged (the allow-grant + write-lock that keep
    the server the single writer);
  - `streaming-transport` has landed the server listener + `subscribe_tcp` /
    `SubscribeStream` client + `EventStream::delivered_through()` resume cursor +
    the ST9 contract wire snapshots + the ST11 restart-resume property;
  - `DT-pre-A` (below) has landed its substrate. As of 2026-06-02 this is
    PARTIALLY SATISFIED by the `connectivity-tunnel` track: the
    `ConnectivityTunnel::Tailscale` variant (`resolve_endpoint` /
    `check_reachability`), the heartbeat emission path that writes
    `RuntimeProcessRef.last_heartbeat_at`, and the `connectivity.health_changed`
    transition event have all LANDED (CT3/CT5). The gate's reachability requirement
    is therefore "a non-loopback `ConnectivityTunnel` variant landed (Tailscale,
    in-tree)" -- NOT `ConnectivityTunnel::Ssh`, which no CT task owns and which
    `connectivity-tunnel/knowledge.md` explicitly defers. If the DT track needs an
    `Ssh` reachability variant (distinct from the RR8 SSH execution runner), it is a
    NEW, unowned deliverable that DT-pre-A must add with a concrete in-tree path;
    otherwise the DT live path resolves reachability over Tailscale / the RR8 SSH
    runner and `ConnectivityTunnel::Ssh` is OUT OF SCOPE for this gate. The one
    still-open DT-pre-A item is the reconnect-leg auditability signal: today only a
    `channel_closed` BOOLEAN exists on the exposure-revoke event, not discrete
    `ConnectivityChannelOpened` / `ConnectivityChannelClosed` event kinds (see the
    corrected DT-pre-A acceptance criterion);
  - `DT-pre-B` (below) has landed: a real `SshRemoteProcessRunner` transport type
    with a runner-side spawn path. As of 2026-06-02 the `SshRemoteProcessRunner`
    type + `SshRemoteConfig` HAVE LANDED via `remote-runtime` RR8
    (`crates/capo-runtime/src/lib.rs:3830`). DT-pre-B's residual is verifying the
    `identity_ref` handle proof and the append-first start ordering as inputs to
    DT3, and noting that `ConnectivityTunnel`-backed endpoint resolution (vs. the
    direct `SshRemoteConfig` RR8 uses) is the DT1/DT3 seam, not DT-pre-B's.
  - These signals supersede the references to a nonexistent `connectivity-tunnel`
    workpad: the variant+heartbeat work is OWNED by `DT-pre-A` here (or by a
    dedicated upstream workpad that DT-pre-A links to), never assumed to exist.
- Record the DIFFERENTIATED per-task prerequisites (each names a CONCRETE landed
  signal, not a phantom workpad):
  - DT-pre-A (tunnel variants + heartbeat + reconnect events) depends on
    `runtime-tunnel.md` design only; it is the connectivity substrate this workpad
    must build or import.
  - DT-pre-B (`SshRemoteProcessRunner` real transport + runner-side spawn) depends
    on `remote-runtime` RR (loopback runner) + DT-pre-A.
  - DT1 (role config CLI) depends on `streaming-transport` (listener +
    `subscribe_tcp`) + the DT0 resolution of DT-D1 (the runner<->server channel
    decision).
  - DT2 (keep-alive) depends on DT-pre-A + DT1.
  - DT3 (runner attach over tunnel) depends on DT-pre-B + DT1.
  - DT4a (connectivity-drop watermark resume) depends on `streaming-transport`
    (watermark + resume cursor) + DT2.
  - DT4b (runner buffered-event reconciliation: spool + idempotent replay) depends
    on DT3 + the DT0 resolution of DT-D2 (the reconciliation mechanism decision).
  - DT5 (auditable/revocable remote control) depends on the connectivity exposure
    lifecycle (in-tree) + `safety-gates` grant model + DT3.
  - DT6 (all-local default regression) depends on DT1 only; it protects the
    single-box path the whole time.
  - DT7 (cross-device E2E smoke) depends on DT1-DT6 (DT4a + DT4b).
  - DT8 (operator docs) depends on DT1-DT5.
- Record the TWO load-bearing mechanism decisions in `knowledge.md` as RESOLVED
  (each with a falsification condition), because DT1-DT4 cannot be written against
  undecided mechanisms:
  - **DT-D1 -- runner<->server channel**: the runner is "a special client that
    owns processes" and reuses the EXISTING JSON-RPC command transport with a
    runner-role classification; the runner ANNOUNCES itself to the server over
    that transport (the server, as the single writer, appends
    `runtime.target_registered`), rather than a local CLI store write. No second
    protocol/bridge. Falsified if `SshRemoteProcessRunner`'s shape forces a
    distinct runtime control channel; then DT-pre-B records the deviation.
  - **DT-D2 -- runner-reconnect reconciliation**: events a runner produced while
    disconnected are reconciled via a runner-side SPOOL + replay-on-reattach,
    de-duplicated by `runtime.*` idempotency keys appended by the single-writer
    server. Idempotency keys ALONE are insufficient (they dedupe re-probes, not
    buffered output deltas), so the spool is a real DT4b deliverable. Falsified if
    a dogfood trace shows no buffered events are ever produced during a drop (then
    DT4b reduces to the watermark-resume of DT4a and the spool is dropped).
- Record the workpad-wide VERIFICATION invariant: no task completes on operator
  self-attestation alone; deterministic three-process-over-loopback / `FakeTunnel`
  tests with INJECTED/CONTROLLABLE timing (fake clock + deterministic drop seam,
  no wall-clock sleeps) land BEFORE any real-network (tailnet/SSH) path; every live
  cross-device smoke is paired with a deterministic assertion (wire snapshot,
  sequence-continuity check, or restart/replay); the live cross-device path is
  opt-in behind an explicit env gate `CAPO_SERVER_RUN_DISTRIBUTED_LIVE` (matching
  the existing `CAPO_SERVER_RUN_*_LIVE` family -- `CAPO_SERVER_RUN_CODEX_LIVE`,
  `CAPO_SERVER_RUN_STREAMING_LIVE`) plus an optional reachability preflight
  `CAPO_DISTRIBUTED_TAILNET_PREFLIGHT`, and skips cleanly when the tailnet/SSH path
  is unavailable.
- Record the workpad-wide SAFETY invariant (a first-class acceptance criterion
  from AGENTS.md): remote-control capabilities are auditable + revocable; a
  subscription-backed agent (Codex/Claude) is a PRIVILEGED CONNECTOR, not an
  ordinary API key; NEVER log API keys, subscription/OAuth tokens, cookies, session
  files, or transcripts-with-secrets; credentials are referenced by HANDLE
  (`auth_ref` / `identity_ref`), never stored or logged raw; tunnel/connectivity
  concerns stay SEPARATE from agent execution and controller state; and -- the
  precise version -- runner->server confidentiality on the wire is a TRANSPORT
  property (SSH/Tailscale encryption), while Capo redaction guarantees secrets are
  redacted before they are STORED and before they reach the client tail, plus an
  explicit runner-side redaction-before-transit deliverable (DT3) so a seeded
  secret is scrubbed before it leaves the runner.
- Record the injected design decision (see `knowledge.md`): the three roles are a
  DEPLOYMENT TOPOLOGY over the existing single-process boundaries, not a new
  distributed-consensus system. The server remains the SINGLE authoritative writer
  of the event log; the runner and client are non-authoritative; cross-device
  resilience is achieved by the event log + resume cursor (+ the runner spool for
  buffered events), not by replicating state. All-local stays the default.

Verification (deterministic-first, live opt-in gated):

- `workpads/distributed-topology/tasks.md`, `knowledge.md`, and `references.md`
  exist and follow the house format (Objective; numbered tasks with Scope /
  Acceptance / Verification / Dependencies; knowledge decisions + open questions;
  references).
- Scope reviewed against `workpads/architecture/boundaries.md`,
  `workpads/architecture/runtime-tunnel.md`, and the `streaming-transport` /
  `remote-runtime` workpads; the substrate-reality list re-verified in-tree.
- `git diff --check`.

Dependencies: none (planning task). Cross-workpad: reads `remote-runtime`,
`streaming-transport`, `safety-gates`, `runtime-tunnel.md`.

## DT-pre-A - Connectivity Substrate: Tunnel Variants + Heartbeat + Reconnect Events

Status: pending.

Prerequisite: `runtime-tunnel.md` design (in-tree architecture doc).

SUBSTRATE UPDATE (2026-06-02): this task was authored when no `connectivity-tunnel`
workpad existed. That workpad now EXISTS and CT3/CT5 LANDED the bulk of this scope:
`ConnectivityTunnel::Tailscale` (`resolve_endpoint` / `check_reachability`), the
heartbeat loop that WRITES `RuntimeProcessRef.last_heartbeat_at`, and the
`connectivity.health_changed` transition event. DT-pre-A's REMAINING, still-unowned
work is therefore narrow: (1) if the DT track requires a `ConnectivityTunnel::Ssh`
reachability variant distinct from the RR8 SSH execution runner, build it (CT
explicitly defers `SshTunnel`, so no task owns it today); and (2) decide whether the
reconnect-leg auditability needs first-class `ConnectivityChannelOpened` /
`ConnectivityChannelClosed` event kinds, since today only a `channel_closed` boolean
payload field exists on `connectivity_exposure_revoked`. A future implementer must
NOT re-build Tailscale or the heartbeat path -- those are done. This must LAND
before DT2/DT3.

Scope:

- Build the non-loopback connectivity substrate the rest of the workpad spans the
  path with: the `ConnectivityTunnel::Ssh` / `::Tailscale` variants, a heartbeat
  emission path that writes `RuntimeProcessRef.last_heartbeat_at`, and the
  `connectivity.health_changed` / `channel_opened` / `channel_closed` reconnect
  event family. Real substrate; no integration into the three-role deployment yet.

Acceptance criteria:

- Extend the `ConnectivityTunnel` enum (today `Fake`/`LocalLoopback`/`EndpointStub`)
  with `Ssh` and `Tailscale` variants implementing `resolve_endpoint` /
  `check_reachability` per `runtime-tunnel.md`; static-dispatch, swappable, and
  `FakeTunnel`-backed in tests.
- Add a heartbeat emission path that actually WRITES
  `RuntimeProcessRef.last_heartbeat_at` (today an unwritten column) on a configured
  interval, with health states `available` -> `degraded` -> `unreachable` matching
  the `ConnectivityEndpoint.status` vocabulary; each transition is a recorded
  `connectivity.health_changed` event.
- Make a leg recovery auditable from the log. IN-TREE TODAY (2026-06-02): the
  health transition itself is auditable via `EventKind::ConnectivityHealthChanged`
  ("connectivity.health_changed"), and `channel_closed` exists as a BOOLEAN payload
  field on the `connectivity_exposure_revoked` event
  (`crates/capo-cli/src/connectivity.rs:1152`). There are NO discrete
  `EventKind::ConnectivityChannelOpened` / `ConnectivityChannelClosed` event kinds
  in `crates/capo-state/src/event.rs`. DT-pre-A must DECIDE and record which form
  the reconnect-leg signal takes: either (a) `connectivity.health_changed` +
  the `channel_closed` payload boolean are SUFFICIENT for "auditable leg recovery"
  (then this AC is already met and is documented as such), or (b) first-class
  `ConnectivityChannelOpened` / `ConnectivityChannelClosed` event kinds are
  required, in which case they are a NEW deliverable to add to `event.rs`. The AC is
  not "a channel_opened/closed event family exists" -- it is "leg recovery is
  auditable from the log, by a named and in-tree-verified mechanism."
- Heartbeat/health payloads carry NO credential material and NO transcript content
  -- only liveness/health + `runtime_process_ref` / `connectivity_endpoint_id`
  handles.

Verification (deterministic-first, live opt-in gated):

- Deterministic `FakeTunnel` + fake-clock test: a missed heartbeat advances
  `available` -> `degraded` -> `unreachable`, each as a recorded event; a recovered
  heartbeat emits `channel_opened`.
- Deterministic test: a heartbeat/health frame scanned for seeded secret markers
  contains none.
- `cargo fmt`; focused `cargo test -p capo-runtime`.
- Live: deferred to DT7 behind the opt-in gate.
- `git diff --check`.

Dependencies: DT0. Cross-workpad: `runtime-tunnel.md` (tunnel variants, heartbeat,
reconnect events, recovery sequence).

## DT-pre-B - Real `SshRemoteProcessRunner` Transport + Runner-Side Spawn

Status: pending.

Prerequisite: `remote-runtime` RR + DT-pre-A.

SUBSTRATE UPDATE (2026-06-02): this task was authored when no `SshRemoteProcessRunner`
type was in-tree. That type HAS SINCE LANDED via `remote-runtime` RR8 --
`SshRemoteProcessRunner` (`crates/capo-runtime/src/lib.rs:3830`) + `SshRemoteConfig`
(`:3337`) + the loopback-parity `FakeRemoteProcessRunner`. A future implementer must
NOT re-build the SSH transport. DT-pre-B's REMAINING work is verification-and-seam,
not new transport: (1) verify the `identity_ref` handle proof before launch and the
append-first start ordering (`runtime.start_requested` before spawn) hold and are
the inputs DT3 relies on; (2) record that RR8 resolves its endpoint via a DIRECT
`SshRemoteConfig` (host/key), NOT via `ConnectivityTunnel` -- so the
`ConnectivityTunnel`-backed endpoint resolution is the DT1/DT3 seam, explicitly NOT
owned by DT-pre-B. This must LAND/verify before DT3.

Scope:

- Confirm and harden the real remote-process transport type behind the
  `RuntimeRunner` boundary: `SshRemoteProcessRunner` (LANDED via RR8) that spawns +
  reaps a process group on a remote device, reusing the loopback runner's
  event/recovery shape. NOTE: RR8 resolves its target via a DIRECT `SshRemoteConfig`
  (host/key), NOT via a `ConnectivityTunnel::Ssh`/`Tailscale` endpoint -- the
  `ConnectivityTunnel`-backed endpoint resolution is DT1/DT3's seam, not this task's.
  DT-pre-B owns runtime ownership + identity-proof + append-first ordering
  verification only; no loop change, no exposure gating, no tunnel wiring here.

Acceptance criteria:

- Add `SshRemoteProcessRunner` (a `RuntimeRunner` variant) reusing the
  `RemoteProcessConfig` / `runtime_process_ref` rewriting and the existing
  `remote_target_resolved` / `remote_process_started` / `remote_interrupt_sent` /
  `remote_terminate_sent` / `remote_recovered` / `remote_orphaned` events.
- The runner-side spawn path exists ON THE RUNNER DEVICE (the spawn that the
  server-side adapter scrub cannot reach); it is the seam DT3/DT5 hook
  runner-side redaction and the privileged-connector env scrub into.
- The runner proves its target identity before launch via an `identity_ref`
  handle, never logged raw.
- Append-first start ordering (`runtime.start_requested` before spawn) and orphan
  reaping on recovery, per `runtime-tunnel.md`.

Verification (deterministic-first, live opt-in gated):

- Deterministic loopback-backed `SshRemoteProcessRunner` test (fake transport): a
  start records `start_requested` -> `remote_process_started`, an interrupt/
  terminate reaps the group with distinct events, and `recover_orphan` emits
  `remote_recovered` / `remote_orphaned`.
- `cargo fmt`; focused `cargo test -p capo-runtime`.
- Live SSH: deferred to DT7 behind the opt-in gate.
- `git diff --check`.

Dependencies: DT0, DT-pre-A. Cross-workpad: `remote-runtime` (loopback runner),
`runtime-tunnel.md` (start/recovery sequence).

## DT1 - Three-Role Configuration And CLI Surface (server / runner / client)

Status: done. Typed three-role config surface (`crates/capo-cli/src/role_config.rs`)
with exactly `server` / `runner` / `client`, peers named by handle
(`--*-endpoint` id resolved through `ConnectivityTunnel`, or a loopback
`--*-addr`; inlined `user:pass@` credentials rejected). Up-front typed
`RoleConfigError` validation (a runner/client with no server endpoint is rejected
before any socket). The DT-D1 announce path is real JSON-RPC: `capo role runner`
sends `RegisterRuntimeTarget` over `send_tcp` to the live server, which (single
writer) appends `runtime.target_registered`; idempotent on
`runtime-target:{project}:{target}`. Non-loopback exposure is
`blocked_pending_permission` until the DT5 grant. Decisions from the adversarial
review: (1) the announce has NO in-process fallback -- a dead server fails loudly
(`ConnectionRefused` -> actionable error), so `announce_source=runner_jsonrpc` is
never a lie; (6) `--connect` that disagrees with a loopback `--server-addr` is
rejected up front (the two flags have distinct documented roles: resolution vs
tunnel-local dial). Tests landed (`crates/capo-cli/tests/server_transport/role_topology.rs`):
the three-process-over-loopback announce/tail test now uses a DISTINCT runner
state root and asserts the runner wrote NO local store (proving the announce rode
TCP), plus a `capo role client` subprocess reporting
`server_tail_reachability=reachable`; a dead-server loud-failure test; a
`--connect`/`--server-addr` mismatch rejection test; and an idempotent
re-announce test (same sequence, single tail occurrence). Residual: the
non-loopback dial itself rides the DT5-granted tunnel (DT5); the all-local
default-inertness regression is proven by DT6.

Prerequisite: `streaming-transport` (server listener + `subscribe_tcp` client
seam) + DT0's DT-D1 resolution (runner<->server channel = reuse JSON-RPC).

Scope:

- Give each of the three roles an explicit, documented way to start and to point
  at the others over the tailnet, reusing the existing server/CLI seam and the
  JSON-RPC command transport. No new transport protocol; no second bridge. This
  task IMPLEMENTS the DT-D1 decision: the runner announces itself to the server
  over JSON-RPC, closing the in-tree gap that `runtime.target_registered` is
  currently only a local CLI store write.

Acceptance criteria:

- Add a typed role configuration surface (`RoleConfig` or equivalent) with exactly
  three roles: `server`, `runner`, `client`. Each role resolves its peer endpoints
  by `connectivity_endpoint_id` (or a `--server-endpoint` / `--runner-endpoint`
  flag that resolves to one), never by an inlined raw address+credential.
- Add CLI subcommands to start each role and point it at the others:
  - server: bind a listener at a resolved endpoint (loopback by default; a
    non-loopback bind requires the DT5 grant path -- see DT5 and the conditional
    bind below);
  - runner: register as a `runtime_target` with a `connectivity_endpoint_id`,
    resolve the server control endpoint, and ANNOUNCE itself to the server over the
    JSON-RPC transport so the server (single writer) appends
    `runtime.target_registered` to the authoritative log -- NOT a local store write
    (this is the DT-D1 seam, new code, not "already exists");
  - client: resolve the server endpoint and open a `subscribe_tcp` tail +
    command channel (the existing CLI path), now parameterized by endpoint
    rather than hardcoded loopback.
- The three role configs are validated up front: a runner config that names no
  server control endpoint, or a client config that names no server endpoint, is
  rejected with a typed error before any connection is attempted.
- Endpoint resolution goes through `ConnectivityTunnel::resolve_endpoint`
  (loopback / `FakeTunnel` in tests; `Ssh`/`Tailscale` from DT-pre-A); a `private`
  or `public` exposure requires the DT5 grant path and is
  `blocked_pending_permission` until granted (reuse the exact
  `expose_connectivity_stub` -> request-approval -> activate flow in
  `connectivity.rs`).
- The default with no role flags is unchanged: a single process behaves exactly as
  today (server + local runner + client in one box over loopback) -- proven by DT6.

Verification (deterministic-first, live opt-in gated):

- Deterministic test: each role config validates/rejects correctly (missing peer
  endpoint rejected; loopback endpoint accepted; private endpoint marked
  `blocked_pending_permission`).
- Deterministic three-process-over-loopback test: a server process, a runner
  process that ANNOUNCES a `runtime_target` over JSON-RPC, and a client process
  (`subscribe_tcp`) all start from role config and the client sees the runner's
  server-appended `runtime.target_registered` event in its tail.
- `cargo fmt`; focused `cargo test -p capo-cli -p capo-server -p capo-runtime`.
- `git diff --check`.

Dependencies: DT0. Cross-workpad: `streaming-transport` (listener,
`subscribe_tcp`), `runtime-tunnel.md` (`RuntimeTarget`, `ConnectivityEndpoint`).

## DT2 - Keep-Alive Across The Whole Path (two separate health planes)

Status: pending.

Prerequisite: DT-pre-A (heartbeat / `last_heartbeat_at` / reconnect events) + DT1.

Scope:

- A heartbeat + health-transition + reconnect plane covering BOTH legs of the
  path, with the two legs kept on SEPARATE planes so a connectivity signal never
  pollutes the authoritative log. Pure liveness; no state replication.

Acceptance criteria:

- Define TWO distinct health planes (resolving review finding 7 -- connectivity
  must not leak into authoritative state):
  - **runner<->server (LOGGED)**: the runner heartbeat advances
    `RuntimeProcessRef.last_heartbeat_at` and the server records a
    `runtime.health_changed` / `connectivity.health_changed` transition on miss,
    BECAUSE runner liveness affects process truth and is legitimately auditable.
  - **client<->server (EPHEMERAL, NOT LOGGED)**: a missed client heartbeat
    transitions an in-memory, server-side connection state to `degraded` and back;
    it is NEVER an authoritative event-log entry. Client connectivity jitter must
    not be able to write into the truth log (this also protects DT6 byte-for-byte).
- Define heartbeat interval + miss threshold as config (safe defaults) and the
  health states `available` -> `degraded` -> `unreachable` matching
  `ConnectivityEndpoint.status`. Every LOGGED transition is a recorded event,
  never a silent flag; ephemeral client transitions are observable via a status
  query, not the log.
- Implement reconnect: when the runner leg recovers, RECORD the reconnect as an
  auditable event and re-run the `runtime-tunnel.md` recovery sequence --
  `RuntimeRunner.health(...)` / `recover_orphan(...)` / `recover_run(...)` ->
  `run.recovered` / `run.orphaned` / `run.exited`. Keep-alive NEVER fabricates
  process liveness; a heartbeat is a liveness signal, not proof the process exists.
  - DECIDED (DT2, resolving the DT-pre-A open question and review finding 2): the
    reconnect signal takes form (a) -- `EventKind::ConnectivityHealthChanged`
    (`"connectivity.health_changed"`) with `status="available"` and
    `detail="reconnected"`. NO new `ConnectivityChannelOpened` event kind is added.
    Rationale: the in-tree `connectivity.health_changed` family already names every
    health edge auditably (`initial`/`degraded`/`lost`/`stalled`/`reconnected`),
    adding a parallel `channel_opened` kind would create a SECOND audit path for the
    same fact (the review's own "no parallel/duplicate event path" invariant), and
    the recovery re-run -- not a discrete open event -- is the load-bearing
    reconnect behavior. The reconnect IS auditable from the log via this named,
    in-tree-verified mechanism, which is exactly what the AC requires.
  - WIRED (resolving review finding 3): `RunnerBeat.must_rerun_recovery` is acted
    on, not merely flagged. `RunnerServerPlane::beat()` ->
    `must_rerun_recovery=true` -> `RemoteProcessRunner::recover_run(...)` ->
    `run.recovered` / `run.exited`, proven end-to-end by
    `dt2_runner_reconnect_drives_recover_run_and_emits_run_recovered` and
    `dt2_runner_reconnect_to_gone_remote_records_run_exited_not_fabricated_liveness`
    (a returned LEG over a GONE process records `run.exited`, never fabricated
    liveness).
- Heartbeat payloads carry NO credential material and NO transcript content -- only
  liveness/health and the `runtime_process_ref` / `connectivity_endpoint_id`
  handles.
- All-local default: in the single-box deployment NEITHER plane is instantiated
  (the heartbeat machinery is constructed only when a non-loopback RoleConfig is
  present -- see DT6), so it adds no events and no frames that would break the DT6
  byte-for-byte regression.

Verification (deterministic-first, live opt-in gated):

- Deterministic `FakeTunnel` + fake-clock test: a missed runner heartbeat
  transitions LOGGED health `available` -> `degraded` -> `unreachable` and records
  each transition; a recovered heartbeat emits the reconnect event and re-runs the
  runner recovery sequence. DONE: the LOGGED `RunnerServerPlane` owns the three-state
  vocabulary layered on the CT5 binary probe -- the FIRST confirmed miss records
  `degraded` and a CONTINUED miss escalates to `unreachable`, each its own
  `connectivity.health_changed` event (resolving review finding 1; the runner leg no
  longer skips `degraded`). Covered by
  `runner_leg_logs_three_state_transitions_and_reruns_recovery_on_reconnect`,
  `runner_degraded_recovers_directly_to_available_and_reruns_recovery`, and
  `runner_stall_past_deadline_logs_degraded_with_stalled_cause`. The recovery re-run
  is wired (see the WIRED note above).
- Deterministic test: a missed client heartbeat degrades the EPHEMERAL connection
  state and adds NO authoritative event. SCOPED (resolving review finding 4): DT2
  proves this STRUCTURALLY -- the `ClientServerPlane` API has no path to produce a
  logged event (`observe_miss`/`observe_beat` return only a changed-bool), proven by
  `client_jitter_produces_no_logged_event_type`. The FULL byte-identical event-log
  comparison (start a real server, jitter the client plane, snapshot-compare the log
  against a no-jitter run) is the DT6 byte-for-byte regression and is DEFERRED to DT6
  by design; DT2's claim is the structural inertness, not the byte-equality snapshot.
- Deterministic test: a heartbeat frame carries no credential/transcript fields
  (`heartbeat_event_carries_no_credentials`).
- Server-side validation (resolving review finding 6): the `RegisterRuntimeTarget`
  server handler re-validates `runner_kind` / `status` against the closed
  vocabularies BEFORE appending, so a raw-TCP JSON-RPC caller bypassing the CLI
  cannot inject an arbitrary string into `runtime.target_registered`
  (`ServerError::InvalidRuntimeTargetField`).
- Test-port TOCTOU (resolving review finding 5): the dead-server announce test
  reserves + releases the loopback port at the call site (`ReservedPort`), so the
  connect hits a closed port and fails FAST with ConnectionRefused while keeping the
  TOCTOU window minimal. (Holding an open-but-silent listener is deliberately NOT
  done: it would accept the connection and the announce would block forever reading
  a reply that never comes -- a hang, not a loud failure.)
- `cargo fmt`; focused `cargo test -p capo-runtime -p capo-server`.
- `git diff --check`.

Dependencies: DT1, DT-pre-A. Cross-workpad: `runtime-tunnel.md` (recovery
sequence, `last_heartbeat_at`), `streaming-transport` (subscription liveness).

## DT3 - Remote Runner Attach Over The Tunnel (runner on a different device)

Status: done. The DT1/DT3 seam landed as `RemoteRunnerAttach`
(`crates/capo-runtime/src/remote_attach.rs`): it resolves the runner's runtime
endpoint via `ConnectivityTunnel::resolve_endpoint` + `open_channel` and binds the
opened reachability channel to a `RemoteProcessRunner`, so the server drives a
remote process group through the EXISTING `RuntimeRunner` boundary -- no loop
change, no new transport protocol. The transport is INJECTED (deterministic suite
binds `FakeRemoteChannel`; the live DT7 path binds RR8's `SshRemoteChannel`), so
reachability resolution (connectivity boundary) stays separate from execution
(runner owns the process group), closing the gap that RR8 used a DIRECT
`SshRemoteConfig` rather than a tunnel-resolved channel. A resolution that requires
permission propagates the tunnel's typed error, so a non-loopback attach is
`blocked_pending_permission` until the DT5 grant (never a silent open). The
append-first start sequence (`runtime.remote_start_requested` ->
`runtime.remote_process_started`), orphan-on-unattachable recovery
(`runtime.remote_run_orphaned`, never a relaunch), interrupt/terminate reaping, and
identity-by-handle proof are the in-tree RR8 surfaces, reused unchanged. The
runner-side redaction-before-transit pass is the existing
`RemoteProcessRunner::stream_output` (it redacts each delta BEFORE it becomes an
event/artifact, on the leg Capo controls); the server-side egress backstop
(`ServerEvent::from_record` -> `redacted_for_egress`) is a DISTINCT second seam for
the client tail; runner->server wire confidentiality is documented as a TRANSPORT
property (SSH/Tailscale encryption), not a Capo redaction property. Tests:
`crates/capo-runtime/src/lib.rs` `dt3_server_drives_remote_process_through_tunnel_resolved_runner`,
`dt3_remote_start_unattachable_recovers_as_orphan_not_duplicate_run`, and
`dt3_runner_side_redaction_scrubs_secret_before_it_crosses_the_tunnel`; plus
`crates/capo-server/src/tests/dt3.rs`
`server_drives_remote_process_through_tunnel_resolved_runner` and
`dt3_two_redaction_seams_both_hold_runner_side_and_server_egress` (the two distinct
seams asserted separately from the server crate). All deterministic over
`FakeTunnel` + `FakeRemoteChannel`, no network. Residual: the actual non-loopback
dial riding the DT5-granted tunnel (DT5); the buffered-event runner spool (DT4b).

Prerequisite: DT-pre-B (`SshRemoteProcessRunner` real transport + runner-side
spawn) + DT1.

Scope:

- Wire the server's turn loop to drive an agent process on the REMOTE runner
  through `RemoteProcessRunner` / `SshRemoteProcessRunner` resolved over the
  tunnel, keeping runtime ownership on the runner and orchestration state on the
  server. Integration of the existing remote runner with a real transport; the
  loop is unchanged. Includes the RUNNER-SIDE redaction-before-transit deliverable
  (resolving review finding 6).

Acceptance criteria:

- The server resolves the runner's runtime endpoint via `ConnectivityTunnel`
  (`FakeTunnel` in tests; `Ssh`/`Tailscale` live) and dispatches a
  `RuntimeRequest` to the runner's `SshRemoteProcessRunner`, which owns the process
  group (adapters/loop never own remote process groups, per `runtime-tunnel.md`).
- The remote start path is append-first and recoverable exactly as
  `runtime-tunnel.md` specifies: `runtime.start_requested` (with idempotency key)
  before spawn; on success `runtime.process_started` /
  `runtime.remote_process_started`; on failure `runtime.process_start_failed`; an
  orphan with a live process but no `process_started` becomes `run.orphaned` on
  recovery. The existing `prepend_remote_events` / `recover_orphan` events are the
  basis.
- Redaction is PLACED PRECISELY (finding 6): a redaction-on-emit pass runs ON THE
  RUNNER before output crosses the tunnel (new runner-side deliverable + test), so
  a seeded secret is scrubbed before transit; the server's existing
  `redacted_for_egress` continues to guard storage and the client tail; and the
  runner->server hop's wire confidentiality is documented as a TRANSPORT property
  (SSH/Tailscale encryption), not a Capo redaction property. Capo does NOT claim a
  redaction guarantee on a leg it cannot enforce; it adds the runner-side pass to
  the leg it can.
- The runner proves its target identity before launch (`identity_ref`), referenced
  by handle, never logged raw.
- Interrupt / terminate / kill on a remote run produce distinct recorded events and
  reap the remote process group (reuse `remote_interrupt_sent` /
  `remote_terminate_sent` + runner-side reaping).

Verification (deterministic-first, live opt-in gated):

- Deterministic loopback/`FakeTunnel` `SshRemoteProcessRunner` test: a turn
  dispatched to the remote runner records `start_requested` ->
  `remote_process_started`, streams output back, finalizes a turn -- proving the
  server drives a remote process through the existing loop with no loop change.
- Deterministic test: a remote start whose `process_started` append fails leaves a
  recoverable orphan (`run.orphaned`), not a duplicate run.
- Deterministic runner-side redaction test: a seeded secret in remote output is
  scrubbed by the RUNNER-SIDE pass before it crosses the (fake) tunnel AND the
  server-side egress redaction still holds for the client tail (two distinct
  assertions, two distinct seams).
- `cargo fmt`; focused `cargo test -p capo-runtime -p capo-server`.
- Live: deferred to DT7 behind the opt-in gate.
- `git diff --check`.

Dependencies: DT1, DT-pre-B. Cross-workpad: `remote-runtime` /
`runtime-tunnel.md` (start/recovery), `streaming-transport` (redaction-on-emit
funnel shape).

## DT4a - Streaming Resume Across A Connectivity Drop (no gap, no dupe)

Status: pending.

Prerequisite: `streaming-transport` (`EventStream` watermark + `subscribe_tcp`
resume cursor) + DT2.

Scope:

- Prove the `Subscribe { from_sequence }` event tail RESUMES across a transport
  drop on either the client or the runner leg with no gap and no duplicate,
  reusing the existing sequence cursor. This is the watermark-resume guarantee the
  tree genuinely supports today; the SEPARATE buffered-event reconciliation is
  DT4b. No new transport.

Acceptance criteria:

- On a client reconnect, the client resumes its tail by issuing a fresh
  `Subscribe { from_sequence = delivered_through }` from the watermark its
  `SubscribeStream` last delivered (`EventStream::delivered_through()` already
  exposes this). The resumed tail re-delivers every committed event strictly after
  the watermark and re-delivers NONE at or below it -- the exact seam guarantee
  `event_tail.rs` enforces, now across a real disconnect.
- The reconnect is durable across a server restart too: after the server restarts
  and rebuilds read models from the event log, a client resuming from
  `from_sequence` sees the identical continuation (restart/replay), reusing the
  `streaming-transport` ST11 restart-resume property.
- Define and document the resume contract for the THREE-role case: the server's
  event log is the single source of continuity; client and runner both resume by
  sequence watermark; neither holds authoritative state to lose. A reconnect that
  presents a stale `from_sequence` is served correctly (re-delivers the backlog
  after that point); a `from_sequence` ahead of the log is rejected as invalid.
- No streaming-by-re-serializing-a-snapshot: resume rides the discrete
  committed-event broadcast, not a periodic read-model dump.
- Scope honesty: this task covers resume of events ALREADY COMMITTED to the
  server's log; events a runner BUFFERED while disconnected are out of scope here
  and are reconciled by DT4b.

Verification (deterministic-first, live opt-in gated):

- Deterministic sequence-continuity test (deterministic drop seam, not wall-clock):
  a client tail is force-dropped mid-stream, reconnects with
  `from_sequence = delivered_through`, and the union of pre-drop + post-resume
  events equals the full committed sequence with NO gap and NO duplicate (strictly
  increasing, contiguous).
- Deterministic restart/replay test: the server restarts between drop and resume;
  the rebuilt log yields the identical continuation from `from_sequence`.
- `cargo fmt`; focused `cargo test -p capo-server -p capo-state`.
- `git diff --check`.

Dependencies: DT2. Cross-workpad: `streaming-transport` (`EventStream`,
`subscribe_tcp`/`SubscribeStream`, restart-resume).

## DT4b - Runner Buffered-Event Reconciliation (spool + idempotent replay)

Status: done. The DT-D2 mechanism landed as a runner-side spool
(`crates/capo-runtime/src/runner_spool.rs`) PLUS a production replay-on-reattach
SEAM over the existing JSON-RPC command transport (resolving review findings 1, 2,
4 -- the replay is no longer a test-only `state_for_test()` backdoor). `RunnerEventSpool`
buffers the `runtime.*` events a runner produced WHILE the runner<->server leg is down
(`mark_disconnected` -> `offer`), bounded (oldest-dropped, recorded via
`SpoolAdmission::BufferedEvictingOldest` + `evicted_count`), and replays them in
production order on reattach (`drain_for_replay`). Each buffered event is a
`SpooledRuntimeEvent` carrying a TYPED `EventKind` (never a raw kind string) + the
stable `runtime.*` idempotency key + the redacted payload.

The reconnecting runner converts each drained `SpooledRuntimeEvent` into a wire
`RunnerReplayFrame` (`From<&SpooledRuntimeEvent>`) and submits a
`ServerCommand::ReplayRunnerEvents { frames }` over the SAME transport DT1 uses;
`CapoServer::handle()` routes it to `handle_replay_runner_events`, the single-writer
append path. Like `RegisterRuntimeTarget`, the server RE-VALIDATES every frame at
this seam BEFORE appending (a replay can arrive from a remote runner speaking
JSON-RPC directly): the wire `kind` must resolve through `EventKind::from_wire` to a
`runtime.remote_*` kind, and `redaction_state` must be a persistable
(`safe`/`redacted`) classification -- a non-runtime/unknown kind or an unscrubbed
frame is refused (`ServerError::InvalidRunnerReplayFrame`), never committed. The
server stays the single writer and de-duplicates the replay on
`(project_id, idempotency_key)` via the in-tree `SqliteStateStore::append_event`
dedupe, so a reattach that re-sends an already-appended event returns the existing
sequence (a no-op) -- exactly-once -- and `append_event` fans the committed event to
live subscribers, so a tailing client sees each replayed event once. While CONNECTED
the spool buffers nothing (`offer` returns `None`), so it is inert in the steady
state and, transitively, in the all-local default where the leg never disconnects
(DT6). Credential safety: DT3 redacts before an event reaches the spool, AND the
spool defensively re-scans every payload on insert (`scan_credential_shapes`), so a
spooled frame can never be where a secret leaks across the reconnect; the server
seam re-checks the classification as a second gate.

Tests: `crates/capo-runtime/src/runner_spool.rs` unit suite (connected-inert,
buffered-in-order, bounded-evict-oldest, secret-scrub, idempotency-key survives onto
the NewEvent) + `crates/capo-server/src/tests/dt4b.rs`, which now drive the replay
END-TO-END through the production seam (`ServerCommand::ReplayRunnerEvents` ->
`CapoServer::handle()`), NOT `state_for_test()`:
`spooled_runner_events_replay_through_the_single_writer_exactly_once` (runner
produces -> spools -> reconnects -> submits over the command transport -> single
writer appends -> tailing client sees each once, in order),
`re_replaying_an_already_appended_event_is_a_no_op_and_the_sequence_stays_contiguous`
(both replays through `handle()`; the second returns identical sequences),
`a_replayed_spooled_frame_carries_no_seeded_secret`, and two seam-validation tests
(`the_replay_seam_refuses_a_non_runtime_kind_before_appending`,
`the_replay_seam_refuses_an_unscrubbed_redaction_classification`) proving a forbidden
kind / unscrubbed frame is rejected before any append and never reaches the log. The
command also round-trips through the real JSON-RPC codec (the `contract` schema
suite). All deterministic: the disconnect/reconnect is the spool's own state
transition, NOT a wall-clock drop.

Falsification (DT-D2, review finding 3): the spool is retained rather than degraded
to DT4a's watermark resume. The falsification EVIDENCE is the ABSENCE of a trace
showing a drop produces zero buffered events; no positive production/dogfood trace
has been collected, so this rests on absence-of-evidence (the weakest tier the DT-D2
text allows), not a positive trace. Live smoke (review finding 6): DT4b has
deterministic assertions only; the live cross-device E2E smoke is deferred to DT7
(pending) -- the live replay path has NOT been exercised here.

Prerequisite: DT3 + DT0's DT-D2 resolution (reconciliation = runner spool +
idempotent replay).

Scope:

- BUILD the mechanism by which runtime events a runner produced WHILE DISCONNECTED
  are reconciled into the authoritative log on reattach without creating duplicate
  runs. This is a new capability the tree does not have today (resolving review
  finding 5); it is NOT assumed to fall out of existing idempotency keys.

Acceptance criteria:

- Add a runner-side SPOOL that buffers `runtime.*` events produced during a
  server-leg disconnect, and a replay-on-reattach path that submits them to the
  server (single writer) for append.
- The server de-duplicates replayed events by `runtime.*` idempotency keys so a
  reattach that re-sends already-appended events produces NO duplicate run and NO
  duplicate event; a client tailing the log sees each exactly once.
- The spool is bounded and never holds credential material or transcripts-with-secrets
  (the runner-side redaction of DT3 applies before spooling).
- Falsification hook (DT-D2): if a dogfood trace proves no buffered events are ever
  produced during a drop, this task degrades to documenting that DT4a's watermark
  resume is sufficient and the spool is removed; that outcome is recorded, not
  silently assumed.

Verification (deterministic-first, live opt-in gated):

- Deterministic runner-reconnect test: events produced during a runner disconnect
  are spooled, replayed on reattach, and appear EXACTLY ONCE in a tailing client's
  stream (idempotency-key dedupe), with no duplicate run.
- Deterministic test: a replay that re-sends an already-appended event is a no-op
  (dedupe), and the resulting sequence is contiguous.
- Deterministic test: a spooled frame contains no seeded secret marker.
- `cargo fmt`; focused `cargo test -p capo-runtime -p capo-server -p capo-state`.
- `git diff --check`.

Dependencies: DT3. Cross-workpad: `runtime-tunnel.md` (idempotency keys on
`runtime.*`), `streaming-transport` (single-writer append + tail).

## DT5 - Auditable + Revocable Remote Control End-To-End

Status: done. DT5 INTEGRATES the in-tree exposure lifecycle + grant model + the RR6
runner revocation into a checkable safety boundary; it adds the two genuinely-new
deliverables the section names and does NOT reimplement the permission engine.

(1) CONDITIONAL NON-LOOPBACK BIND (review finding 12). The transport bind guard
(`crates/capo-server/src/transport.rs`) now routes through
`capo_runtime::authorize_server_bind(is_loopback, grant)`: with NO grant (the
all-local default, `serve_tcp_with_handler` -> `..._and_grant(None)`) it is the
loopback-only policy's HARD rejection, byte-for-byte the prior behavior; with an
ACTIVE `capo_runtime::ExposureBindGrant` a non-loopback bind is permitted under the
grant's promoted ceiling + `auth_ref` handle. `ExposureBindGrant::from_active_exposure`
is constructed ONLY from an `active` exposure's audited fields (status must be
`active`, a non-empty `capability_grant_id` + non-raw `auth_ref` handle required) so
a `blocked_pending_permission` / `revoked` exposure, a missing grant, a missing
handle, or a RAW credential in the handle field all fail closed. It carries only
handles + a provenance label, never a raw credential.

(2) RUNNER-SIDE PRIVILEGED-CONNECTOR ENV SCRUB (review finding 11).
`capo_runtime::scrub_privileged_connector_env` (the name set in
`PRIVILEGED_CONNECTOR_ENV_VARS`: `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`/...
+ `CAPO_CONNECTOR_TOKEN`, case-insensitive, plus a defense-in-depth value-shape net)
runs in `RemoteProcessRunner::start_process` on the RUNNER spawn path BEFORE
`transport.launch`, where the server-side adapter scrub cannot reach. The dropped
NAMES (never values) are recorded as a `runtime.remote_target_resolved` /
`env_scrubbed` audit event, so the scrub is auditable from the trail.

The rest of the DT5 surface is the EXISTING in-tree lifecycle, now proven as a DT5
acceptance criterion end-to-end: `expose-stub` (private,
`blocked_pending_permission`) -> `request-approval` -> `permission decide
--decision allow_once` (the matching allow grant; `allow_always` is CLI-restricted
to read scopes) -> `activate-exposure` (`active`) -> `revoke-exposure` (`revoked`,
`reachable=false`, re-activation REFUSED). The RR6 `RemoteProcessRunner.revoke_control`
/ `ensure_control_granted` (in-tree) is the runner-side "a revoked grant forbids new
execution" guarantee, reused unchanged.

Tests (all deterministic; no live tailnet/SSH, no wall clock):
`crates/capo-runtime/src/lib.rs` `dt5_authorize_server_bind_rejects_non_loopback_without_a_grant_and_allows_with_one`,
`dt5_exposure_bind_grant_refuses_to_build_from_a_non_active_or_handleless_exposure`,
`dt5_scrub_privileged_connector_env_drops_known_vars_and_credential_shaped_values`,
`dt5_runner_side_spawn_scrubs_privileged_connector_env_before_launch` (asserts the
scrubbed launch env via the transport's `last_launched_request`, plus the audit
event); `crates/capo-server/src/tests/dt5.rs` proves the conditional bind through the
REAL transport guard (`serve_tcp_with_handler_and_grant`, `0.0.0.0:0` non-loopback,
`max_connections=0` so it never blocks): loopback+no-grant accepted, non-loopback+no-grant
REFUSED, non-loopback+active-grant permitted; `crates/capo-cli/tests/server_transport/dt5_exposure.rs`
proves the lifecycle end-to-end over the CLI (blocked-until-grant-then-active,
revoke-makes-unreachable-and-refuses-reactivation, and a replay/audit test that two
independent fresh-store reads reconstruct the identical terminal `revoked` lifecycle).
Residual: the actual non-loopback dial riding the DT5-granted tunnel on a real device
is the DT7 live opt-in smoke (still pending); a `public` exposure stays disabled by
default (CT8 in-tree).

Prerequisite: connectivity exposure lifecycle (in-tree,
`crates/capo-cli/src/connectivity.rs`) + `safety-gates` grant model + DT3.

Scope:

- Make every remote-control capability on the distributed path a recorded,
  grant-backed, revocable exposure, and make the subscription-backed agent a
  privileged connector. This is the AGENTS.md safety boundary as a first-class,
  checkable acceptance criterion. Integration of the existing exposure lifecycle;
  no new permission engine. Includes the CONDITIONAL non-loopback bind (resolving
  review finding 12) and the RUNNER-SIDE env scrub (resolving review finding 11).

Acceptance criteria:

- A non-loopback server bind (DT1) and a remote runner control channel (DT3) are
  each modeled as a `ConnectivityExposure` that is `blocked_pending_permission`
  until an explicit grant exists, using the EXACT existing flow:
  `expose_connectivity_stub` (`exposure=private`/`public`) ->
  `request_connectivity_exposure_approval` ->
  `activate_connectivity_exposure` (requires a matching allow grant with the
  `permission_scope` and subject) -> `revoke_connectivity_exposure`. No
  distributed remote control activates without a grant.
- Conditional non-loopback bind (finding 12): `transport.rs` enforces loopback on
  the bound address unconditionally today. Add a runtime check so a non-loopback
  bind is permitted ONLY when an ACTIVE `ConnectivityExposure` grant exists; the
  DEFAULT (no grant) preserves the current HARD rejection. Both branches are tested
  (granted -> bind allowed; no grant -> bind refused).
- `revoke_connectivity_exposure` revokes the capability end-to-end: after revoke,
  the exposure status is `revoked`, `reachable=false`, and a new control attempt on
  that channel is refused -- proven by a test, not attestation.
- Every exposure state change is an event
  (`connectivity.exposure_requested` / `_changed` / `_revoked`, already wired) so
  the full lifecycle is auditable; an operator can read status via
  `connectivity_exposure_status`.
- The subscription-backed agent (Codex/Claude) on a remote runner is a PRIVILEGED
  CONNECTOR: its `auth_ref` / `identity_ref` is referenced by handle, and the
  privileged-connector env scrub (`ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` and
  the connector token) EXECUTES IN THE RUNNER-SIDE SPAWN PATH (DT-pre-B), because
  the spawn happens on the runner device where the server-side adapter scrub cannot
  reach (finding 11 -- this is a real new runner-side deliverable, not covered by
  the existing server-side scrub). The server never reads provider session
  credentials; no key, OAuth/subscription token, cookie, session file, or
  transcript-with-secrets is logged or crosses the tunnel in the clear.
- A `public` exposure is high-risk and stays disabled by default: it requires the
  explicit grant AND is short-lived/auditable per `runtime-tunnel.md`
  (`ReverseTunnel`/Funnel out of scope beyond requiring the grant + audit).

Verification (deterministic-first, live opt-in gated):

- Deterministic test: a private/public server bind or remote control channel is
  `blocked_pending_permission` until a matching grant is recorded, then `active`;
  without the grant `activate` fails AND the non-loopback bind is refused.
- Deterministic revocation test: after `revoke`, the exposure is `revoked` /
  `reachable=false` and a subsequent control attempt is refused.
- Deterministic runner-env-scrub test: the runner-side spawn path drops the
  privileged-connector token and the `ANTHROPIC_*` vars; the spawned env contains
  no seeded credential marker.
- Audit test: replaying the event log reconstructs the full exposure lifecycle
  (requested -> active -> revoked) identically.
- `cargo fmt`; focused `cargo test -p capo-cli -p capo-server -p capo-runtime`.
- `git diff --check`.

Dependencies: DT3. Cross-workpad: `safety-gates` (grant model), in-tree
connectivity exposure lifecycle, `runtime-tunnel.md` (`auth_ref`/`exposure`),
`protocol-provider.md` (connector env policy).

## DT6 - All-Local Default Regression (single-box path unchanged + structurally inert)

Status: pending.

Prerequisite: DT1.

Scope:

- Protect the single-box, all-local path as the DEFAULT and a regression that runs
  in the always-on suite the entire time the distributed surface is built. Pure
  guard; no new capability. Adds an explicit INERTNESS GATING mechanism (resolving
  review finding 8) so "adds no events" is a checkable design, not an aspiration.

Acceptance criteria:

- A deterministic regression proves that with NO role flags / NO distributed
  config, a single process is byte-for-byte equivalent to today: server + local
  runner + client over loopback, the existing turn loop, the existing
  `Subscribe`/thread read model, and the checked-in `streaming-transport` ST9
  contract wire snapshots are UNCHANGED.
- INERTNESS GATING (finding 8 -- the mechanism, not just the claim): the heartbeat
  plane (DT2), the remote runner path (DT3), the buffered-event spool (DT4b), and
  the exposure-gating machinery (DT5) are CONSTRUCTED ONLY when a distributed
  `RoleConfig` (a non-loopback endpoint) is present. In the default single-process
  path that code is NOT ENTERED. This is asserted TWO ways: (1) the ST9 snapshots
  are byte-identical, AND (2) a STRUCTURAL assertion that no `connectivity.*` /
  heartbeat / exposure event type can be produced without a non-loopback endpoint
  in scope.
- The all-local path stays loopback-only by default: the server listener still
  HARD-rejects a non-loopback bind unless an explicit active exposure grant exists
  (the DT5 conditional bind; the default branch preserves today's enforcement).
- This regression is part of the always-on suite (not `#[ignore]`); a change that
  alters the single-box path fails it.

Verification (deterministic-first):

- Deterministic test: a single-process all-local run produces the same thread
  projection and the same ST9 contract wire snapshots as the pre-distributed
  baseline (regenerate-and-diff against the checked-in fixtures).
- Deterministic structural test: with no non-loopback endpoint, the heartbeat /
  exposure / remote-runner machinery is not constructed and no `connectivity.*` or
  heartbeat event type is reachable.
- Deterministic test: the loopback-only bind enforcement still rejects a
  non-loopback bind absent a grant.
- `cargo fmt`; focused `cargo test -p capo-server -p capo-cli`.
- `git diff --check`.

Dependencies: DT1. Cross-workpad: `streaming-transport` (contract snapshots,
loopback enforcement).

## DT7 - Cross-Device End-To-End Smoke (opt-in/gated) Paired With A Deterministic E2E Gate

Status: pending.

Prerequisite: DT1-DT6 (incl. DT4a + DT4b).

Scope:

- A deterministic three-process E2E gate (always-on, deterministic-timed) plus a
  live cross-device smoke (opt-in, gated, skips cleanly) that exercise the full
  path: role config -> remote attach -> stream -> reconnect-resume -> revoke. The
  live smoke is paired with the same shape assertion the gate pins, so completion
  is never operator-attested.

Acceptance criteria:

- Add an ALWAYS-ON deterministic E2E gate that runs the three roles as three
  separate OS PROCESSES over loopback / `FakeTunnel`, with INJECTED/CONTROLLABLE
  timing (resolving review finding 9): a fake clock for heartbeat timing, a
  deterministic drop-injection test seam (NOT wall-clock sleeps), and an explicit
  TIMEOUT + CLEANUP (kill + reap) bound on every step so the gate cannot hang or
  leak processes:
  - server process binds loopback; runner process announces + attaches via
    `SshRemoteProcessRunner` (fake transport); client process opens a
    `subscribe_tcp` tail;
  - drive one real turn on the remote runner (a scripted/fixture adapter, no live
    provider), observe the incremental output + `TurnFinished` in the client tail;
  - deterministically drop the client mid-turn and resume from `delivered_through`
    with no gap/dupe (DT4a); drop the runner, spool + replay buffered events, and
    assert exactly-once (DT4b); transition a runner heartbeat to degraded and
    recover (DT2);
  - revoke the remote control exposure and assert the control channel is refused
    (DT5);
  - assert runner-side redaction holds for a seeded secret before it crosses the
    (fake) tunnel (DT3).
- Add a LIVE cross-device smoke behind the explicit opt-in env gate
  (`CAPO_SERVER_RUN_DISTRIBUTED_LIVE=1`, optional
  `CAPO_DISTRIBUTED_TAILNET_PREFLIGHT=1`; matching the existing
  `CAPO_SERVER_RUN_*_LIVE` family), `#[ignore]`d and skipping cleanly when the
  tailnet/SSH path is unavailable, that runs server / runner / client across real
  devices (or three hosts over the tailnet) and drives the same flow over
  `Ssh`/`Tailscale`. The smoke gates on a generic reachable `Ssh`/`Tailscale`
  endpoint (not vendor-locked).
- The live smoke is PAIRED with the deterministic gate's shape assertion (same
  event/turn/reconnect/refusal shape via a shared helper) so it is never the sole
  evidence; its transcript is captured with secrets stripped (artifacts pass the
  existing credential scan; any sensitive/unknown artifact is withheld or
  referenced by handle).
- The gate confirms the cross-cutting invariants on the live path: the server stays
  the single authoritative writer; the runner/client hold no authoritative state;
  exposures are grant-backed and revocable; no credential crosses the tunnel raw.

Verification (deterministic-first, live opt-in gated):

- Always-on deterministic three-process E2E gate passes with deterministic timing
  + per-step timeouts + process cleanup (role config, remote attach, stream,
  drop-resume DT4a, spool-replay DT4b, heartbeat degrade/recover, revoke-refusal,
  runner-side redaction).
- Live cross-device smoke runs only under the opt-in env gate, skips cleanly when
  unavailable, and asserts the identical shape via the shared helper; transcript
  attached with secrets stripped.
- `cargo fmt`; focused `cargo test -p capo-server -p capo-cli -p capo-runtime`,
  widening to `cargo test` if shared transport/state behavior changed.
- `git diff --check`.

Dependencies: DT1-DT6. Cross-workpad: `streaming-transport` (E2E gate pattern,
`subscribe_tcp`), `remote-runtime`, DT-pre-A / DT-pre-B.

## DT8 - Operator Docs For The Distributed Run Path

Status: pending.

Prerequisite: DT1-DT5.

Scope:

- Operator-facing documentation for starting and running Capo across three
  devices, and for auditing/revoking remote control. Docs only; references the
  shipped commands and gates, no new behavior.

Acceptance criteria:

- A `workpads/distributed-topology/` operator doc (and/or a top-level
  operator-facing doc the workpad references) covers, concretely against the DT1
  CLI surface:
  - how to start the server role and (optionally) expose it privately over the
    tailnet with a grant (DT5), including that a non-loopback bind requires an
    active grant and is otherwise hard-refused;
  - how to start a remote runner on another device, register + ANNOUNCE its
    `runtime_target` + endpoint over JSON-RPC, and attach it (DT1/DT3);
  - how to start the client on a third device and tail/steer the session
    (DT1/DT4a);
  - how keep-alive + reconnect behave, what the health states mean, and which leg
    is logged (runner<->server) vs. ephemeral (client<->server) (DT2);
  - how to audit exposures (`connectivity_exposure_status`) and REVOKE remote
    control (`revoke_connectivity_exposure`) (DT5);
  - the explicit statement that all-local is the default and what changes when you
    go distributed (DT6).
- The doc states the safety posture in operator terms: subscription agents are
  privileged connectors; credentials are referenced by handle and never logged;
  runner->server wire confidentiality depends on the tunnel transport
  (SSH/Tailscale encryption) plus Capo's runner-side redaction; tailnet ACLs are
  part of the deployment security posture and must be reviewed before remote
  dogfood (per `runtime-tunnel.md`).
- The doc references the deterministic E2E gate (DT7) and the opt-in live smoke env
  gate (`CAPO_SERVER_RUN_DISTRIBUTED_LIVE`) so an operator can reproduce both.

Verification (deterministic-first):

- The operator doc exists, references the actual DT1 CLI commands and the DT5/DT7
  surfaces, and is internally consistent with the shipped flags (a doc-vs-CLI
  consistency check or a referenced help-text snapshot).
- `git diff --check`.

Dependencies: DT1-DT5; references DT7. Cross-workpad: `runtime-tunnel.md`
(tailnet ACL posture), AGENTS.md (safety boundary).
