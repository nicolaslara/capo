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

### RR5 - Connectivity Exposure Operator Surface

Status: completed

Acceptance:

- Add an operator command that resolves a stub connectivity endpoint and records an exposure row without opening a real tunnel.
- Keep provider/runtime execution untouched: no agent process launch, no provider CLI execution, and no remote credentials.
- Fail closed for private/public exposure by recording `blocked_pending_permission` until a separate durable grant exists.
- Reuse the shared dashboard/read-model path for recorded exposure status.

Evidence:

- CLI `capo connectivity expose-stub --endpoint ENDPOINT_ID --owner-kind runtime_target|capo_server --owner-id OWNER_ID --channel control|stdio|logs|dashboard|artifact --exposure loopback|private|public [--record]` in `../../crates/capo-cli/src/main.rs`.
- Private stub exposure records `permission_scope=network:connect:private_tunnel` and `status=blocked_pending_permission`.
- Public stub exposure rejects disallowed channels before recording.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Treat this as an exposure-planning and audit surface, not actual remote networking. Concrete SSH/Tailscale/cloud adapters remain deferred until real local-agent semantics are proven.
- The command records only endpoint/owner/channel/scope/status/health metadata. It does not create tunnels, run provider CLIs, inspect credentials, or mutate runtime process refs.

### RR6 - Connectivity Exposure Approval Bridge

Status: completed

Acceptance:

- Add an operator path that queues a permission approval from a blocked connectivity exposure row.
- Activate a blocked exposure only after a matching durable allow grant exists.
- Keep activation as metadata/read-model state only: no tunnel creation, runtime process launch, provider CLI execution, or credential inspection.
- Cover the full blocked -> approval -> grant -> active dashboard path with a CLI regression test.

Evidence:

- CLI `capo connectivity request-approval --exposure EXPOSURE_ID [--approval APPROVAL_ID]` in `../../crates/capo-cli/src/main.rs`.
- CLI `capo connectivity activate-exposure --exposure EXPOSURE_ID` in `../../crates/capo-cli/src/main.rs`.
- Activation requires an allow grant whose scope includes the exposure permission scope and whose subject contains the exposure ID, endpoint, owner, channel, and exposure scope.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Reuse the existing `permission request` / `permission decide` event and grant machinery instead of adding a connectivity-specific policy store.
- Treat `activate-exposure` as an audited state transition from blocked to active. It does not prove real network reachability beyond the stub health metadata recorded by the endpoint adapter.

### RR7 - Connectivity Exposure Revocation Surface

Status: completed

Acceptance:

- Add an operator command that revokes an active or pending connectivity exposure row.
- Preserve the linked grant for audit while marking the exposure disabled and unreachable.
- Keep revocation as state/audit metadata only: do not manage real tunnels, runtime processes, provider CLIs, or credentials.
- Show revoked status, disabled health, reachability, and revocation time through the existing dashboard path.

Evidence:

- CLI `capo connectivity revoke-exposure --exposure EXPOSURE_ID [--reason REASON]` in `../../crates/capo-cli/src/main.rs`.
- Revocation records `EventKind::ConnectivityExposureRevoked` and projects `status=revoked`, `health_status=disabled`, `reachable=false`, and `revoked_at`.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Keep capability grants append-only for now. Revocation changes exposure availability, not historical grant evidence.
- Use the existing dashboard row as the operator visibility surface for revoked exposure state.

### RR8 - Connectivity Exposure Evidence Export

Status: completed

Acceptance:

- Add a provider-free command that exports a Capo-owned evidence artifact for a recorded connectivity exposure.
- Include endpoint, owner, channel, exposure scope, permission scope, status, health, reachability, linked grant, and revocation state.
- Record the exported markdown as project-level evidence so dashboards and future dogfood readiness checkpoints can inspect it.
- Do not open tunnels, launch runtimes or providers, inspect credentials, or mutate exposure state.

Evidence:

- CLI `capo connectivity exposure-evidence --exposure EXPOSURE_ID --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Exported artifacts use the `<!-- capo:connectivity-exposure-evidence -->` marker and guarded overwrite behavior.
- Artifact records use `kind=connectivity_exposure_evidence`; evidence projections use `kind=connectivity_exposure_evidence`.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Treat connectivity exposure evidence as an operator review artifact, not proof of real tunnel reachability.
- Keep the export read-model-derived and provider-free. It records endpoint/owner/channel/scope/status/health/grant/revocation metadata only, without opening tunnels, launching runtimes, launching provider CLIs, inspecting credentials, or mutating exposure state.
