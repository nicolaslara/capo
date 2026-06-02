# Connectivity Tunnel Tasks

## Objective

Implement the `ConnectivityTunnel` boundary BEYOND `LocalLoopbackTunnel` /
`FakeTunnel` so the Capo server is reachable by clients AND by runtime targets
that RESOLVE THROUGH the tunnel on OTHER DEVICES, with the FIRST real tunnel
adapter being TAILSCALE. This workpad delivers REACHABILITY only — it never
builds the remote runner that executes a process on another device (that stays a
`depth`/runtime concern; it merely resolves its endpoint through this tunnel).
Cross-device reachability rides the tailnet (device identity + ACLs); Tailscale
Funnel / public exposure stays OUT OF SCOPE and remains gated behind explicit
permission, short-lived grants, and audit events.

This workpad realizes the `ConnectivityTunnel` half of
`workpads/architecture/runtime-tunnel.md` (the runtime/`RuntimeRunner` half is
owned by `depth` DP7/DP8). It builds directly on the existing exposure lifecycle
already in-tree: `crates/capo-cli/src/connectivity.rs` (expose-stub /
request-approval / activate / revoke / exposure-status / exposure-evidence) and
the `ConnectivityTunnel { Fake, LocalLoopback, EndpointStub }` enum +
`ExposureScope { Loopback, Private, Public }` in `crates/capo-runtime/src/lib.rs`.
It adds: a real `TailscaleTunnel` adapter behind the enum, an explicit
`ExposurePolicy`, `auth_ref` credential HANDLES (never raw, never logged),
tunnel health (heartbeat + `last_heartbeat_at` + reconnect events), opt-in
anti-sleep so a laptop server stays reachable, and auditable + revocable exposure
end-to-end.

Connectivity stays STRICTLY separate from execution (no process handles, no
`RuntimeRunner` coupling) and from controller/turn state. Deterministic
`FakeTunnel`/replay tests land before any live Tailscale path; the live Tailscale
path is opt-in behind an explicit env gate and `#[ignore]`/skips cleanly when the
tailnet/CLI is unavailable. No acceptance criterion may rest on operator
self-attestation.

## Status

Planned. All tasks pending.

GATE: `distributed-topology` (a NEW gate this workpad defines and registers in
CT0; it does not exist in-tree today). `remote-runtime` is a feature workpad
file, not a gate, and is NOT used as a gate here.

## Feature Set

- An `ExposurePolicy` that defaults to `loopback`, requires explicit opt-in for
  `private`/`public`, and refuses any non-loopback bind without an auth handle
  (CT1).
- Credential handling by `auth_ref` HANDLE only: tunnel/device identity and auth
  are referenced indirectly; raw tokens/keys/cookies/session files are never
  stored or logged; resolution is architecturally confined to the adapter, with a
  redaction guard as a secondary net over all connectivity events and CLI output
  (CT2).
- The `ResolvedEndpoint` / `ConnectivityEndpointConfig` schema extended with
  `auth_ref` + `identity_ref` HANDLES and `identity_fingerprint` + `expires_at`,
  added by an OWNING task rather than assumed (CT2).
- A real `TailscaleTunnel` adapter behind the `ConnectivityTunnel` enum: tailnet
  endpoint resolution, host/device-identity checks, reconnect events, redacted
  logs; `FakeTunnel` carries the same surface deterministically (CT3, CT4).
- An explicit `open_channel`/`close_channel` extension to the
  `ConnectivityTunnel` surface so revoke can really TEAR DOWN a channel, added by
  an OWNING task (CT3) before CT7 depends on it.
- Tunnel health: heartbeat + `last_heartbeat_at` projection +
  `connectivity.health_changed` + reconnect events, driven by an injectable
  clock/ticker so stall-deadline behavior is deterministic (CT5).
- Anti-sleep when serving locally (macOS IOKit power assertion, Linux
  `systemd-inhibit`) as an opt-in LIFECYCLE concern kept SEPARATE from agent
  execution, with a one-way exposure-state -> inhibitor dependency (CT6).
- Auditable + revocable exposure: exposure/grant events plus a working revoke
  that tears the tunnel down and proves unreachability (CT7), building on the
  existing revoke command.
- Funnel/public exposure stays out of scope: permission-required, short-lived
  (clock-swept `expires_at` auto-revoke), audited, and unavailable without an
  explicit grant (CT8).
- A consolidated deterministic suite + a single live opt-in Tailscale smoke with
  a defined skip predicate, paired with deterministic assertions (CT9, CT10).

## CT0 - Workpad, Routing, Gate Definition, Boundary Ownership, And Verification Invariant

Status: pending.

Scope:

- Register `connectivity-tunnel` as its own workpad and route it in
  `workpads/WORKPADS.md` + `TASKS.md` (task-id prefix `CT`).
- DEFINE and register the new `distributed-topology` gate (it does not exist
  in-tree). Do NOT gate on `remote-runtime` (a feature workpad file, not a gate).
- Establish the boundary this workpad OWNS and the ones it explicitly does NOT.

Acceptance criteria:

- `distributed-topology` is introduced as a NAMED gate: CT0 records its meaning
  (the Capo server is reachable by clients and runtime targets across devices via
  an auditable, revocable tunnel) and registers it in `workpads/WORKPADS.md`
  alongside the existing gate vocabulary, so the gate is evaluable and the
  workpad is routable. No reference is made to any undefined gate string, and
  `remote-runtime` is NOT used as a gate.
- Record that this workpad owns the `ConnectivityTunnel` boundary only:
  `ExposurePolicy`, `auth_ref`/`identity_ref` handles, the `open_channel`/
  `close_channel` surface extension, `TailscaleTunnel` + `FakeTunnel` parity,
  tunnel health/heartbeat/reconnect, anti-sleep lifecycle, and auditable+revocable
  exposure.
- Record the DEFERRED/foreign boundaries: `RuntimeRunner` process lifecycle + OS
  sandbox + worktrees stay with `depth` (DP7/DP8); the `RemoteProcessRunner`/
  `SshRemoteProcessRunner` remote-EXECUTION runner stays OUT of this workpad (it
  is a runtime concern that merely RESOLVES through a tunnel); `SshTunnel` and
  `ReverseTunnel` enum variants stay named-but-unbuilt per `runtime-tunnel.md`;
  the `PermissionPolicy`/grant lifecycle engine stays with `safety-gates` (this
  workpad CONSUMES grants, it does not redefine them); the web/`web/app` client is
  the web agent's.
- Record that the injected decision (Tailscale is the FIRST real tunnel;
  Funnel/public is out of scope) is the workpad's defining design choice,
  captured in `knowledge.md`.
- Record the workpad-wide verification invariant: deterministic `FakeTunnel`/
  replay tests land before any live Tailscale path; every manual smoke is paired
  with a deterministic assertion (resolution snapshot, exposure-event shape, or
  restart/replay); the live Tailscale path is opt-in behind an explicit env gate
  mirroring `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` + `CAPO_SERVER_RUN_CODEX_LIVE`
  (i.e. `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT` +
  `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE`) and `#[ignore]`/skips cleanly; no
  criterion rests on operator self-attestation; secrets are stripped from all
  evidence.
- Record the per-task prerequisites: CT1-CT2 (policy + auth handle + schema
  extension) have no cross-workpad prerequisite beyond the existing exposure
  lifecycle; CT3-CT5 (Tailscale adapter + channel surface, identity, health)
  build on CT1-CT2; CT6 (anti-sleep) is independent additive lifecycle; CT7
  (revoke teardown) extends the existing revoke command and depends on the CT3
  channel surface, CT5 heartbeat, and (soft) CT6 release; CT8 (Funnel
  out-of-scope guard) builds on CT1 and reuses the CT5 clock for expiry; CT9-CT10
  consolidate determinism + the single live smoke.

Verification:

- `workpads/connectivity-tunnel/tasks.md` + `knowledge.md` + `references.md`
  exist and pass `git diff --check`.
- `connectivity-tunnel` is PRESENT in `workpads/WORKPADS.md` (entry + `Load:`
  block + `Rules:`) and in `TASKS.md` with the `CT` prefix, and the
  `distributed-topology` gate is registered and described; verified by grep, not
  by assertion.
- No Rust code in CT0.

Dependencies: none (intra); names the cross-workpad seams with `safety-gates`
(grants), `depth` (runtime boundary), and the architecture `runtime-tunnel.md`
enum.

## CT1 - ExposurePolicy: Loopback Default, Explicit Opt-In, Non-Loopback Bind + Connect Require Auth

Status: pending.

Scope:

- Introduce an explicit `ExposurePolicy` type in `crates/capo-runtime/src/lib.rs`
  alongside the existing `ExposureScope { Loopback, Private, Public }`, and make
  it the gate the server bind + connect + tunnel resolution consult.
- Decide and STATE the scope of `connect_loopback`: both the listener guard
  (`transport.rs:563`) and the connect-side guard (`connect_loopback`, ~`:648`)
  are IN scope for CT1, so the bind and connect sides stay symmetric.

Acceptance criteria:

- `ExposurePolicy` resolves a requested `ExposureScope` against an effective
  ceiling whose DEFAULT is `Loopback`; promotion to `Private` or `Public`
  requires explicit opt-in (config/flag/grant), never an implicit default.
- Promotion of the effective ceiling (Loopback -> Private/Public) is itself an
  AUDITED event: it emits `connectivity.policy_changed` carrying the old/new
  ceiling, the opt-in source (config/flag/grant), and a timestamp, with NO secret
  in the payload, so an operator can reconstruct WHY a private/public exposure
  became possible (the per-exposure `exposure_requested` trail alone does not
  record the policy change). The event is replay-stable. (Resolved open question
  in `knowledge.md`.)
- A non-loopback bind/resolution is REFUSED when no `auth_ref` handle is attached
  (CT2): `ExposurePolicy::authorize(scope, auth_ref)` returns a typed refusal
  (e.g. `ConnectivityError::AuthRequired`) for `Private`/`Public` with no handle,
  and the refusal is representable as a `connectivity.exposure_requested` ->
  blocked event, not a silent allow.
- BOTH the server transport loopback listener guard
  (`crates/capo-server/src/transport.rs:563`, "server listener must be loopback")
  AND the `connect_loopback` connect-side guard (~`:648`) are REPLACED by an
  `ExposurePolicy` check; loosening only one side is an asymmetric hole and is not
  acceptable. Loopback still passes with zero config on BOTH sides; a non-loopback
  bind/connect requires the policy to have been explicitly promoted AND an auth
  handle present, otherwise it fails closed with the same fail-closed semantics.
- A regression test pins that with NO `ExposurePolicy` config, the existing
  loopback bind AND connect behavior is byte-for-byte unchanged (the safe default
  is genuinely identical, not merely "loopback still passes"), and that every
  existing caller of `serve_tcp_with_handler` (notably `capo-web` at
  `127.0.0.1:4177`) is unaffected.
- `ExposureScope::requires_permission()` (Loopback=false, else true) and
  `permission_scope()` (`network:connect:localhost` /
  `network:connect:private_tunnel` / `network:expose:public`) remain the mapping
  into the `safety-gates` grant scopes used by the existing activate path; CT1
  does not change the grant engine.
- The existing `expose-stub`/`activate-exposure` flow
  (`crates/capo-cli/src/connectivity.rs`) routes through `ExposurePolicy` so a
  `private`/`public` exposure without a satisfied policy + auth handle stays
  `blocked_pending_permission` and cannot reach `active`.

Verification:

- Deterministic tests in `capo-runtime`: loopback resolves with no auth and no
  opt-in; `Private`/`Public` without `auth_ref` -> `AuthRequired` refusal;
  `Private`/`Public` with opt-in + handle -> permitted resolution but still
  `permission_required = true` (grant still gates activation); a ceiling promotion
  emits a `connectivity.policy_changed` event with the old/new ceiling and opt-in
  source and no secret.
- Deterministic `capo-server` test: loopback bind AND loopback connect succeed
  unchanged under the default (no-config) policy; a simulated non-loopback bind
  AND connect fail closed under the default policy.
- Deterministic `capo-cli` test: `private` expose-stub without a satisfied policy
  stays `blocked_pending_permission` through `activate-exposure`.
- `cargo test -p capo-runtime -p capo-server -p capo-cli`; `cargo fmt`;
  `git diff --check`.

Dependencies: CT0. Consumes `safety-gates` grant scopes (already in-tree); does
not modify the grant engine.

## CT2 - Auth By `auth_ref` Handles + Schema Extension: Confined Resolution, Never Logged, Redaction Guard

Status: done. `ConnectivityEndpointConfig` gained opaque `auth_ref`/`identity_ref`
HANDLES (`with_handles` builder, empty normalized to `None`); `ResolvedEndpoint`
gained `identity_fingerprint`/`expires_at` (`with_identity_fingerprint`/
`with_expires_at` builders). The four fields propagate through the
`ConnectivityExposureProjection`, the `capo-state` event codec (payload_json,
since the lettered columns were exhausted), the `connectivity_exposures` table
(+ nullable back-fill migration), the projection reader, and the exposure
projection — round-trip + restart-replay stable (`capo-state`
`ct2_connectivity_handle_schema_round_trips_and_replays`). The connectivity
redaction guard (`crates/capo-state/src/connectivity_redaction.rs`) is the
SECONDARY net with PER-FIELD rules: a credential-pattern match in a HANDLE field
FAILS CLOSED (refuse to persist), free-text payload fields are scrubbed; it scans
every emitted surface (expose-stub event payload + exposure-evidence artifact)
before emission, making the previously-unverified `RedactionState::Safe` marker
mean something. The expose-stub CLI takes `--auth-ref`/`--identity-ref`, guards
them fail-closed, records auth MODE + opaque handles only (never a raw key), and
the policy `authorize` now consults the handle. The PRIMARY never-logged
guarantee remains ARCHITECTURAL CONFINEMENT (the planted-pattern net proves only
the planted shapes are caught, not an arbitrary credential).

Scope:

- Extend the connectivity schema with the HANDLE + audit fields the rest of the
  workpad needs, mirroring `runtime-tunnel.md`, as an OWNED change here (not
  buried in a verification line): add `auth_ref: Option<String>` and
  `identity_ref: Option<String>` to `ConnectivityEndpointConfig` /
  `ConnectivityEndpoint`, and add `identity_fingerprint: Option<String>` +
  `expires_at: Option<...>` to `ResolvedEndpoint`. These propagate through the
  `connectivity_endpoints` record shape, the event codec in `capo-state`, and the
  exposure projection.
- Make credential resolution architecturally confined to the adapter and add the
  redaction guard as a secondary net.

Acceptance criteria:

- `ConnectivityEndpointConfig` carries `auth_ref` + `identity_ref` HANDLES;
  `ResolvedEndpoint` carries `identity_fingerprint` + `expires_at`. All four are
  opaque pointers / derived values (e.g. `keychain:capo/tailnet-authkey`,
  `tailscale:device:<stable-id>`, a hash fingerprint, an expiry instant), never a
  raw credential value. The schema additions round-trip through the event codec
  and projection and are replay-stable (a restart rebuilds them identically).
- The PRIMARY never-logged guarantee is ARCHITECTURAL: `auth_ref` resolution to a
  real credential happens ONLY inside the tunnel adapter at connect time; the
  resolved value is structurally never returned to the controller (no
  controller-facing type carries a field that holds the secret), never stored, and
  never logged. State this as an ARCHITECTURAL CONFINEMENT guarantee (a
  design-level structural commitment enforced by confinement + redaction, NOT a
  Rust compile-time/type-system guarantee: the handle fields are `Option<String>`,
  so the compiler does not prevent a raw value from being placed in one — that is
  exactly why the fail-closed pattern guard below exists). The pattern guard is
  defense-in-depth, not the proof of "never logged." (If true compile-time
  enforcement is later desired, introduce a newtype `AuthHandleRef(String)` whose
  constructor rejects raw-credential-pattern strings; CT2 does not specify such a
  wrapper, so it does not claim type-level enforcement.)
- The connectivity redaction guard is the SECONDARY net: per-field rules, not an
  "or". A credential-pattern match in a HANDLE field (`auth_ref`/`identity_ref`)
  is a BUG and FAILS CLOSED (refuse to persist), because a raw value in a handle
  field must never be silently scrubbed. Scrubbing/redaction is reserved ONLY for
  free-text payload fields where redaction is the documented behavior. The guard
  covers every emitted surface (event payload, projection field, CLI render,
  evidence artifact) before emission.
- This guard ADDS the missing enforcement behind the existing
  `RedactionState::Safe` marker on `connectivity.exposure_*` events
  (`crates/capo-cli/src/connectivity.rs` lines 125/268/349/413), which today is an
  UNVERIFIED assertion (the code marks events Safe; nothing scans them). CT2 makes
  the marker mean something. Word it as adding the enforcing guard, not reusing an
  existing guard.
- The Tailscale connector (CT3) records AUTH MODE / device identity fingerprint
  only (e.g. `auth_mode = tailscale_authkey_handle`,
  `identity_fingerprint = <hash>`), never the authkey, matching the
  subscription-connector "record auth mode only" rule from `protocol-provider.md`.

Verification:

- Deterministic schema test: an endpoint config with `auth_ref`/`identity_ref`
  handles and a resolved endpoint with `identity_fingerprint`/`expires_at`
  round-trip the HANDLE/derived values only through codec + projection, and a
  restart rebuilds identical state.
- Deterministic fail-closed test: a config carrying a raw-credential-looking value
  in a HANDLE field is REFUSED (fail closed), not scrubbed.
- Deterministic redaction (defense-in-depth) test: a planted fake authkey/token/
  cookie (e.g. `tskey-auth-DEADBEEF...`) is absent from every emitted connectivity
  surface (event payload, projection, CLI render, exposure-evidence artifact),
  while the assertion text records that this proves the planted patterns are
  caught, NOT that an arbitrary credential is universally caught — the universal
  guarantee is the architectural confinement above.
- `cargo test -p capo-runtime -p capo-cli -p capo-state`; `cargo fmt`;
  `git diff --check`.

Dependencies: CT0, CT1. The `auth_ref`/`identity_ref`/`identity_fingerprint`/
`expires_at` additions touch the `capo-state` event codec + projection schema, so
`capo-state` is explicitly in scope (hence the `-p capo-state` test). Reuses the
existing `RedactionState` marker discipline in `crates/capo-state` while adding
the enforcement it lacked.

## CT3 - TailscaleTunnel Adapter + `open_channel`/`close_channel` Surface (Endpoint Resolution Over The Tailnet)

Status: pending.

Scope:

- Extend the `ConnectivityTunnel` surface with `open_channel` / `close_channel`
  (today the enum exposes only `resolve_endpoint` / `check_reachability` /
  `exposure_report` / `binding` at `crates/capo-runtime/src/lib.rs:1743-1792`;
  there is NO channel open/close). This is an OWNED prerequisite for CT7's real
  teardown — added here, not assumed by CT7.
- Add a real `Tailscale(TailscaleTunnel)` variant to the `ConnectivityTunnel`
  enum (today `{ Fake, LocalLoopback, EndpointStub }`), implementing the full
  surface: `resolve_endpoint`, `check_reachability`, `open_channel`,
  `close_channel`, `exposure_report`, `binding`.

Acceptance criteria:

- The `ConnectivityTunnel` enum + every implementing tunnel gain
  `open_channel(resolved_endpoint) -> ConnectivityResult<OpenChannel>` and
  `close_channel(channel: OpenChannel) -> ConnectivityResult<()>`, wired through
  every match arm. `OpenChannel` is the owned reachability handle returned by
  `open_channel` and consumed by `close_channel` (it supersedes the
  `runtime-tunnel.md` design's tentative `ChannelRef` name; CT3 OWNS this naming —
  the design doc's `ChannelRef`/unspecified `close_channel` signature is resolved
  here to `OpenChannel` + the signature above). `LocalLoopback`/`EndpointStub`/
  `Fake` implement them coherently (loopback opens a loopback channel; `FakeTunnel`
  opens/closes a scripted channel). The channel is a reachability handle, NOT a
  process handle and NOT a `RuntimeRunner` coupling.
- `TailscaleTunnel` resolves a Capo-server / runtime-target endpoint to a TAILNET
  address (MagicDNS name or `100.64.0.0/10` CGNAT tailnet IP) at
  `ExposureScope::Private`, NOT loopback and NOT public; the resolved
  `ResolvedEndpoint` carries `exposure = Private`, the
  `network:connect:private_tunnel` permission scope, and
  `permission_required = true`.
- `TailscaleTunnel::resolve_endpoint()` called with `ExposureScope::Public`
  returns a typed `ConnectivityError` (e.g. `ScopeNotSupported`) at the adapter
  level until CT8 installs the full Funnel/public guard — a test-covered refusal,
  NOT a silent pass. This closes the CT3->CT8 window at the adapter layer (CT1's
  `ExposurePolicy` and the existing grant check already gate public exposure at
  the policy + activation layers; CT8 then replaces this stub refusal with the
  full short-lived/audited public guard).
- Endpoint resolution + status come through an injectable `TailscaleStatusSource`
  abstraction (modeled on the ACP `ScriptedAcpTransport`/`PipedProcessTransport`
  pattern from `depth`: a trait with a deterministic scripted implementation for
  tests and a live `tailscale status --json` / LocalAPI implementation for the
  gated path), so resolution is testable without a live tailnet.
- The adapter NEVER owns a process handle and never couples to `RuntimeRunner`: it
  resolves reachability/endpoints and opens/closes reachability channels only; a
  `RemoteProcessRunner` that later executes over the tailnet RESOLVES through this
  tunnel but is out of scope here (CT0 boundary note).
- All adapter logs/events are redacted via CT2 (no authkey, no raw
  `tailscale status` blob with tokens; identity is a fingerprint/handle).
- `binding()` reports `variant = "tailscale"`, `fake = false`; the variant is
  wired through every `ConnectivityTunnel` match arm.

Verification:

- Deterministic tests with a scripted `TailscaleStatusSource`: resolution yields a
  tailnet (private) endpoint with the correct scope/`permission_required`; a
  channel kind not allowed for private exposure is refused; `open_channel` then
  `close_channel` round-trips on the scripted source and on `FakeTunnel`;
  resolution output contains no secrets.
- Deterministic test asserting the enum match arms are exhaustive (compile-level)
  across all six methods and `binding()` is correct.
- `cargo test -p capo-runtime`; `cargo fmt`; live tailnet resolution deferred to
  CT10.
- `git diff --check`.

Dependencies: CT0, CT1, CT2.

## CT4 - Host/Device Identity Checks And FakeTunnel Parity

Status: pending.

Scope:

- Add tailnet device-identity verification to `TailscaleTunnel` and extend
  `FakeTunnel` so it carries the SAME surface deterministically (identity, health,
  reconnect, channel open/close) for controller/CLI tests.

Acceptance criteria:

- Before resolving a private endpoint, `TailscaleTunnel` verifies the peer/device
  identity against an expected `identity_ref` (stable tailnet device ID / node key
  fingerprint) from the endpoint config; an UNEXPECTED or unverified device yields
  a typed `ConnectivityError::IdentityMismatch` and a blocked exposure event,
  never a silent connect.
- The adapter records the OBSERVED device identity fingerprint into the
  `ResolvedEndpoint.identity_fingerprint` field added in CT2 (CT2 handle/fingerprint
  only) for audit; tailnet ACLs are treated as deployment security posture and
  `knowledge.md` records that ACLs must be reviewed before the live path.
- `FakeTunnel` gains a scriptable identity + health + reconnect + channel surface
  (e.g. `FakeTunnel::with_script(...)`) so CT5/CT7/CT9 controller and CLI tests
  can drive identity-mismatch, degraded-health, reconnect, channel-close, and
  revoke paths with NO live tailnet and NO real network.
- A deterministic test proves identity match -> resolution succeeds and identity
  mismatch -> refusal+blocked event, on BOTH the scripted Tailscale source and the
  `FakeTunnel`, demonstrating parity at the enum surface.

Verification:

- Deterministic tests: identity match/mismatch on scripted Tailscale source and on
  `FakeTunnel`; the mismatch path records a blocked exposure event (CT7 audit
  shape), not a silent failure; the matched path records the observed fingerprint
  on the resolved endpoint.
- `cargo test -p capo-runtime -p capo-cli`; `cargo fmt`; `git diff --check`.

Dependencies: CT0, CT1, CT2, CT3.

## CT5 - Tunnel Health: Heartbeat Loop (Injectable Clock), last_heartbeat_at, And Reconnect Events

Status: pending.

Scope:

- Implement tunnel health as event-sourced state: a heartbeat that updates
  `last_heartbeat_at`, projects health, and emits reconnect events, reusing the
  existing `connectivity.health_changed` event kind and `ConnectivityHealth`
  struct.
- DEFINE the heartbeat DRIVER: an injectable clock/ticker abstraction (modeled on
  `TailscaleStatusSource`) plus a named owning module so the loop's lifecycle is
  pinned, not left open. Name the crate/module that owns the heartbeat loop in
  `knowledge.md` (the same lifecycle home as anti-sleep, CT6) — it must NOT be an
  open question if CT5 is to be testable.

Acceptance criteria:

- Add a `last_heartbeat_at` field to the connectivity exposure/endpoint projection
  (mirroring `runtime_process_refs.last_heartbeat_at` from `runtime-tunnel.md`) and
  update it from `ConnectivityTunnel::check_reachability()` results; a heartbeat
  that transitions reachable->unreachable or back emits a
  `connectivity.health_changed` event (already an `EventKind` at
  `crates/capo-state/src/event.rs:25`).
- The heartbeat loop is driven by an INJECTABLE clock/ticker, not wall-clock: tests
  advance a fake clock; the live path uses a real timer. The owning module/crate is
  named and the loop's start/stop lifecycle is defined (bound to a held exposure,
  released on revoke — see CT7).
- Define reconnect semantics as events: a recovered tunnel after an unreachable
  window emits a reconnect marker (a `connectivity.health_changed` transition with
  a `reconnected` detail, or a dedicated `connectivity.reconnected` kind if a new
  kind is justified in `knowledge.md`); the heartbeat/health timeline is replayable
  and rebuilds identically.
- Health/heartbeat is computed from the tunnel surface ONLY (no process handles, no
  turn state): connectivity health must never depend on, or mutate, controller/run/
  turn state — it is a separate boundary.
- A degraded/unreachable tunnel surfaces as `health_status` + `reachable=false` on
  the exposure projection (already carries `health_status`/`reachable` in
  `crates/capo-cli/src/connectivity.rs`), and the `exposure-status`/
  `exposure-evidence` CLI surfaces show `last_heartbeat_at`.
- Heartbeat cadence is configurable and bounded; a stalled heartbeat (no update
  past a deadline) is itself a health transition, NOT a hang — proven by advancing
  the fake clock past the deadline, never by a wall-clock sleep.

Verification:

- Deterministic tests driving a scripted `FakeTunnel` health timeline plus the fake
  clock: reachable->unreachable->reconnected produces the expected ordered
  `connectivity.health_changed` events and `last_heartbeat_at` updates; the
  stall-past-deadline case advances the fake clock and asserts a transition (not a
  hang).
- Restart/replay test: the health/heartbeat timeline rebuilds identical projected
  state.
- Deterministic test asserting connectivity health does not touch run/turn read
  models.
- `cargo test -p capo-runtime -p capo-state -p capo-cli`; `cargo fmt`;
  `git diff --check`.

Dependencies: CT0, CT3, CT4.

## CT6 - Anti-Sleep When Serving Locally (Opt-In Lifecycle, One-Way Coupling, Separate From Execution)

Status: pending.

Scope:

- Add an opt-in anti-sleep capability so a laptop server stays reachable while it
  is serving a tunnel/clients, kept STRICTLY as a server-lifecycle concern separate
  from agent execution and from the tunnel adapter itself.

Acceptance criteria:

- Add a cross-platform sleep inhibitor (macOS IOKit power assertion; Linux
  `systemd-inhibit` or `gnome-session-inhibit`; no-op on unsupported platforms)
  modeled after the vendored codex `sleep-inhibitor` crate
  (`workpads/references/repos/openai-codex/codex-rs/utils/sleep-inhibitor/`), which
  uses NATIVE IOKit power assertions, NOT spawning `caffeinate`. The single rule
  for the macOS path is IOKit power assertions; there is NO `caffeinate`
  invocation. (If an IOKit escape hatch is ever genuinely needed — e.g. an IOKit
  link failure — it must be opt-in behind a documented condition and explicitly
  named, never implied as a default fallback; the vendored codex `sleep-inhibitor`
  model has no `caffeinate` path at all.) Enabled ONLY behind an explicit opt-in
  flag/env (e.g. `CAPO_SERVER_ANTI_SLEEP=1`), OFF by default.
- Anti-sleep is bound to SERVING lifecycle, not to a turn or process: it engages
  while the server holds an active non-loopback exposure (or while explicitly
  requested at server start) and releases on shutdown / last-exposure-revoked. The
  coupling direction is ONE-WAY: exposure-state -> inhibitor. The inhibitor NEVER
  reads exposure/turn/controller state back, and is NEVER coupled to
  `RuntimeRunner` or turn execution. (CT7 may CALL release on the last revoke; that
  is the permitted one-way edge.)
- The inhibitor degrades cleanly: on a platform where it cannot be enforced, Capo
  records the limitation (a `connectivity`/server lifecycle log + status field) and
  does NOT claim the laptop will stay awake — mirroring the DP7 "do not claim what
  the OS cannot enforce" discipline.
- Engage/release is observable: an anti-sleep engaged/released transition is
  recorded (status field and/or a lifecycle event) so it is auditable, and it is
  never recorded with any secret.

Verification:

- Deterministic test against a fake inhibitor backend: off-by-default (no assertion
  taken), engage on active exposure, release on revoke/shutdown, no-op on
  unsupported platform reports the limitation; assert the inhibitor never reads
  exposure/turn state back (one-way dependency).
- `cargo test` for the crate hosting the inhibitor (named in `knowledge.md`
  alongside the CT5 heartbeat owner — likely a small `capo-runtime` lifecycle
  module or `capo-server`, chosen so it does NOT couple to `RuntimeRunner`);
  `cargo fmt`.
- Live anti-sleep behavior (a real power assertion held) deferred to CT10 behind
  the opt-in gate, paired with a deterministic state assertion.
- `git diff --check`.

Dependencies: CT0 (independent of CT3-CT5; can land in parallel with the Tailscale
adapter). NOTE: CT7 takes a SOFT dependency on CT6 (see CT7) — CT7 can land with a
no-op inhibitor and wire real release when CT6 lands.

## CT7 - Auditable + Revocable Exposure: Exposure/Grant Events + Working Revoke Teardown

Status: pending.

Scope:

- Make exposure fully auditable and REVOCABLE end-to-end for the real
  `TailscaleTunnel`, extending the existing exposure event trail
  (`connectivity.exposure_requested` / `exposure_changed` / `exposure_revoked`,
  capability grants, permission approvals) and the existing `revoke-exposure` CLI
  command in `crates/capo-cli/src/connectivity.rs`.
- Teardown is defined against the surface that NOW exists after CT3: it closes the
  resolved channel via `close_channel`, stops the heartbeat (CT5), drops the
  resolved endpoint, and releases anti-sleep (CT6) if it was the last exposure.

Acceptance criteria:

- A Tailscale exposure produces the full auditable trail:
  `connectivity.exposure_requested` (blocked pending permission) -> `safety-gates`
  grant approval (existing `request-approval`/`activate-exposure` path, scope
  `network:connect:private_tunnel`) -> `connectivity.exposure_changed` (active) —
  every event carrying the endpoint/owner/channel/exposure-scope/permission-scope
  and a stable idempotency key, with NO secret in any payload.
- `revoke-exposure` for a live Tailscale exposure DOES MORE than flip status: it
  CALLS `close_channel` on the resolved channel (the surface added in CT3), stops
  the heartbeat (CT5), drops the resolved endpoint, releases anti-sleep (CT6) if it
  was the last exposure, emits `connectivity.exposure_revoked` + a terminal
  `connectivity.health_changed` (reachable=false), and a subsequent
  `check_reachability` PROVES unreachability — not merely a flag change.
- Revocation is idempotent and irreversible-within-record (the existing revoke
  already short-circuits a re-revoke); a revoked exposure cannot be reactivated
  without a new exposure + new grant.
- The audit surface is queryable: `exposure-status --latest` and
  `exposure-evidence` render the full lifecycle (requested -> active -> revoked)
  with grant id, health, `last_heartbeat_at`, and revoke reason/timestamp, and the
  evidence artifact passes the CT2 redaction guard.

Verification:

- Deterministic test driving the full lifecycle on a scripted `FakeTunnel`:
  requested(blocked) -> grant -> active -> revoke; after revoke, `check_reachability`
  reports unreachable AND `close_channel` was invoked / the channel handle is
  closed (not just status flipped).
- Restart/replay test: the exposure lifecycle rebuilds identically and a revoked
  exposure stays revoked.
- Deterministic test: revoke of the last active exposure releases anti-sleep (CT6)
  and stops the heartbeat (CT5).
- `cargo test -p capo-runtime -p capo-state -p capo-cli`; `cargo fmt`;
  `git diff --check`.

Dependencies: CT0, CT3 (channel surface), CT4, CT5 (heartbeat stop). SOFT
dependency on CT6 (anti-sleep release): CT7 may land first with a no-op inhibitor
and the real release wired when CT6 lands, so a CT6 slip does not block CT7's
core teardown. Consumes the existing `safety-gates` grant lifecycle.

## CT8 - Funnel / Public Exposure Stays Out Of Scope: Permission-Required, Clock-Swept Short-Lived, Audited Guard

Status: pending.

Scope:

- Encode the injected decision that Tailscale Funnel / public exposure is OUT OF
  SCOPE as an enforced guard, not just documentation: `ExposureScope::Public` for a
  Tailscale endpoint must require explicit permission, be short-lived, be fully
  audited, and be UNAVAILABLE without an explicit grant in the prototype profile.
- COMMIT to the expiry enforcement mechanism: the CT5 heartbeat/clock tick is the
  expiry sweep (no separate scheduler). This resolves the open question rather than
  leaving an acceptance criterion dependent on an undecided mechanism.

Acceptance criteria:

- A `public`/Funnel exposure request on `TailscaleTunnel` is REFUSED in the
  default/prototype profile (mirroring `runtime-tunnel.md`: "Tailscale
  Funnel/public exposure is out of prototype scope and must require explicit
  permission, short-lived exposure, and audit events") and the refusal is an
  audited `connectivity.exposure_requested` -> blocked event, never a silent allow.
- If a public exposure is ever permitted (behind an explicit, separately-named
  grant + opt-in), the resolved endpoint carries a REQUIRED `expires_at` (the
  `ResolvedEndpoint.expires_at` field added in CT2; short-lived, with a documented
  maximum ceiling recorded in `knowledge.md`), and expiry past the deadline
  auto-revokes via the CT7 teardown path with a `connectivity.exposure_revoked`
  event. Expiry is enforced by the CT5 heartbeat/clock tick sweep: when the
  injectable clock passes `expires_at`, the next tick triggers auto-revoke.
- The `network:expose:public` permission scope (already mapped by
  `ExposureScope::Public::permission_scope()`) is the only path; a public exposure
  without that explicit grant cannot reach `active`.
- `knowledge.md` records the threat model: public exposure is high-risk,
  default-off, short-lived, and audited; Funnel is not built in this workpad beyond
  the guard + the (gated) short-lived path.

Verification:

- Deterministic test: a Tailscale `public` exposure request in the default profile
  is refused + audited; with the explicit public grant + opt-in it resolves but
  carries `expires_at`, and advancing the fake clock (CT5) past `expires_at` fires
  the CT7 auto-revoke with a `connectivity.exposure_revoked` event.
- `cargo test -p capo-runtime -p capo-cli`; `cargo fmt`; `git diff --check`.

Dependencies: CT0, CT1, CT3, CT5 (clock-driven expiry sweep), CT7 (auto-revoke
reuses teardown).

## CT9 - Consolidated Deterministic FakeTunnel/Replay Suite (No Live Tailnet)

Status: pending.

Scope:

- Consolidate the deterministic suite that must pass with NO live tailnet and NO
  real network, asserting every connectivity invariant end-to-end and
  replay-stable.

Acceptance criteria:

- Assert the policy invariants: loopback is default and needs no auth on BOTH bind
  and connect sides; non-loopback without `auth_ref` is refused (CT1); the
  no-config default behavior is byte-for-byte unchanged (CT1); a planted secret
  never appears on any connectivity surface and a raw value in a handle field fails
  closed (CT2).
- Assert the Tailscale-via-FakeTunnel invariants: private resolution with correct
  scope; identity mismatch refused+audited and observed fingerprint recorded (CT4);
  channel open/close round-trip (CT3); health reachable->unreachable->reconnected
  timeline with `last_heartbeat_at` and a fake-clock stall-deadline transition
  (CT5); full exposure lifecycle requested->active->revoked with real
  `close_channel` teardown + proven unreachability (CT7); public/Funnel
  refused-by-default and clock-swept auto-revoke when granted (CT8).
- Assert anti-sleep state machine off-by-default/engage/release with a fake backend
  and the one-way dependency (CT6).
- Make every assertion replay-stable: a restart/rebuild reproduces identical
  projected exposure/health/audit state including the CT2 schema fields.

Verification:

- Restart/replay tests across the exposure, health, and audit paths proving
  identical rebuilds.
- `cargo test -p capo-runtime -p capo-state -p capo-cli`, widening to
  `cargo test --workspace` if shared state behavior changes broadly; `cargo fmt`;
  `git diff --check`.

Dependencies: CT1-CT8.

## CT10 - Live Opt-In Tailscale Smoke (Secrets Stripped, Defined Skip Predicate) Paired With Deterministic Assertions + Gate Review

Status: pending.

Scope:

- Add a single live, opt-in Tailscale smoke behind an explicit env gate, separate
  from ordinary test runs, paired with the deterministic assertion that pins the
  same shape, and close the workpad with a gate review.

Acceptance criteria:

- A live smoke (`#[ignore]` + `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT=1` +
  `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE=1`, mirroring
  `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` / `CAPO_SERVER_RUN_CODEX_LIVE`) resolves a
  REAL Capo-server endpoint over the live tailnet, verifies the peer device
  identity, runs the full exposure lifecycle (request -> grant -> active ->
  heartbeat -> revoke -> proven unreachable), and SKIPS CLEANLY when the gate is
  unset or the tailnet is unavailable.
- The SKIP predicate is DEFINED and deterministic, not operator-judged: the smoke
  skips when the env gate is unset, OR the `tailscale` binary is absent (resolution
  failure / non-zero exit), OR `tailscale status` reports not-logged-in / no
  reachable peers. The skip reason is recorded so "clean skip" is checkable, not
  eyeballed.
- The live smoke is paired with the SAME deterministic assertion the always-on CT9
  suite pins (resolution shape, scope, identity-checked, health transitions,
  channel-close revoke teardown), so completion is never solely operator-attested.
- Secrets are stripped from all smoke evidence: the redaction guard (CT2) scans
  artifacts/logs; any authkey/token/`tailscale status` blob is redacted (free-text
  payload) or fails closed (handle field) before retention.
- Confirm boundary fit in review notes: connectivity stays separate from execution
  (no process handles, no controller-state coupling), Funnel/public stayed out of
  scope, anti-sleep stayed a separate one-way lifecycle concern, and exposure is
  auditable + revocable; record whether to deepen (`SshTunnel`/`ReverseTunnel`/
  `RemoteProcessRunner`-over-tailnet) or close the workpad, and confirm the
  `distributed-topology` gate result.

Verification:

- Always-on deterministic gate (CT9) + the gated live Tailscale smoke paired with
  its deterministic shape assertion; cleanly skipped via the defined predicate when
  unavailable.
- `cargo test -p capo-runtime -p capo-server -p capo-cli -p capo-state`;
  `cargo fmt`; `git diff --check`.

Dependencies: CT1-CT9 landed with their deterministic suites green.
