# Remote Runtime And Tunnel Feature

## Objective

Add remote execution and connectivity adapters without collapsing the separation between runtime process ownership and tunnel/exposure mechanics.

## Prototype Inputs

- P5 proved local process lifecycle and redacted output artifacts.
- Architecture keeps `RuntimeRunner` and `ConnectivityTunnel` separate.
- P15 defers remote runtime until local real-agent semantics are reliable.

## Dependencies

- Real local Codex/Claude semantics should be proven before remoteizing them.
- Public exposure requires explicit permission and audit events.

## Tasks

### RR1 - Remote Runtime Contract Test

Status: completed

Acceptance:

- Add a fake or loopback remote runtime contract test that proves runtime refs, health, interrupt, terminate, and recovery events.
- Do not require Tailscale or cloud credentials.

Evidence:

- `RemoteProcessRunner` and `RemoteProcessConfig::loopback_for_test` in `../../crates/capo-runtime/src/lib.rs`.
- Loopback remote process refs include remote target and endpoint identity separately from local process artifacts.
- Runtime events include `runtime.remote_target_resolved`, `runtime.remote_process_started`, `runtime.remote_interrupt_sent`, and `runtime.remote_terminate_sent`.
- Recovery reports `remote_recovered` for live refs and `remote_orphaned` for non-live refs.
- `cargo test -p capo-runtime remote_runtime -- --nocapture`: passed.

### RR2 - Tunnel Adapter Stub

Status: completed

Acceptance:

- Add a tunnel adapter stub for endpoint resolution and health.
- Keep tunnel records separate from runtime process records.
- Record exposure scope and permission requirements.

Evidence:

- `EndpointStubTunnel`, `ConnectivityEndpointConfig`, `ResolvedEndpoint`, `ConnectivityHealth`, and `ExposureReport` in `../../crates/capo-runtime/src/lib.rs`.
- Endpoint resolution records `connectivity_endpoint_id`, typed owner, channel kind, resolved URI, exposure scope, permission scope, and whether permission is required.
- Runtime process refs remain in `RemoteProcessRunner` / `LocalRuntimeProcessRef`; tunnel health and exposure data remain in connectivity-only records.
- Loopback exposure resolves only loopback-safe channels and does not require remote/public permission.
- Private/public stub exposures report `network:connect:private_tunnel` or `network:expose:public` permission requirements and `connectivity.exposure_changed` audit kind.
- `cargo test -p capo-runtime tunnel -- --nocapture`: passed.

### RR3 - Explicit Exposure Policy

Status: completed

Acceptance:

- Require durable permission events before public or remote-control exposure.
- Make revocation and health visible in read models.

Evidence:

- `ConnectivityExposureProjection`, `EventKind::ConnectivityExposureRequested`, `EventKind::ConnectivityExposureChanged`, `EventKind::ConnectivityExposureRevoked`, and `EventKind::ConnectivityHealthChanged` in `../../crates/capo-state/src/lib.rs`.
- Exposure read models include endpoint ID, owner, channel, exposure scope, permission scope, status, linked capability grant, health status, reachability, and revocation timestamp.
- Regression test proves remote-control exposure starts as `blocked_pending_permission`, becomes `active` only after a durable `capability.grant_created` event/projection, then rebuilds as `revoked` with disabled health.
- `cargo test -p capo-state connectivity_exposure -- --nocapture`: passed.

### RR4 - Dashboard Exposure Visibility

Status: completed

Acceptance:

- Expose connectivity exposure rows through the shared dashboard/query surface.
- Render exposure status, health, permission scope, and grant/revocation state in the CLI dashboard.
- Keep dashboard rendering read-only; approval decisions remain in the permission queue.

Evidence:

- `ProjectDashboard.connectivity_exposures` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo dashboard` renders exposure ID, endpoint, owner, channel, exposure scope, status, health, reachability, permission scope, grant ID, and revocation timestamp from the shared query contract.
- `cargo test -p capo-query connectivity -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_connectivity -- --nocapture`: passed.
