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

### RR9 - Latest Connectivity Exposure Status

Status: completed

Acceptance:

- Add a shared query helper that selects the latest recorded connectivity exposure without requiring the operator to know an exposure ID.
- Support optional owner and channel filters for remote-control surfaces that need the latest exposure for a runtime target, Capo server, or dashboard channel.
- Expose exact and latest connectivity exposure status through a read-only operator command.
- Do not open tunnels, launch runtimes or providers, inspect credentials, request approvals, activate grants, revoke exposure, or mutate state.

Evidence:

- `ProjectDashboard::connectivity_exposure_status(...)` and `ProjectDashboard::latest_connectivity_exposure(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo connectivity exposure-status --exposure EXPOSURE_ID` in `../../crates/capo-cli/src/main.rs`.
- CLI `capo connectivity exposure-status --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel control|stdio|logs|dashboard|artifact]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query latest_connectivity -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Treat latest connectivity exposure selection as query/read-model behavior. The selector uses the newest exposure projection sequence, with exposure ID as a deterministic tie breaker.
- Keep exact `--exposure` lookup and latest `--latest` lookup mutually exclusive. Owner and channel filters are only valid for latest lookup.
- Keep status read-only. It does not open tunnels, launch runtimes or providers, inspect credentials, request approvals, activate grants, revoke exposure, or mutate state.

### RR10 - Latest Connectivity Exposure Evidence Export

Status: completed

Acceptance:

- Add a provider-free latest-selector path for connectivity exposure evidence export.
- Support the same optional owner/channel filters as latest exposure status.
- Reuse the Capo-marked connectivity exposure evidence artifact format and guarded writer.
- Do not open tunnels, launch runtimes or providers, inspect credentials, request approvals, activate grants, revoke exposure, or mutate exposure state.

Evidence:

- CLI `capo connectivity exposure-evidence --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel control|stdio|logs|dashboard|artifact] --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Latest export reuses `ProjectDashboard::latest_connectivity_exposure(...)` and the existing Capo-marked evidence renderer/writer.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Treat latest evidence export as operator ergonomics over the shared connectivity exposure selector. It selects the latest matching exposure, then writes the same Capo-owned connectivity exposure evidence artifact as exact export.
- Keep exact `--exposure` and latest `--latest` mutually exclusive; owner/channel filters are only valid with `--latest`.
- Keep the export read-model-derived and provider-free. It does not open tunnels, launch runtimes or providers, inspect credentials, request approvals, activate grants, revoke exposure, or mutate exposure state.

### RR11 - Runtime Target Inventory

Status: completed

Acceptance:

- Add a first-class persisted runtime target read model so connectivity exposures can point at known execution-machine metadata instead of opaque owner IDs only.
- Keep runtime target metadata separate from connectivity exposure rows and adapter/provider dispatch plans.
- Expose a provider-free operator surface for registering and listing runtime targets.
- Render runtime targets through the shared dashboard/query path.
- Do not launch runtimes, launch provider CLIs, inspect credentials, open tunnels, or activate exposure.

Evidence:

- `RuntimeTargetProjection`, `EventKind::RuntimeTargetRegistered`, SQLite `runtime_targets`, projection-log encode/decode, read query, and rebuild coverage in `../../crates/capo-state/src/lib.rs`.
- `ProjectDashboard.runtime_targets` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo runtime target register ...` and `capo runtime target list` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state runtime_targets -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime targets as execution placement metadata: runner kind, workspace root, artifact root, default cwd, capability profile, optional connectivity endpoint, and health/status label.
- Keep actual process execution in runtime requests/outcomes and keep reachability in connectivity exposure records. A runtime target can be listed without any tunnel or provider process existing.
- Use this registry as the stable owner side for later SSH/Tailscale/cloud target adapters and for dashboard/operator review before real remote execution is enabled.

### RR12 - Runtime Target Exposure Validation

Status: completed

Acceptance:

- Fail closed before recording a connectivity exposure whose owner is `runtime_target` unless the runtime target exists in Capo state.
- Keep dry-run exposure planning available for inspection without mutating state.
- Keep `capo_server` owner exposure independent from runtime target inventory.
- Do not launch runtimes, launch provider CLIs, inspect credentials, open tunnels, request approvals, activate grants, or mutate runtime target state.

Evidence:

- Recorded `capo connectivity expose-stub --owner-kind runtime_target ... --record` validates the owner against `SqliteStateStore::runtime_targets(...)` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime target registration as the durable execution-machine inventory. Connectivity exposure rows for runtime owners must point at known targets so later SSH/Tailscale/cloud adapters and dashboards are not built on opaque strings.
- Keep the validation at the CLI/operator write surface for now because runtime targets and connectivity exposures are still metadata-only feature scaffolding. A future service/controller write path should enforce the same invariant closer to the command handler.

### RR13 - Runtime Target Endpoint Consistency

Status: completed

Acceptance:

- Fail closed before recording a `runtime_target` connectivity exposure when the registered runtime target has a configured connectivity endpoint and the requested exposure uses a different endpoint.
- Allow targets without a configured endpoint to remain flexible for early metadata scaffolding.
- Keep the validation metadata-only and do not open tunnels, launch runtimes, launch providers, inspect credentials, request approvals, activate grants, or mutate runtime target state.

Evidence:

- Recorded `capo connectivity expose-stub --owner-kind runtime_target ... --record` compares the requested endpoint against the registered runtime target endpoint when present in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat a runtime target's configured connectivity endpoint as a binding constraint once present. This prevents operator dashboards and future remote adapters from mixing execution-machine metadata with unrelated tunnel endpoints.
- Keep endpoint-less targets valid for now because remote target discovery and real tunnel adapters are still deferred behind local real-agent proof.

### RR14 - Runtime Target Availability Guard

Status: completed

Acceptance:

- Fail closed before recording a `runtime_target` connectivity exposure unless the registered runtime target is `available`.
- Keep disabled or unhealthy targets visible in the runtime target inventory without allowing exposure writes.
- Keep the validation metadata-only and do not open tunnels, launch runtimes, launch providers, inspect credentials, request approvals, activate grants, or mutate runtime target state.

Evidence:

- Recorded `capo connectivity expose-stub --owner-kind runtime_target ... --record` checks registered runtime target status in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime target status as an exposure precondition. If a target is disabled or unhealthy, Capo should not create new connectivity exposure records for it even when endpoint metadata matches.
- Keep target status changes as future metadata work; this slice only enforces the status already stored in the runtime target inventory.

### RR15 - Runtime Target Status Update Surface

Status: completed

Acceptance:

- Add a provider-free operator command for changing runtime target status between `available`, `disabled`, and `unhealthy`.
- Preserve all other runtime target metadata while updating status.
- Keep the status update in the runtime target read model and event log, separate from connectivity exposure state.
- Prove the updated status affects the recorded connectivity exposure guard without launching runtimes, launching providers, opening tunnels, inspecting credentials, requesting approvals, or activating grants.

Evidence:

- CLI `capo runtime target set-status --target TARGET_ID --status available|disabled|unhealthy` in `../../crates/capo-cli/src/main.rs`.
- `EventKind::RuntimeTargetStatusChanged` in `../../crates/capo-state/src/lib.rs`.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime target status changes as metadata transitions over the target inventory, not runtime lifecycle operations.
- Reuse the existing `RuntimeTargetProjection` upsert path so dashboards and exposure validation see the latest target state through the same read model.

### RR16 - Runtime Target Status Query Surface

Status: completed

Acceptance:

- Add a shared query helper that selects an exact runtime target by ID from the runtime target read model.
- Expose a read-only operator command for inspecting one runtime target's latest placement/status metadata.
- Render the same metadata shape as runtime target list/dashboard rows so future voice/web/mobile surfaces have a stable contract.
- Return a clear error for an unknown runtime target.
- Do not launch runtimes, launch providers, inspect credentials, open tunnels, request approvals, activate grants, or mutate state.

Evidence:

- `ProjectDashboard::runtime_target_status(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo runtime target status --target TARGET_ID` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat exact runtime target status as shared query/read-model behavior so CLI, dashboard, voice, web, and mobile surfaces can reuse the same selector.
- Keep the operator command read-only. It renders placement/status metadata and explicitly reports that provider CLIs, tunnels, runtime processes, and state mutation were not used.
- Keep missing-target behavior fail-closed with a clear operator error instead of returning an empty status row.

### RR17 - Voice Runtime Target Status Query

Status: completed

Acceptance:

- Add a voice/input intent for asking Capo about one runtime target's status.
- Lower the utterance into a read-only command envelope and shared runtime target status read scope.
- Render the target's latest placement/status metadata through the same dashboard query surface used by CLI status.
- Return a clear spoken missing-target row when the target is unknown.
- Do not launch runtimes, launch providers, inspect credentials, open tunnels, request approvals, activate grants, retain raw transcripts, or mutate state.

Evidence:

- Voice runtime-target status intent, read scope, parser, and regression coverage in `../../crates/capo-voice/src/lib.rs`.
- CLI voice runtime-target status rendering and regression coverage in `../../crates/capo-cli/src/main.rs`.
- Shared exact target selector in `../../crates/capo-query/src/lib.rs`.
- `cargo test -p capo-voice runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat voice runtime target status as a read-only input surface over the same shared target selector used by CLI status.
- Keep runtime target status distinct from latest connectivity exposure: a user can ask whether a machine is available without asking whether a tunnel/exposure is active.
- Return a spoken missing-target row for unknown targets instead of mutating state or requesting permission.

### RR18 - Runtime Target Evidence Export

Status: completed

Acceptance:

- Add a provider-free command that exports a Capo-owned evidence artifact for one runtime target's latest placement/status metadata.
- Record the exported markdown as project-level evidence so dashboards and later dogfood/readiness checks can cite runtime target state.
- Use guarded overwrite behavior so Capo does not overwrite user-authored files.
- Return a clear error for an unknown runtime target.
- Do not launch runtimes, launch providers, inspect credentials, open tunnels, request approvals, activate grants, retain raw transcripts, or mutate runtime target state.

Evidence:

- CLI `capo runtime target evidence --target TARGET_ID --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Exported artifacts use the `<!-- capo:runtime-target-evidence -->` marker and guarded overwrite behavior.
- Artifact records use `kind=runtime_target_evidence`; evidence projections use `kind=runtime_target_evidence`.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime target evidence as an operator review artifact over placement/status metadata, not proof that a runtime process is live.
- Keep runtime target evidence separate from connectivity exposure evidence: target status answers whether Capo can select a placement, while exposure evidence answers whether a channel was opened/approved/revoked.
- Keep the export read-model-derived and provider-free. It records target metadata only, without launching runtimes, launching provider CLIs, opening tunnels, inspecting credentials, materializing prompts, requesting approvals, activating grants, retaining raw transcripts, or mutating target state.
