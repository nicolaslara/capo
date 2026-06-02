# Distributed Topology Knowledge

## Objective

Capture the decisions for the `distributed-topology` (DT) workpad: the
capstone/integration workpad that proves Capo runs as three roles --
server/controller, remote runner, client -- on different devices end-to-end, while
keeping all-local the default and a regression. It composes the boundaries the
connectivity track builds (`runtime-tunnel.md` design, the `DT-pre-A` connectivity
substrate, the `DT-pre-B` real remote runner) with the streaming resume cursor
(`streaming-transport`) and the already-in-tree connectivity exposure lifecycle; it
does NOT re-architect the loop, transport, permission, or goal models.

## Substrate Reality And The Hard Do-Not-Start Gate

This workpad was authored AHEAD of its substrate. The bullets below are the
ORIGINAL authoring-time snapshot and are NOW MOSTLY SUPERSEDED -- read the
"Substrate Update" subsection that follows for the current in-tree state:

- There is no `connectivity-tunnel` workpad; `ConnectivityTunnel` has only
  `Fake`/`LocalLoopback`/`EndpointStub` -- no `Ssh`/`Tailscale`.
- No heartbeat emission exists; `last_heartbeat_at` is an unwritten SQLite column.
- No `SshRemoteProcessRunner` type exists; `remote-runtime` RR1-RR14 delivered only
  the loopback-delegating `RemoteProcessRunner`.
- `runtime.target_registered` is emitted by a LOCAL CLI command, not by a remote
  runner announcing to a server.
- The redaction funnel (`redacted_for_egress`) runs at the SERVER egress
  (server->client), not on the runner->server hop.
- `transport.rs` enforces loopback unconditionally; there is no conditional
  non-loopback bind.

### Substrate Update (re-verified in-tree, 2026-06-02 after CT/RR landed)

The snapshot above is superseded; the connectivity + remote-runtime substrate has
LANDED on this branch. Current in-tree reality:

- The `connectivity-tunnel` workpad EXISTS (CT2/CT5/CT6/CT7/CT8/CT9/CT10 `done`).
- `ConnectivityTunnel::Tailscale` is in-tree (`crates/capo-runtime/src/lib.rs:5403`)
  with `resolve_endpoint`/`check_reachability`/`open_channel`. There is NO
  `ConnectivityTunnel::Ssh` variant (CT explicitly DEFERS `SshTunnel`); the SSH that
  landed is the RR8 `SshRemoteProcessRunner` EXECUTION runner, a different boundary.
- Heartbeat landed (CT5): `RuntimeProcessRef.last_heartbeat_at` IS written and
  `EventKind::ConnectivityHealthChanged` ("connectivity.health_changed") IS emitted
  (`crates/capo-state/src/event.rs:25`, `apply.rs`, `schema.rs`).
- `SshRemoteProcessRunner` + `SshRemoteConfig` ARE in-tree via RR8
  (`crates/capo-runtime/src/lib.rs:3830`, `:3337`).
- `transport.rs` already routes the bind/connect decision through
  `ExposurePolicy::loopback_default().authorize_socket(..)` (CT1), not a hand-rolled
  unconditional `is_loopback()`; the grant-gated non-loopback branch is the DT5 seam.

DT1 DONE: the runner->server ANNOUNCE path is now real JSON-RPC, not a local
store write. `capo role runner` sends `RegisterRuntimeTarget` over `send_tcp` to
the live server, which (single writer) appends `runtime.target_registered`,
idempotent on `runtime-target:{project}:{target}`. Decisions forced by the
adversarial review: the announce has NO in-process fallback (a dead server fails
loudly with `ConnectionRefused` -> actionable error, so
`announce_source=runner_jsonrpc` can never be a false label); and a `--connect`
that disagrees with a loopback `--server-addr` is rejected up front (the two
flags have distinct, documented roles -- endpoint resolution vs tunnel-local
dial). Tests: a three-process-over-loopback announce/tail test using a DISTINCT
runner state root that asserts the runner wrote no local store (proving the
announce rode TCP) plus a `capo role client` subprocess; a dead-server
loud-failure test; a `--connect`/`--server-addr` mismatch test; an idempotent
re-announce test (same sequence, single tail occurrence).

Still missing (still owned by DT/DT-pre tasks): the `ConnectivityTunnel::Ssh`
reachability variant if the DT track needs one (unowned upstream -- DT-pre-A);
runner-side redaction-before-transit and env scrub (DT3/DT5); the
buffered-event runner spool + idempotent replay (DT4b); the actual non-loopback
dial riding the `ConnectivityTunnel`-backed (DT5-granted) endpoint into the
runner attach (DT3/DT5, vs. RR8's direct `SshRemoteConfig`; DT1 resolves the
endpoint and blocks-pending-permission but does not yet open the granted tunnel);
and the reconnect-leg auditability decision (today only a `channel_closed`
payload boolean, not discrete `ConnectivityChannelOpened/Closed` event kinds).

Two consequences, both recorded as decisions:

1. The variant + heartbeat + reconnect work and the real SSH transport are made
   EXPLICIT UPSTREAM PREREQUISITE TASKS (`DT-pre-A`, `DT-pre-B`) owned by this
   workpad (or by a dedicated upstream workpad they link to), never references to a
   phantom `connectivity-tunnel` workpad. "No DT task begins before its named
   prerequisite lands" is now satisfiable, because every named prerequisite has a
   plan to land.
2. DT0 records a HARD do-not-start gate with concrete completion signals
   (`safety-gates` grant merged; `streaming-transport` listener + resume cursor +
   ST9 snapshots + ST11 restart-resume landed; `DT-pre-A` tunnel variants +
   heartbeat landed; `DT-pre-B` `SshRemoteProcessRunner` landed), not soft prose.
   UPDATE (2026-06-02): much of this gate is now SATISFIED -- the
   `connectivity-tunnel` track (CT3 Tailscale variant, CT5 heartbeat +
   `connectivity.health_changed`) and `remote-runtime` RR8 (`SshRemoteProcessRunner`)
   have LANDED on this branch. The still-open gate items are the
   `ConnectivityTunnel::Ssh` reachability variant (if the DT track needs one --
   unowned, CT defers it), the reconnect-leg event-kind decision, and the
   `safety-gates` grant model; see the "Substrate Update" subsection above.

## Injected Decision: Topology Over Boundaries, Single Authoritative Writer

The defining decision: the three roles are a DEPLOYMENT TOPOLOGY over Capo's
existing single-process boundaries, NOT a new distributed-consensus or
state-replication system.

- The **server/controller** remains the SINGLE authoritative writer of the
  event-sourced SQLite log and owns the turn loop and broadcast hub. No
  multi-writer, no quorum, no CRDT. `streaming-transport` already documented that
  concurrent writers stay unsupported until the `safety-gates` write lock; this
  workpad does not add a second writer -- the runner and client are
  NON-AUTHORITATIVE. Even the runner's buffered-event reconciliation (DT4b) routes
  through the server for append; the runner never writes truth directly.
- The **remote runner** owns process lifecycle via
  `RemoteProcessRunner`/`SshRemoteProcessRunner` behind the `RuntimeRunner`
  boundary and reports runtime events/heartbeat to the server. It holds no
  orchestration state, so a runner can be lost and replaced without losing truth.
- The **client** submits commands and tails the log via `subscribe_tcp` /
  `SubscribeStream`. It holds no authoritative state, so a client can disconnect
  and resume by sequence watermark.

Cross-device resilience comes from THREE things: the authoritative event log, the
`Subscribe { from_sequence }` resume cursor (`EventStream::delivered_through()`),
and the runner spool that replays buffered events through the single writer. We
integrate these across a transport drop rather than inventing replication. This is
why the workpad is an integration capstone, not a re-architecture.

## DT-D1 (Resolved): Runner<->Server Channel Reuses JSON-RPC

DECISION (was an open question; promoted to a committed decision because DT1/DT2/DT3
cannot be written against it undecided): the runner is "a special client that owns
processes" and reuses the EXISTING JSON-RPC command transport with a runner-role
classification. The runner ANNOUNCES itself to the server over that transport; the
server (single writer) appends `runtime.target_registered`. There is no second
protocol and no second bridge (honoring the non-goal). This closes the in-tree gap
that `runtime.target_registered` is currently only a local CLI store write -- DT1
builds the runner->server announce as new code, it does not pretend it "already
exists." FALSIFICATION: if `SshRemoteProcessRunner`'s shape forces a distinct
runtime control channel, `DT-pre-B` records the deviation and DT1 adapts; until
then JSON-RPC reuse is the committed answer.

## DT-D2 (Resolved): Runner-Reconnect Reconciliation = Spool + Idempotent Replay

DECISION (was an open question; promoted because DT4's "exactly once" cannot be
tested against an undecided mechanism): events a runner produced while disconnected
are reconciled by a runner-side SPOOL + replay-on-reattach, de-duplicated by
`runtime.*` idempotency keys at the server. Idempotency keys ALONE are insufficient
-- the in-tree keys dedupe command replays and recovery re-probes (same PID + boot
id seen twice), NOT arbitrary buffered output deltas -- so the spool is a real
DT4b deliverable, not an assumed capability. This is why DT4 is SPLIT: DT4a is the
watermark-resume the tree genuinely supports today; DT4b builds the spool. The
draft's single DT4 over-claimed "exactly once" against a mechanism that did not
exist; the split makes the claim honest and testable. FALSIFICATION: if a dogfood
trace shows no events are ever buffered during a drop, DT4b degrades to documenting
that DT4a's watermark resume suffices and the spool is removed -- recorded, not
silently assumed.

## All-Local Is The Default And A Structurally-Inert Regression

The single-box, all-local path stays the default with no role flags, and DT6 makes
it an always-on regression. Crucially, "the distributed surface is inert in the
all-local default" is now backed by a MECHANISM, not an aspiration: the heartbeat
plane, remote-runner path, buffered-event spool, and exposure gating are
CONSTRUCTED ONLY when a non-loopback `RoleConfig` is present. In the default
single-process path that code is not entered. DT6 asserts this two ways: the ST9
contract wire snapshots are byte-identical, AND a structural assertion that no
`connectivity.*` / heartbeat / exposure event type is reachable without a
non-loopback endpoint in scope. The server stays loopback-only by default; going
distributed is an explicit, grant-backed step (the DT5 conditional bind).

## Two Separate Health Planes (Connectivity Must Not Pollute The Truth Log)

Keep-alive (DT2) reuses `RuntimeProcessRef.last_heartbeat_at` and the
`connectivity.health_changed` event family from `DT-pre-A`, but the two legs are on
SEPARATE planes so a connectivity signal never writes into authoritative state:

- **runner<->server is LOGGED**: runner liveness affects PROCESS TRUTH, so a missed
  runner heartbeat is a legitimately auditable `runtime.health_changed` /
  `connectivity.health_changed` event, and on recovery the runner re-runs the
  `runtime-tunnel.md` recovery sequence (`health` / `recover_orphan` ->
  `run.recovered` / `run.orphaned` / `run.exited`).
- **client<->server is EPHEMERAL**: a missed client heartbeat transitions an
  in-memory, server-side connection state and is NEVER an authoritative log entry.
  If client jitter could write `degraded` into the truth log, a flaky client would
  spam the authoritative stream and DT6's byte-for-byte regression would break.
  This separation is the resolution of the review finding that the draft leaked a
  connectivity concern into subscription/event state.

A heartbeat is a LIVENESS signal, not proof a process exists; health transitions
are recorded events (runner leg) or observable status (client leg), never silent
flags; heartbeat payloads carry only liveness + handles, never credentials or
transcripts.

DECISION (DT2, resolving the DT-pre-A open reconnect-form question + adversarial
review findings 1-3):

- **Three-state on the LOGGED plane.** The CT5 `HeartbeatMonitor` is a BINARY probe
  (reachable | unreachable). The DT2 `RunnerServerPlane` owns the
  `available -> degraded -> unreachable` three-state vocabulary that
  `ConnectivityEndpoint.status` publishes: the FIRST confirmed miss records
  `degraded`, a CONTINUED miss escalates to `unreachable`, and each edge is its own
  `connectivity.health_changed` event. The runner leg does NOT skip `degraded`
  (finding 1). The `status` field carries the three-state value; the `detail` field
  carries the CAUSE (`initial` / `lost` / `stalled` / `reconnected`).
- **Reconnect form = (a), `health_changed` + `detail="reconnected"`.** No new
  `ConnectivityChannelOpened` event kind is added (finding 2). The in-tree
  `connectivity.health_changed` family already names every health edge auditably; a
  separate `channel_opened` kind would be a SECOND audit path for the same fact,
  violating the "no parallel/duplicate event path" invariant. Leg recovery is
  auditable from the log via this named, in-tree-verified mechanism.
- **The reconnect flag is WIRED, not self-attesting (finding 3).**
  `RunnerBeat.must_rerun_recovery` is acted on by the caller, which drives
  `RemoteProcessRunner::recover_run(...)` -> `run.recovered` / `run.orphaned` /
  `run.exited` / `recovery_pending`. A returned LEG over a GONE process records
  `run.exited`, never fabricated liveness. Proven end-to-end (no sleep, no network)
  by the `dt2_runner_reconnect_*` tests in `capo-runtime`.

## Streaming Resume Is The Core Integration Guarantee (Split DT4a / DT4b)

The `Subscribe { from_sequence }` tail resumes across a client OR runner
connectivity drop with no gap and no duplicate (DT4a), reusing the EXACT seam
guarantee `event_tail.rs` enforces (`delivered_through` watermark; live events
strictly greater than the watermark) and `streaming-transport` ST11's
restart-resume. This half is well-grounded in-tree. The SEPARATE problem of events
a runner buffered while offline is DT4b (spool + idempotent replay, per DT-D2). The
server's log is the single source of continuity; neither client nor runner holds
authoritative state to lose.

### DT4a (Resolved): The Three-Role Resume Contract

The resume contract for the three-role case (server/controller, remote runner,
client), as implemented and tested in DT4a:

- **Single source of continuity.** The server's durable event log
  (`SqliteStateStore`, the single authoritative writer) is the ONLY source of
  stream continuity. Both the client leg and the runner leg are non-authoritative
  observers; neither holds state that can be lost on a drop. A drop on EITHER leg
  is recovered identically by re-reading the log from a cursor -- there is no
  per-leg replication and no parallel event path.
- **Watermark cursor.** Each subscriber tracks the highest sequence it has
  delivered via `EventStream::delivered_through()` (the in-process seam) /
  `SubscriptionBacklog::next_sequence` (the TCP seam). On reconnect it re-issues
  `Subscribe { from_sequence = delivered_through }`. The runner leg uses the SAME
  mechanism (DT-D1: "a runner is a special client that owns processes"), typically
  session-scoped to the session whose process it owns; the client leg is usually
  unscoped.
- **`from_sequence` semantics (the half-open seam).** The backlog re-delivers
  every committed event STRICTLY AFTER `from_sequence` and NONE at or below it, so
  the union of (pre-drop delivered) + (post-resume backlog) is contiguous,
  strictly increasing, and duplicate-free -- "no gap, no dupe". A STALE cursor
  (well behind the head) is served the full backlog after that point.
- **Ahead-of-log rejection boundary.** A `from_sequence` STRICTLY AHEAD of the
  committed head has no servable continuation and is rejected with the typed
  `ServerError::SubscribeFromSequenceAheadOfLog { from_sequence, latest_sequence }`
  (wire kind `subscribe_from_sequence_ahead_of_log`), rather than masking a client
  cursor bug as an empty backlog. `from_sequence == head` is VALID (an empty
  backlog, resuming exactly at the tail); `head + 1` and beyond are rejected. On an
  empty log the head is 0: cursor 0 is valid, any positive cursor is rejected. This
  validation runs in ONE shared seam (`read_subscription_backlog_validated`) used
  by BOTH the in-process `subscribe()` path and the in-process
  `ServerCommand::Subscribe` handler arm, and is enforced over the wire on the TCP
  `subscribe_tcp` path (returned as `TransportError::Remote { kind: ... }`).
- **Restart durability.** The guarantee survives a server restart: after the
  server reopens and rebuilds read models from the durable log, a resume from the
  same `from_sequence` yields the byte-identical continuation (ST11 restart-resume).
- **Scope boundary.** DT4a covers resume of events ALREADY COMMITTED to the
  server's log. Events a runner BUFFERED while disconnected are out of scope here
  and are reconciled by DT4b (spool + idempotent replay, per DT-D2).

## Redaction Is Placed Precisely (Don't Claim A Property The Architecture Can't Enforce)

The draft claimed "a secret never crosses the tunnel in the clear" via
redaction-on-emit, but the in-tree `redacted_for_egress` funnel runs at the SERVER
egress (server->client), which is UPSTREAM of the runner->server hop -- so it
protects the client tail, not the leg where raw runner stdout travels. The honest,
enforceable design (DT3):

- A redaction pass runs ON THE RUNNER before output crosses the tunnel (a new
  runner-side deliverable + test), so a seeded secret is scrubbed before transit on
  the leg Capo controls.
- The server's `redacted_for_egress` continues to guard storage and the client
  tail.
- runner->server wire confidentiality is documented as a TRANSPORT property
  (SSH/Tailscale encryption), not a Capo redaction property.

This honors the AGENTS.md boundary "do not claim a security property the deployment
cannot enforce": Capo adds redaction to the leg it can enforce and is explicit that
the wire's confidentiality is the tunnel's job.

## Auditable + Revocable Remote Control Is A First-Class Acceptance Criterion

Per the AGENTS.md safety boundary, DT5 makes every remote-control capability a
recorded, grant-backed, revocable `ConnectivityExposure`, using the EXACT existing
lifecycle in `crates/capo-cli/src/connectivity.rs` (`expose_connectivity_stub` ->
`request_connectivity_exposure_approval` -> `activate_connectivity_exposure` ->
`revoke_connectivity_exposure`, with `activate` requiring a matching allow grant +
subject). This is why `connectivity.rs` is a working lifecycle, not a stub: the
workpad integrates it; it does not build a permission engine.

Two precision points the draft glossed:

- The non-loopback server bind is CONDITIONAL: `transport.rs` hard-rejects
  non-loopback today, so DT5 adds a runtime check that permits a non-loopback bind
  ONLY with an active exposure grant; the default (no grant) preserves the hard
  rejection. Both branches are tested.
- The privileged-connector env scrub must EXECUTE IN THE RUNNER-SIDE SPAWN PATH
  (`DT-pre-B`), because on a remote runner the spawn happens on the runner device
  where the server/adapter-side scrub cannot reach. The existing server-side scrub
  does NOT cover the remote runner; this is a real new runner-side deliverable.

The subscription-backed agent (Codex/Claude) on a remote runner is a PRIVILEGED
CONNECTOR, not an ordinary API key: its `auth_ref`/`identity_ref` is a handle, the
server never reads provider session credentials, and no key, OAuth/subscription
token, cookie, session file, or transcript-with-secrets is logged or crosses the
tunnel raw. `public` exposure stays disabled by default and high-risk per
`runtime-tunnel.md`.

## Deterministic-First, Controllable Timing, Live Opt-In Gated

Discipline mirrors the rest of the track: deterministic three-process-over-loopback
/ `FakeTunnel` tests land BEFORE any real tailnet/SSH path, and they use
INJECTED/CONTROLLABLE timing -- a fake clock for heartbeats and a deterministic
drop-injection seam, NOT wall-clock sleeps -- with explicit per-step TIMEOUTS and
process CLEANUP so the always-on three-OS-process E2E gate cannot hang, flake, or
leak processes. The live cross-device smoke is opt-in behind
`CAPO_SERVER_RUN_DISTRIBUTED_LIVE` (aligned with the existing
`CAPO_SERVER_RUN_*_LIVE` family -- `CAPO_SERVER_RUN_CODEX_LIVE`,
`CAPO_SERVER_RUN_STREAMING_LIVE`; the draft's `CAPO_RUN_DISTRIBUTED_LIVE` dropped
the `CAPO_SERVER_` prefix, now corrected), with an OPTIONAL reachability preflight
`CAPO_DISTRIBUTED_TAILNET_PREFLIGHT`; it is `#[ignore]`d, skips cleanly when the
tailnet is unavailable, and is PAIRED with the deterministic gate's shape assertion
so nothing completes on operator self-attestation; secrets are stripped from all
evidence.

## Non-Goals

- No new transport protocol and no second HTTP/SSE bridge: reuse the JSON-RPC
  transport (DT-D1), the broadcast tail, and `capo-web`.
- No multi-writer event log, no consensus/quorum, no CRDT, no state replication:
  the server is the single authoritative writer; the runner spool replays THROUGH
  the server.
- No changes to the turn loop (`real-turn-loop`), the `Subscribe`/streaming
  contract (`streaming-transport`), the permission/grant engine (`safety-gates`),
  the goal model (`goal-autonomy`), the adapters (`depth`), or the web client.
- No public-internet exposure as a default; `ReverseTunnel`/Funnel beyond requiring
  the grant + audit is out of scope.
- Do not claim a security property the deployment cannot enforce: runner->server
  wire confidentiality is the tunnel transport's property, not a Capo redaction
  property; tailnet ACL posture is the operator's, documented in DT8.

## Open Questions

- DT-OQ3: What heartbeat interval / miss-threshold defaults balance fast failure
  detection against tailnet jitter? Tune with a fake clock first, then a live
  smoke. (Tuning, not a load-bearing mechanism -- legitimately open.)
- DT-OQ4: Should the live cross-device smoke require Tailscale specifically, or run
  over any `Ssh` endpoint? Leaning: gate on a generic reachable `Ssh`/`Tailscale`
  endpoint so the smoke is not vendor-locked, recording tailnet as the reference
  path.
- DT-OQ5: When the server is exposed privately for distributed control, what is the
  short-lived-exposure / re-grant cadence before it is considered stale (ties to
  the `runtime-tunnel.md` `expires_at` on `ResolvedEndpoint`)?
- DT-OQ6: Should the `DT-pre-A` connectivity substrate (tunnel variants +
  heartbeat + reconnect events) live as a standalone upstream workpad rather than
  as prerequisite tasks inside this one? It is owned here by default so the plan is
  self-contained; promote it to its own workpad if the connectivity track is
  formally scheduled in `TASKS.md`.
- DT-OQ7 (recorded for DT6, not blocking DT4a): the DT4a ahead-of-log validation
  added a `latest_sequence()` read (`SELECT COALESCE(MAX(sequence), 0) FROM
  events`) to EVERY `server.subscribe()` call, including the all-local single-box
  path (e.g. capo-web). The query is functionally correct and follows the existing
  per-query connection pattern, but it is a NEW DB round-trip on the default path.
  DT6 (byte-for-byte single-box regression) must account for it: either confirm it
  is not a hot path for single-box deploys, or cache the latest sequence in the
  broadcast hub on commit so the resume-cursor validation does not open a fresh
  SQLite connection per subscribe. Recorded here before DT6 so the cost is not
  discovered as a surprise during the regression.

(NOTE: the draft's DT-OQ1 and DT-OQ2 are NO LONGER open questions -- they were
load-bearing mechanism decisions for DT1-DT4 and are promoted to the resolved
decisions DT-D1 and DT-D2 above, each with a falsification condition. A capstone
integration workpad cannot start with its two central mechanisms undecided.)
