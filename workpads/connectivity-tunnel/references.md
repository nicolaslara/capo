# Connectivity Tunnel References

## Objective

Record the local and external sources that shape the `connectivity-tunnel`
workpad: the `ExposurePolicy`, `auth_ref`/`identity_ref` handles, the
`ResolvedEndpoint` schema growth, the `open_channel`/`close_channel` surface
extension, the `TailscaleTunnel` adapter, tunnel health/heartbeat/reconnect (with
an injectable clock), anti-sleep, and auditable+revocable exposure. Dated claims
reflect 2026-06-02.

## Local Architecture Sources

- `workpads/architecture/runtime-tunnel.md`
  - Key facts: THE design this workpad implements. `RuntimeRunner` (execution) and
    `ConnectivityTunnel` (reachability) are SEPARATE boundaries; the tunnel "does
    not own task/session state or process handles." Connectivity enum is
    `LocalLoopback`/`Ssh`/`Tailscale`/`Reverse`/`Fake`; only `LocalLoopback`+`Fake`
    were required for the first prototype. `ConnectivityEndpoint` carries
    `tunnel_kind`, `address_ref`, `identity_ref?`, `auth_ref?`,
    `exposure: loopback|private|public`; "`auth_ref` points to a secret handle or
    OS/vendor credential location, never raw credential material." `ResolvedEndpoint`
    carries `identity_fingerprint?` + `expires_at?` IN THE DESIGN — NOTE these are
    design fields, NOT yet in code; CT2 adds them (see implementation source below).
    Tunnel contract IN THE DESIGN: `resolve_endpoint`/`check_reachability`/
    `open_channel`/`close_channel`/`exposure_report` — but the in-tree enum has NO
    `open_channel`/`close_channel`; CT3 adds them. `TailscaleTunnel` provides tailnet
    identity + private endpoint resolution; "Tailnet ACLs become part of deployment
    security posture and must be reviewed before remote dogfood"; "Tailscale
    Funnel/public exposure is out of prototype scope and must require explicit
    permission, short-lived exposure, and audit events."
    `runtime_process_refs.last_heartbeat_at` is the heartbeat field shape to mirror.
    Connectivity events: `connectivity.endpoint_registered/_updated/_resolved`,
    `connectivity.health_changed`, `connectivity.channel_opened/_closed`,
    `connectivity.exposure_changed`.
- `workpads/architecture/boundaries.md`
  - Key facts: the controller owns the loop; the `ConnectivityTunnel` sits as a
    boundary below it; connectivity is kept modular and separate from execution,
    controller, provider, and state per the workpad load rules in
    `workpads/WORKPADS.md`.
- `AGENTS.md` (Safety Boundary)
  - Key facts: never log API keys/subscription tokens/OAuth tokens/cookies/session
    files; treat subscription-backed access as a PRIVILEGED CONNECTOR not an
    ordinary API key; keep tunnel/connectivity concerns SEPARATE from agent
    execution and controller state; make remote-control capabilities AUDITABLE and
    REVOCABLE; do not claim what the OS cannot enforce.
- `workpads/depth/tasks.md` + `knowledge.md` + `references.md`
  - Key facts: the FORMAT EXEMPLAR for this workpad's house style (Objective;
    numbered tasks with Scope/Acceptance/Verification/Dependencies; differentiated
    per-task prerequisites; deterministic-before-live; opt-in env gates mirroring
    `CAPO_SERVER_RUN_CODEX_LIVE`/`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`;
    `#[ignore]`/skip-clean live smokes; secrets stripped; the
    scripted-transport-vs-live-transport pattern used for ACP — the model for both
    `TailscaleStatusSource` and the injectable heartbeat clock). NOTE: `depth`
    tasks.md carries NO `GATES:` line — confirming the draft's `GATES:` was an
    invention; this workpad declares a single defined gate instead.

## Local Product And Implementation Sources

- `crates/capo-runtime/src/lib.rs`
  - Key facts: `ConnectivityTunnel` enum (`:1737`) is `{ Fake, LocalLoopback,
    EndpointStub }` with EXACTLY `resolve_endpoint`/`check_reachability`/
    `exposure_report`/`binding` match arms (`:1743-1792`) — there is NO
    `open_channel`/`close_channel`; CT3 adds both as an owned surface extension and
    the `Tailscale` arm. `ExposureScope { Loopback, Private, Public }`
    (`:1949-1966`) with `permission_scope()` (`network:connect:localhost` /
    `network:connect:private_tunnel` / `network:expose:public`) and
    `requires_permission()` (Loopback=false, else true) — CT1 adds `ExposurePolicy`
    over this. `ConnectivityEndpointConfig` (`:2017`) has `endpoint_id/name/
    tunnel_kind/address_ref/exposure/allowed_channels/status` and NO
    `auth_ref`/`identity_ref` (CT2 adds them). `ResolvedEndpoint` (`:2062-2099`)
    carries `exposure`/`permission_scope`/`permission_required` and NO
    `expires_at`/`identity_fingerprint` (CT2 adds them; CT4 writes the fingerprint,
    CT8 requires `expires_at`). `ConnectivityHealth` (`:2105+`), `ExposureReport`,
    `EndpointOwner` (`runtime_target`/`capo_server`), `ChannelKind`
    (`Control/Stdio/Logs/Dashboard/Artifact` with `is_loopback_safe`).
- `crates/capo-cli/src/connectivity.rs`
  - Key facts: the EXISTING exposure lifecycle CT7/CT8 extend —
    `expose_connectivity_stub` (resolves via `ConnectivityTunnel`, records
    `connectivity.exposure_requested`/`_changed`, blocks non-loopback pending
    permission), `request_connectivity_exposure_approval` (queues
    `PermissionApproval`, `capability_profile_id = remote-control-reviewed`, `:236`),
    `activate_connectivity_exposure` (matches an allow grant on the exposure's
    `permission_scope`), `revoke_connectivity_exposure` (`:363`; flips status/health
    today — CT7 makes this a real `close_channel` teardown),
    `connectivity_exposure_status`. All events SET `RedactionState::Safe` (lines
    125/268/349/413) but NOTHING SCANS them — the marker is an unverified assertion;
    CT2 adds the enforcing guard.
- `crates/capo-cli/src/connectivity_evidence.rs`
  - Key facts: `connectivity_exposure_evidence` renders the exposure lifecycle to an
    artifact — CT7 extends it with `last_heartbeat_at`/grant/revoke detail and CT2's
    redaction guard scans the artifact.
- `crates/capo-server/src/transport.rs`
  - Key facts: `serve_tcp_with_handler` HARD-rejects a non-loopback bind (`:563`,
    "server listener must be loopback"); `connect_loopback` (~`:648`) enforces
    loopback-only on the CONNECT side. CT1 replaces BOTH guards with an
    `ExposurePolicy` check (loosening only the listener is an asymmetric hole;
    connect side is explicitly in scope). A regression pins the no-config default
    byte-for-byte unchanged on both sides.
- `crates/capo-web/src/main.rs`
  - Key facts: binds `CAPO_WEB_ADDR` defaulting to `127.0.0.1:4177` (`:83`) — an
    existing caller of the loopback transport path that CT1's regression must prove
    UNAFFECTED by the policy swap; the loopback default a policy-promoted private
    bind would later change for tailnet reachability.
- `crates/capo-state/src/event.rs`
  - Key facts: connectivity event kinds already exist —
    `ConnectivityExposureRequested`/`ExposureChanged`/`ExposureRevoked`/`HealthChanged`
    (`:22`, wire strings `connectivity.exposure_requested/_changed/_revoked/
    .health_changed` at `:161`); `RedactionState` lives here (`:379`). CT5 reuses
    `HealthChanged` for heartbeat/reconnect; CT7 reuses the exposure kinds; a
    `connectivity.reconnected` kind is an open question. CT2's
    `auth_ref`/`identity_ref`/`identity_fingerprint`/`expires_at` schema additions
    touch this codec — hence `capo-state` is in CT2's test scope.
- `crates/capo-state/src/projections.rs` + `queries.rs`
  - Key facts: `ConnectivityExposureProjection` carries `health_status`/`reachable`/
    `revoked_at`/`capability_grant_id`/`permission_scope`/`status` but NO
    `last_heartbeat_at` (CT5 adds it); `connectivity_exposures`/
    `latest_connectivity_exposure` queries back `exposure-status`/`-evidence`.
- `crates/capo-cli/src/main.rs`
  - Key facts: `connectivity` subcommand routing (`:326`) for `expose-stub`/
    `request-approval`/`activate-exposure`/`revoke-exposure`/`exposure-status`/
    `exposure-evidence` — the surface CT7 extends (teardown) and where CT6
    anti-sleep status / CT5 heartbeat surfaces are rendered.
- `crates/capo-server/src/live_provider.rs` (+ `util.rs`)
  - Key facts: the opt-in gate PATTERN to mirror — `live_execution_opt_in` +
    `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`, with `CAPO_SERVER_RUN_CODEX_LIVE` as the
    named live gate. CT10's `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT` +
    `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE` mirror this exactly; CT10 ADDS a defined
    skip predicate (binary absent / not-logged-in / no peers) so "clean skip" is
    deterministic.

## Routing / Gate Sources (CT0)

- `workpads/WORKPADS.md`
  - Key facts: the workpad registry + per-workpad `Load:`/`Rules:` blocks and the
    gate vocabulary surface. CT0 ADDS the `connectivity-tunnel` entry (with `Load:`
    + `Rules:`) and DEFINES + registers the new `distributed-topology` gate here.
    Verified 2026-06-02: `connectivity-tunnel` is absent today and must be written.
- `TASKS.md`
  - Key facts: tracks the active workpad track + checklist; CT0 ADDS
    `connectivity-tunnel` with the `CT` prefix. Verified 2026-06-02:
    `connectivity-tunnel` is absent today.
- `workpads/features/remote-runtime.md`
  - Key facts: this is a FEATURE WORKPAD FILE, NOT a gate. The draft's
    `GATES: remote-runtime` was invalid; `remote-runtime` is not used as a gate.
    `distributed-topology` did not exist anywhere in `workpads/` or `AGENTS.md` and
    is DEFINED new by CT0.

## External Sources

- OpenAI codex `sleep-inhibitor` crate
  (`workpads/references/repos/openai-codex/codex-rs/utils/sleep-inhibitor/`)
  - Observed 2026-06-02.
  - Key facts: the cross-platform anti-sleep model for CT6. `lib.rs` exposes a
    `SleepInhibitor { enabled, turn_running, platform }` with
    `set_turn_running(bool)`/`release()`; `lib.rs:4` states it "Uses native IOKit
    power assertions instead of spawning `caffeinate`" — so the CT6 macOS path is
    IOKit assertions with NO `caffeinate` invocation (the model crate has no
    `caffeinate` path at all). Platform
    backends are `macos.rs` (IOKit power assertions via `iokit_bindings.rs`),
    `linux_inhibitor.rs` (`systemd-inhibit`/`gnome-session-inhibit`),
    `windows_inhibitor.rs` (`PowerCreateRequest`/`PowerSetRequest`), and `dummy.rs`
    (no-op on unsupported platforms). Capo binds engage/release to SERVING lifecycle
    (active exposure) rather than `turn_running`, ONE-WAY (exposure-state ->
    inhibitor), keeping it separate from execution.
- Tailscale CLI / LocalAPI (`tailscale status --json`)
  - To be confirmed at CT3/CT10 against the installed `tailscale` on the dev box
    (Darwin).
  - Key facts (to verify): `tailscale status --json` exposes self + peer device
    identity (node keys, stable IDs), MagicDNS names, and tailnet `100.64.0.0/10`
    CGNAT addresses for endpoint resolution + device-identity checks; tailnet ACLs
    govern which devices may reach the Capo server and are deployment posture
    reviewed before the live path. The CT10 skip predicate keys off `tailscale`
    binary absence / `tailscale status` not-logged-in / no-peers. Funnel/public
    exposure is a distinct, explicit, high-risk capability kept out of scope (CT8).
    Record exact JSON field shapes and the LocalAPI-vs-CLI decision when the live
    path is built.

## Notes On The House-Style Fit

This workpad mirrors `depth` exactly: an Objective, numbered `CT0..CT10` tasks each
with Scope / Acceptance criteria (concrete + checkable) / Verification
(deterministic-first, live opt-in gated) / Dependencies (intra- and
cross-workpad), a `knowledge.md` recording the injected decision + rationale +
non-goals + open questions, and a `references.md` separating local architecture,
local implementation, routing/gate, and dated external sources. Like `depth`,
prerequisites are differentiated per task (CT1-CT2 stand alone on the existing
exposure lifecycle + own the schema growth; CT3-CT5 build the Tailscale adapter +
channel surface + clock-driven health; CT6 is independent additive lifecycle with a
soft edge into CT7; CT7-CT8 close the audit/revoke + out-of-scope guards; CT9-CT10
consolidate determinism then the single gated live smoke), and no live network path
runs without an explicit env gate that skips on a DEFINED predicate. Schema and
surface changes (`auth_ref`/`identity_ref`/`identity_fingerprint`/`expires_at`,
`open_channel`/`close_channel`) are owned by explicit tasks, not assumed by a
verification line.
