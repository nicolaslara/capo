# Connectivity Tunnel Knowledge

## Objective

Capture decisions for the `connectivity-tunnel` workpad: implement the
`ConnectivityTunnel` boundary beyond `LocalLoopbackTunnel`/`FakeTunnel` so the
Capo server is reachable by clients AND by runtime targets that resolve THROUGH
the tunnel on other devices, with TAILSCALE as the first real tunnel adapter.
This workpad delivers REACHABILITY only; it never builds the remote runner that
executes a process on another device. Funnel/public exposure stays out of scope
behind permission + short-lived + audit guards. Connectivity stays strictly
separate from execution and controller state.

## Injected Decision (Defining Choice)

The FIRST real tunnel adapter is TAILSCALE (`TailscaleTunnel`): cross-device
reachability via the tailnet, using device identity + ACLs. This is chosen over
`SshTunnel` (the other deferred `runtime-tunnel.md` variant) because the tailnet
gives device identity, NAT traversal, and an ACL posture for free, which is the
cleanest path to "reachable from another device" without standing up a public
listener. Tailscale Funnel / public exposure is explicitly OUT OF SCOPE: it must
require explicit permission, be short-lived, and emit audit events (CT8).
`SshTunnel` and `ReverseTunnel` remain named-but-unbuilt enum variants per
`runtime-tunnel.md`.

## Scope Discipline: Reachability, Not Remote Execution

The objective is REACHABILITY, never the remote runner. "Reachable by runners on
other devices" is reframed throughout as "reachable by runtime targets that
RESOLVE through the tunnel": a `RemoteProcessRunner`/`SshRemoteProcessRunner` that
later runs a process on another device resolves its endpoint through this tunnel,
but command execution belongs to the runtime runner (`depth` DP7/DP8) even when
the transport is SSH/Tailscale — `runtime-tunnel.md` is explicit on this. The
headline objective must not edge toward remote execution; this workpad ships a
tunnel, not a runner.

## Gate Vocabulary (Corrected)

The draft cited `GATES: remote-runtime, distributed-topology`. Verified in-tree:
`remote-runtime` is a FEATURE WORKPAD FILE (`workpads/features/remote-runtime.md`),
NOT a gate, and `distributed-topology` did not exist anywhere. A workpad cannot be
routed on a feature-file name or an undefined string.

Resolution: this workpad DEFINES and registers a single NEW gate,
`distributed-topology` (the Capo server is reachable by clients and runtime
targets across devices via an auditable, revocable tunnel). CT0 owns the
registration in `workpads/WORKPADS.md` alongside the existing gate vocabulary and
verifies the registration landed. `remote-runtime` is not used as a gate.

## Boundary Ownership

This workpad owns the `ConnectivityTunnel` boundary ONLY:

- `ExposurePolicy` (loopback default, explicit opt-in, non-loopback requires
  auth) — gating BOTH the bind and the connect side.
- `auth_ref`/`identity_ref` credential HANDLES, the
  `identity_fingerprint`/`expires_at` resolved-endpoint fields, and the
  connectivity redaction guard.
- The `open_channel`/`close_channel` extension to the `ConnectivityTunnel`
  surface (so revoke can really tear down).
- `TailscaleTunnel` adapter + `FakeTunnel` parity.
- Tunnel health (heartbeat with an injectable clock, `last_heartbeat_at`,
  reconnect events).
- Anti-sleep server lifecycle (one-way exposure-state -> inhibitor).
- Auditable + revocable exposure.

It explicitly does NOT own:

- `RuntimeRunner` process lifecycle, OS sandbox, git worktrees — those are
  `depth` (DP7/DP8).
- The remote-EXECUTION runner (`RemoteProcessRunner`/`SshRemoteProcessRunner`). A
  remote runner that runs a process on another device RESOLVES its endpoint
  through this tunnel, but execution is a runtime concern, kept out of this
  workpad.
- The `PermissionPolicy`/grant lifecycle engine — `safety-gates` owns it; this
  workpad CONSUMES grants (`network:connect:private_tunnel`,
  `network:expose:public`) via the already-in-tree exposure approval/activate
  path.
- The web/`web/app` client — the web agent's.

## Current State (Verified In-Tree, 2026-06-02)

- `ConnectivityTunnel` enum is `{ Fake(FakeTunnel), LocalLoopback(LocalLoopbackTunnel),
  EndpointStub(EndpointStubTunnel) }` in `crates/capo-runtime/src/lib.rs:1737`; no
  `Tailscale`/`Ssh`/`Reverse` variant is built yet.
- The enum surface is exactly `resolve_endpoint` / `check_reachability` /
  `exposure_report` / `binding` (`lib.rs:1743-1792`). There is NO
  `open_channel`/`close_channel` in code — those appear only in the
  `runtime-tunnel.md` DESIGN. CT3 ADDS them as an owned surface extension so CT7
  can tear a channel down; CT7's teardown was unimplementable against the surface
  that exists today.
- `ExposureScope { Loopback, Private, Public }` exists with `permission_scope()`
  and `requires_permission()` (`lib.rs:1955-1966`), but there is no
  `ExposurePolicy` type — exposure ceiling/opt-in is implicit.
- `ResolvedEndpoint` (`lib.rs:2062-2099`) carries `exposure`, `permission_scope`,
  `permission_required` and NO `expires_at` / `identity_fingerprint`. The
  references claim of "`ResolvedEndpoint` carries `identity_fingerprint?` +
  `expires_at?`" describes `runtime-tunnel.md`'s design, not the code. CT2 OWNS
  adding both fields (plus `auth_ref`/`identity_ref` on the config) so CT4 (record
  fingerprint) and CT8 (require `expires_at`) have real fields to write to.
- The exposure LIFECYCLE already exists end-to-end in
  `crates/capo-cli/src/connectivity.rs`: `expose-stub`, `request-approval`
  (`capability_profile_id = remote-control-reviewed`), `activate-exposure`,
  `revoke-exposure` (flips status + health), `exposure-status`,
  `exposure-evidence`. Connectivity event kinds exist:
  `connectivity.exposure_requested/_changed/_revoked` and
  `connectivity.health_changed` (`crates/capo-state/src/event.rs:22`).
- `RedactionState::Safe` is MARKED on `connectivity.exposure_*` events
  (`connectivity.rs` lines 125/268/349/413) but NOTHING SCANS them today — the
  marker is an unverified assertion. CT2 adds the enforcing guard that makes the
  marker mean something; it does not "reuse" a guard that already exists.
- The exposure projection (`ConnectivityExposureProjection`) carries
  `health_status` + `reachable` + `revoked_at` + `capability_grant_id` +
  `permission_scope` + `status`, but NO `last_heartbeat_at` and NO real heartbeat
  producer — health is a static `check_reachability` snapshot. CT5 adds
  `last_heartbeat_at` and a clock-driven heartbeat loop.
- The server transport HARD-enforces loopback on BOTH sides: the listener guard
  (`crates/capo-server/src/transport.rs:563`, "server listener must be loopback")
  and the connect-side `connect_loopback` (~`:648`). `capo-web` binds
  `127.0.0.1:4177` (`crates/capo-web/src/main.rs:83`). CT1 replaces BOTH guards
  with an `ExposurePolicy` check (loosening one side only is an asymmetric hole).
- No `auth_ref`/`identity_ref` handle field on the endpoint config;
  `ConnectivityEndpointConfig` has `endpoint_id/name/tunnel_kind/address_ref/
  exposure/allowed_channels/status` only.
- No anti-sleep anywhere in `crates/`; the codex `sleep-inhibitor` crate is
  vendored under `workpads/references/repos/openai-codex/codex-rs/utils/sleep-inhibitor/`
  as the cross-platform model (IOKit power assertions, NOT spawning `caffeinate`).

So this workpad ADDS `ExposurePolicy`, `auth_ref`/`identity_ref` +
`identity_fingerprint`/`expires_at` schema, the `open_channel`/`close_channel`
surface, the `Tailscale` enum variant + identity/health, the clock-driven
heartbeat/reconnect loop, anti-sleep, and a real revoke teardown — on top of an
already-working exposure-grant-audit lifecycle. It does not invent the exposure
lifecycle from scratch.

## ExposurePolicy Design

- DEFAULT ceiling is `Loopback`; promotion to `Private`/`Public` is explicit
  opt-in (config/flag/grant), never implicit.
- A non-loopback bind/resolution requires an `auth_ref` handle to be present;
  without it, fail closed with a typed `AuthRequired` refusal recorded as a
  blocked exposure event.
- `ExposurePolicy` REPLACES the hard loopback guards in the server transport on
  BOTH sides — the listener guard AND `connect_loopback`. Loosening only the
  listener is an asymmetric hole; the connect side is explicitly in scope.
  Loopback keeps working with zero config (byte-for-byte unchanged default,
  pinned by regression); non-loopback requires explicit promotion + auth,
  fail-closed otherwise.
### `authorize_socket` transport-level guard — PRE-CT8 PREREQUISITE

- `ExposurePolicy::authorize_socket` is the TRANSPORT-level guard (it sees a
  `SocketAddr`, not an `ExposureScope`). It deliberately classifies any non-loopback
  socket as `ExposureScope::Private` — see the SCOPE NOTE in its doc comment. As a
  consequence, a `Public`-scope bind PASSES `authorize_socket` whenever the ceiling
  is at least `Private`; the socket layer cannot distinguish Private from Public.
- This is CORRECT for CT2 (loopback vs non-loopback is all the socket layer can
  know): the real Public/Funnel gate lives in `authorize()`, which sees the declared
  `ExposureScope` and refuses Public by default (`ScopeExceedsCeiling`).
- PRE-CT8 PREREQUISITE: when CT8 wires `TailscaleTunnel` + Funnel/public binds, the
  CT8 implementer MUST NOT treat an `authorize_socket` pass as a Public authorization.
  Funnel/public binds must additionally be gated through `authorize()` with the
  declared `ExposureScope::Public` (plus the short-lived + audited grant), or a
  Public bind that classifies as `Private` at the socket layer will silently slip the
  ceiling check. Verify this guard chain before enabling any Funnel/public path.
- `ExposureScope::requires_permission()`/`permission_scope()` remain the bridge
  into the `safety-gates` grant scopes; the policy gates BIND/CONNECT/resolution,
  the grant gates ACTIVATION — two independent checks, both required for a live
  private/public exposure.

## Auth By Handle (Never Raw, Never Logged) — Architecture First, Regex Second

- The PRIMARY guarantee is ARCHITECTURAL CONFINEMENT (a design-level structural
  commitment, NOT a Rust type-system/compile-time guarantee — the handle fields are
  `Option<String>`, so the compiler does not prevent a raw value being placed in
  one; that is why the fail-closed pattern guard exists): credentials/identity are
  referenced by HANDLE only (`auth_ref`, `identity_ref`); the raw value is resolved
  ONLY inside the adapter at connect time and is STRUCTURALLY never returned to the
  controller (no controller-facing type carries a field that holds the secret),
  never stored, never logged. A test that "a planted authkey never appears" only
  proves the
  planted PATTERNS are caught; it cannot prove an arbitrary credential is never
  emitted, so the never-logged guarantee must rest on confinement, not regex
  coverage. (CLAUDE/AGENTS: do not claim what you cannot enforce.)
- The redaction guard is the SECONDARY net, with PER-FIELD rules (not an "or"):
  - A credential-pattern match in a HANDLE field (`auth_ref`/`identity_ref`) is a
    BUG and FAILS CLOSED — refuse to persist. A raw value in a handle field must
    never be silently scrubbed, because silent scrubbing can mask a real
    programming error that was about to log a token.
  - Scrubbing/redaction is reserved ONLY for free-text payload fields where
    redaction is the documented behavior.
- This guard ADDS the enforcement behind the existing `RedactionState::Safe`
  marker (today an unverified assertion); it scans every emitted surface (event
  payload, projection field, CLI render, evidence artifact).
- Treat the Tailscale connector as a PRIVILEGED CONNECTOR (per AGENTS.md safety
  boundary), not an ordinary API key: record auth MODE + device identity
  FINGERPRINT only, mirroring the `protocol-provider.md` subscription-connector
  rule.

## Schema Extension (Owned By CT2)

CT2 owns the schema growth the rest of the workpad depends on, rather than burying
it in a verification line: `auth_ref` + `identity_ref` on
`ConnectivityEndpointConfig`/`ConnectivityEndpoint`; `identity_fingerprint` +
`expires_at` on `ResolvedEndpoint`. These propagate through the `capo-state` event
codec and the exposure projection and must be replay-stable. Because they touch the
`capo-state` schema, `capo-state` is explicitly in CT2's test scope.

## TailscaleTunnel Design

- A real `Tailscale(TailscaleTunnel)` variant resolves a Capo-server/runtime-target
  endpoint to a tailnet address (MagicDNS name or CGNAT `100.64.0.0/10` IP) at
  `ExposureScope::Private` — never loopback, never public.
- Endpoint resolution + status come through an injectable `TailscaleStatusSource`
  trait (scripted impl for deterministic tests; live `tailscale status --json` /
  LocalAPI impl for the gated path), mirroring the ACP
  `ScriptedAcpTransport`/`PipedProcessTransport` pattern from `depth`.
- Device identity is verified against an expected `identity_ref` before resolving;
  an unexpected/unverified device is an audited refusal
  (`ConnectivityError::IdentityMismatch`), not a silent connect. Through the CLI the
  refusal is RECORDED as a blocked exposure (`connectivity.exposure_requested`,
  `status = blocked_pending_permission`, `block_reason = identity_mismatch`,
  carrying the expected/observed FINGERPRINTS only) so the mismatch is auditable,
  never invisible in the log. The observed fingerprint is recorded onto
  `ResolvedEndpoint.identity_fingerprint`.
- Identity FINGERPRINT algorithm: `identity_fingerprint_of` derives a
  `tsnode:sha256:<hex>` label using SHA-256 with a `capo:tsnode:` domain separator
  (NOT the non-cryptographic FNV-1a `content_hash` used for artifact content
  addressing). The `==` comparison in the identity-mismatch gate is therefore
  collision-resistant; the `sha256:` prefix names the algorithm so the audit label
  is self-describing. The PRIMARY security gate remains the TAILNET ACL (device
  identity + ACL posture); the fingerprint is the auditable identity LABEL layered
  on top. Tailnet ACLs are deployment posture that MUST be reviewed before the live
  (CT10) path.
- The adapter never owns a process handle and never couples to `RuntimeRunner` — it
  resolves reachability/endpoints and opens/closes reachability channels only.

## Channel Surface (Owned By CT3)

CT3 adds `open_channel`/`close_channel` to the `ConnectivityTunnel` enum + every
implementing tunnel, because the in-tree surface has no channel concept and CT7's
revoke teardown ("close the resolved channel") is otherwise unimplementable. The
signatures are `open_channel(resolved_endpoint) -> ConnectivityResult<OpenChannel>`
and `close_channel(channel: OpenChannel) -> ConnectivityResult<()>`. CT3 OWNS this
naming: `runtime-tunnel.md` sketched a tentative `ChannelRef` with an unspecified
`close_channel` signature; CT3 resolves that drift to the owned `OpenChannel` type
+ the signatures above. The channel is a REACHABILITY handle, never a process
handle and never a `RuntimeRunner` coupling. `LocalLoopback`/`EndpointStub`/`Fake`
implement it coherently so the enum stays exhaustive.

## Tunnel Health, Heartbeat, Reconnect — With An Injectable Clock

- Add `last_heartbeat_at` (mirroring `runtime_process_refs.last_heartbeat_at`) to
  the exposure/endpoint projection; the heartbeat updates it and emits
  `connectivity.health_changed` on reachable/unreachable transitions. A reconnect
  after an unreachable window is a recorded transition (reuse
  `connectivity.health_changed` with a `reconnected` detail unless a dedicated kind
  is justified).
- The heartbeat is an ACTIVE periodic loop, so CT5 must pin its DRIVER: an
  injectable clock/ticker (modeled on `TailscaleStatusSource`) so the
  stall-past-deadline case is deterministic by advancing a fake clock, never a
  wall-clock sleep. The owning crate/module is NAMED in CT5 (the same lifecycle
  home as the anti-sleep inhibitor), not left as an open question — CT5 is not
  testable otherwise.
- RESOLVED (CT5): the owning module is `capo-runtime::connectivity_health`
  (`crates/capo-runtime/src/connectivity_health.rs`) — a LEAF module that depends
  ONLY on the `ConnectivityTunnel` surface (`check_reachability`) and the injectable
  clock. It deliberately does NOT depend on the controller, any run/session/turn
  read model, a state store, or `RuntimeRunner`, so connectivity health stays a
  separate boundary. The CT6 anti-sleep inhibitor will share this lifecycle home.
  The injectable clock is `ConnectivityClock` (a manual logical-ms clock for tests,
  a real monotonic clock for the live path); `HeartbeatMonitor::beat()` is a pure
  function of (tunnel, clock, config) returning a `HeartbeatOutcome` whose
  `HealthTransition` (`Initial`/`Steady`/`Lost`/`Reconnected`/`Stalled`) the
  CLI/state layer maps to a `connectivity.health_changed` event. Event emission +
  projection writes stay in the CLI/state layer (`connectivity_exposure_heartbeat`);
  the monitor emits/persists nothing.
- RESOLVED (CT5): a RECONNECT reuses `connectivity.health_changed` with a
  `reconnected` detail (a `transition` field in the payload), NOT a dedicated event
  kind — audit/replay needs only the detail. A stall past the deadline is a
  `stalled` transition (also `health_changed`), surfaced by advancing the clock.
- RESOLVED (CT5): heartbeat cadence + stall deadline are bounded config
  (`HeartbeatConfig`, default cadence 15s / stall deadline 45s = 3 missed beats),
  both clamped away from zero so the cadence cannot busy-loop and the deadline
  cannot trip every beat. Per-endpoint configurability is exposed via the
  `exposure-heartbeat --step-ms/--stall-deadline-ms` flags.
- `last_heartbeat_at` is a bare LOGICAL instant LABEL (`heartbeat-ms:<logical-ms>`)
  derived from the injectable clock, added to `ConnectivityExposureProjection`
  (carried in the projection `payload_json`, nullable column + back-fill migration),
  round-trip + restart-replay stable. It is never a credential.
- Health is computed from the tunnel surface ONLY; it must never read or mutate
  controller/run/turn state — separation of boundaries is a first-class acceptance
  criterion.
- A stalled heartbeat past its deadline is itself a health transition, never a hang.

## Anti-Sleep (Separate Lifecycle Concern, One-Way Coupling)

- Opt-in (`CAPO_SERVER_ANTI_SLEEP=1`), OFF by default. macOS IOKit power assertion
  — the single rule is IOKit power assertions with NO `caffeinate` invocation
  (the vendored codex `sleep-inhibitor` model has no `caffeinate` path at all; any
  IOKit escape hatch must be opt-in behind a documented, explicitly-named
  condition, never an implied default fallback). Linux
  `systemd-inhibit`/`gnome-session-inhibit`, no-op elsewhere — modeled on the
  vendored codex `sleep-inhibitor` crate, whose `lib.rs` states it "Uses native
  IOKit power assertions instead of spawning `caffeinate`."
- Bound to SERVING lifecycle (engaged while an active non-loopback exposure is
  held; released on shutdown / last-exposure-revoked), NOT to a turn or
  `RuntimeRunner`. The coupling direction is ONE-WAY: exposure-state -> inhibitor.
  CT7 may CALL release on the last revoke (the permitted one-way edge); the
  inhibitor NEVER reads exposure/turn/controller state back, so "separate lifecycle"
  does not quietly become bidirectional coupling.
- Degrades cleanly: on an unsupported platform Capo records the limitation and does
  NOT claim the laptop stays awake (the DP7 "don't claim what the OS can't enforce"
  discipline).
- RESOLVED (CT6): the anti-sleep lifecycle home is `capo-runtime::anti_sleep`
  (`crates/capo-runtime/src/anti_sleep.rs`) — the same leaf-module home as the CT5
  `connectivity_health` heartbeat, with NO `RuntimeRunner`/controller/run/turn
  coupling. The inhibitor is an INJECTABLE `SleepInhibitorBackend` trait (a pure
  acquire/release/capability SINK with NO exposure/turn input, so the one-way
  `exposure-state -> inhibitor` edge is structural): `FakeInhibitorBackend` for the
  deterministic suite (no OS call, no spawned process), `platform_backend()` for the
  live CT10 path (macOS IOKit / Linux `systemd-inhibit`, no `caffeinate`; no-op
  `UnsupportedBackend` elsewhere). `AntiSleepController` is OFF by default behind
  `CAPO_SERVER_ANTI_SLEEP=1`; the SERVING driver is `set_active_exposures(count)`
  (engage while > 0, release at 0). Each update returns an observable secret-free
  `AntiSleepTransition` + `AntiSleepStatus`; `keeping_awake()` is intent AND
  enforceability, so an unsupported platform reports `EngageUnsupported` + a recorded
  limitation rather than a false "awake" claim. CT7 wires the last-revoke `release`
  edge.

## Auditable + Revocable Exposure

- The full audit trail already has its event kinds and grant/approval path in-tree;
  CT7 makes the trail real for Tailscale and makes REVOKE actually tear down:
  `close_channel` (the CT3 surface), stop the clock-driven heartbeat (CT5), drop the
  resolved endpoint, release anti-sleep if last (CT6, soft), and PROVE
  unreachability via a subsequent `check_reachability` — not merely flip a status
  flag.
- Revocation is idempotent and a revoked exposure needs a fresh exposure + grant to
  come back. This satisfies the AGENTS.md "auditable and revocable" safety boundary
  as a checkable criterion, not self-attestation.
- CT7 live-teardown deferral (the honest scope of `proven_unreachable=true`): at the
  CLI tier the teardown proof runs against a scripted `FakeTunnel`, not the live
  tailnet. `TailscaleTunnel::close_channel` is a RECORDED no-op (it consumes the
  `OpenChannel` handle but makes no tailnet call), documented inline the same way
  `LiveTailscaleStatusSource::peer_status` documents its CT10 deferral. The teardown
  fake uses a `[true, false]` health timeline and probes `check_reachability` ONCE
  before the close (asserting reachable) and ONCE after (asserting unreachable), so
  the unreachability is a sequential TRANSITION attributable to the close call rather
  than a value scripted to `false` from step 0. The proof becomes CAUSAL at CT10,
  when the live `close_channel` signals the tailnet (revoke ACL tag / DisconnectPeer)
  so the post-close probe is down BECAUSE the channel was torn down. Until then,
  `proven_unreachable=true` attests the fake-tunnel transition + the recorded
  close, not a live-peer reachability change.
- CT7 "stops the heartbeat" (CT5) is enforced by the revoke state edge and proven
  end-to-end: `connectivity_exposure_heartbeat` refuses a revoked exposure with
  "connectivity exposure is revoked; no heartbeat", so the loop cannot be
  (re)started against it. The CLI test `ct7_revoke_stops_the_heartbeat` drives
  active heartbeat -> revoke -> refused heartbeat.

## Funnel / Public Out Of Scope — With A Committed Expiry Sweep

- Public/Funnel is refused-by-default and audited as a blocked request; if ever
  permitted it is behind a separately-named explicit grant
  (`network:expose:public`), carries a REQUIRED short-lived `expires_at` (CT2 field,
  with a documented maximum ceiling), and auto-revokes at expiry through the CT7
  teardown.
- Expiry enforcement is COMMITTED to the CT5 heartbeat/clock tick as the sweep (no
  separate scheduler): when the injectable clock passes `expires_at`, the next tick
  triggers auto-revoke. This resolves the prior open question and adds a CT5
  dependency to CT8. Funnel itself is not built beyond this guard + the gated
  short-lived path.

## Verification Discipline

Deterministic `FakeTunnel`/replay tests land before any live Tailscale path. Every
manual smoke is paired with a deterministic assertion (resolution shape,
exposure-event shape, restart/replay). The live Tailscale smoke is opt-in behind
`CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT` + `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE`
(mirroring `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` / `CAPO_SERVER_RUN_CODEX_LIVE`) and
`#[ignore]`/skips cleanly. The skip is DETERMINISTIC, not operator-judged: skip when
the gate is unset, OR the `tailscale` binary is absent, OR `tailscale status`
reports not-logged-in / no reachable peers; the skip reason is recorded. No
criterion rests on operator self-attestation; secrets are stripped from all
evidence.

## Non-Goals

- No SSH tunnel, no reverse tunnel, no Funnel/public listener built in this workpad
  (named-but-unbuilt enum variants + the public guard only).
- No remote-EXECUTION runner here (`RemoteProcessRunner`/`SshRemoteProcessRunner`
  stays a `depth`/runtime concern; it merely resolves through this tunnel later).
- Do not change the `safety-gates` grant/permission engine; consume it.
- Do not couple connectivity to `RuntimeRunner`, process handles, or
  controller/turn state. The CT6 anti-sleep coupling is strictly one-way
  (exposure-state -> inhibitor).
- Never store or log raw authkeys/tokens/cookies/session files; handles +
  fingerprints only, with architectural confinement as the primary guarantee.
- Do not claim a laptop stays awake on a platform where anti-sleep cannot be
  enforced.
- No web client.

## Open Questions

- Does the live `TailscaleStatusSource` use the `tailscale` CLI
  (`tailscale status --json`) or the Tailscale LocalAPI socket directly? (Leaning:
  CLI first for portability, LocalAPI if the CLI proves too coarse for
  device-identity verification.) Resolved enough to build the SCRIPTED source now;
  the live choice is pinned at CT3/CT10.
- RESOLVED (CT5): `connectivity.reconnected` is NOT a dedicated event kind — a
  `connectivity.health_changed` transition with a `reconnected` detail is sufficient
  for replay/audit.
- RESOLVED (CT5): heartbeat cadence default 15s, stall deadline default 45s (3
  missed beats), both bounded away from zero; per-endpoint configurable via the
  `exposure-heartbeat --step-ms`/`--stall-deadline-ms` flags.
- What is the maximum allowed `expires_at` ceiling for a (gated) short-lived public
  exposure? (Mechanism is RESOLVED — the CT5 heartbeat/clock tick is the sweep; only
  the numeric ceiling remains, pinned at CT8.)
- (none currently open)

RESOLVED (formerly open):
- WHO runs the heartbeat loop / WHERE the anti-sleep inhibitor lives: a named
  lifecycle module (CT5 names it; same home as the inhibitor) chosen so it does NOT
  couple to `RuntimeRunner`. No longer an open question — CT5 requires it pinned to
  be testable.
- WHETHER `expires_at` expiry uses a heartbeat sweep or a scheduler: COMMITTED to
  the CT5 heartbeat/clock tick sweep (CT8).
- WHETHER `ExposurePolicy` promotion is itself an audited event: RESOLVED — YES.
  Promotion of the effective exposure ceiling (Loopback -> Private/Public) MUST
  emit a `connectivity.policy_changed` audit event, because an operator must be
  able to reconstruct WHY a private/public exposure became possible. The
  per-exposure `connectivity.exposure_requested` trail records that an exposure was
  blocked or granted, but not why the policy ceiling that permitted it changed; a
  policy promotion without an event is not "auditable and revocable" in the
  AGENTS.md sense. This is committed into CT1's acceptance criteria (the promotion
  path emits `connectivity.policy_changed` carrying the old/new ceiling, the
  opt-in source — config/flag/grant — and a timestamp, with no secret in the
  payload). The event is replay-stable like the other connectivity event kinds.
