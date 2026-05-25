# Capo Runtime And Tunnel Plan

## Objective

Define Capo's prototype execution and connectivity architecture: local process runtime, remote runtime abstraction, tunnel/connectivity boundary, lifecycle events, restart behavior, safety claims, and deferred runtime options.

This is the A4 architecture artifact. It refines the `RuntimeRunner` and `ConnectivityTunnel` boundaries from `boundaries.md` into an implementation-facing plan.

## Design Rules

- Runtime execution and network reachability are separate boundaries.
- `RuntimeRunner` owns process/container/devbox lifecycle. It starts, stops, interrupts, health-checks, and streams output from agent processes.
- `ConnectivityTunnel` owns endpoint resolution, private/public reachability, tunnel health, and exposure policy. It does not own task/session state or process handles.
- Capo never claims hard sandboxing unless the selected runtime actually enforces it through OS/container/VM mechanisms and tests prove it.
- The prototype uses `LocalProcessRunner` plus `LocalLoopbackTunnel`.
- Remote execution is modeled now but deferred until local spawn, stop, output capture, restart recovery, and inspection are proven.
- Every runtime side effect must be reflected as an event before UI/read-model state depends on it.
- Subscription-backed CLIs run as privileged local processes with scrubbed environment and redacted logs; Capo does not read provider session credentials.

## Static Dispatch Shape

Prototype enum:

```text
enum RuntimeRunner {
  LocalProcess(LocalProcessRunner),
  RemoteProcess(RemoteProcessRunner),
  Container(ContainerRunner),
  Fake(FakeRuntimeRunner),
}
```

Only `LocalProcessRunner` and `FakeRuntimeRunner` are required for the first prototype. `RemoteProcessRunner` and `ContainerRunner` are named now so state, configuration, and tests do not assume everything is local.

Connectivity enum:

```text
enum ConnectivityTunnel {
  LocalLoopback(LocalLoopbackTunnel),
  Ssh(SshTunnel),
  Tailscale(TailscaleTunnel),
  Reverse(ReverseTunnel),
  Fake(FakeTunnel),
}
```

Only `LocalLoopbackTunnel` and `FakeTunnel` are required for the first prototype. Tailscale and SSH are reachability options, not Capo execution engines.

## Core Records

### RuntimeTarget

Configured execution placement.

Fields:

- `runtime_target_id`
- `project_id?`
- `name`
- `runner_kind`: `local_process`, `remote_process`, `container`
- `workspace_root`
- `artifact_root`
- `default_cwd`
- `env_policy_json`
- `capability_profile_id`
- `connectivity_endpoint_id?`
- `status`: `available`, `disabled`, `unhealthy`
- `created_at`
- `updated_at`

`env_policy_json` includes allowed variable names, inherited variable policy, explicit variable overrides by secret handle, and redaction patterns for runtime logs. It stores policy and secret references, never raw secret values.

### RuntimeRequest

Controller-to-runtime request to start or control a process.

Fields:

- `runtime_request_id`
- `runtime_target_id`
- `session_id`
- `run_id`
- `adapter_id`
- `program`
- `argv`
- `launch_mode`: `direct_exec`, `shell`
- `cwd`
- `workspace_roots`
- `env_allowlist`
- `env_redaction_rules`
- `stdio_mode`: `pipe`, `pty`
- `timeout_ms?`
- `idle_timeout_ms?`
- `output_limit_bytes`
- `capability_profile_id`
- `permission_decision_id?`
- `idempotency_key`

`launch_mode = shell` requires an explicit shell capability decision because shell code can bypass pre-launch path checks.

### RuntimeProcessRef

Opaque reference returned by a runtime.

Fields:

- `runtime_process_ref_id`
- `runtime_target_id`
- `run_id`
- `external_pid?`
- `process_group_ref?`
- `remote_process_ref?`
- `started_at`
- `last_heartbeat_at?`
- `status`
- `redaction_state`

Capo stores this as a reference, not as proof the process still exists. Runtime health must be checked after restart.

### ConnectivityEndpoint

Configured way to reach a runtime or Capo server/client surface.

Fields:

- `connectivity_endpoint_id`
- `project_id?`
- `name`
- `tunnel_kind`: `local_loopback`, `ssh`, `tailscale`, `reverse`
- `address_ref`
- `identity_ref?`
- `auth_ref?`
- `exposure`: `loopback`, `private`, `public`
- `allowed_channels`
- `status`: `available`, `degraded`, `unreachable`, `disabled`
- `created_at`
- `updated_at`

`auth_ref` points to a secret handle or OS/vendor credential location, never raw credential material.

### ResolvedEndpoint

Resolved endpoint/channel for a runtime, Capo server, input surface, or artifact operation.

Fields:

- `resolved_endpoint_id`
- `connectivity_endpoint_id`
- `owner_kind`: `runtime_target`, `capo_server`, `input_surface`, `artifact_store`
- `owner_id`
- `channel_kind`: `control`, `stdio`, `logs`, `dashboard`, `artifact`
- `resolved_uri`
- `identity_fingerprint?`
- `expires_at?`
- `redaction_state`
- `created_at`

`EndpointOwner` is the typed pair `owner_kind` / `owner_id`. This keeps local dashboard/API/input-surface endpoints from being forced into runtime target records.

## Local Process Runner

### Contract

```text
prepare(RuntimeTarget) -> RuntimePrepared
start_process(RuntimeRequest) -> RuntimeStartResult
write_stdin(RuntimeProcessRef, Bytes) -> WriteResult
interrupt(RuntimeProcessRef, InterruptKind) -> InterruptResult
terminate(RuntimeProcessRef, TerminationKind) -> TerminateResult
kill(RuntimeProcessRef) -> KillResult
stream_output(RuntimeProcessRef) -> RuntimeOutputStream
health(RuntimeProcessRef) -> RuntimeHealth
cleanup(RuntimeProcessRef, CleanupPolicy) -> CleanupResult
```

`RuntimeStartResult` is either `started(RuntimeProcessRef)` or `failed(RuntimeStartFailure)` with retryability and cleanup detail.

### Required Prototype Behavior

- Spawn with explicit program, argv, launch mode, cwd, workspace root, and environment allowlist.
- Reject launch requests whose cwd is outside configured workspace roots unless a reviewed profile permits it.
- Scrub environment variables not on the allowlist.
- Capture stdout/stderr as bounded artifacts and stream normalized output events.
- Support pipe mode first; add PTY mode only when a target adapter needs terminal semantics.
- Track process group/session when the platform supports it.
- Implement kill escalation: interrupt, terminate, kill.
- Emit health/liveness data without relying on UI polling.
- Preserve output redaction metadata before storing artifacts.

### Non-Goals

- Hard filesystem or network sandboxing.
- Browser cookie/session access.
- Secret material reads.
- Persistent shell sessions outside a recorded `Run`.

### Failure Modes

- Child process survives direct parent exit.
- Grandchild keeps stdout/stderr open after timeout.
- CLI requires PTY behavior but runtime launched with pipes.
- Output contains secrets before redaction.
- Process is alive after Capo restart but adapter cannot reattach.
- Workspace path check is bypassed by shell code after launch.

### Start Sequence

The controller/runtime start path is append-first where possible:

1. Controller validates the `RuntimeRequest`, permission decision, workspace roots, and `env_policy_json`.
2. Controller appends `runtime.start_requested` with the request idempotency key and status `pending`.
3. Runtime attempts `start_process(...)`.
4. On success, controller appends `runtime.process_started` and then projects `run.started` / `run.health_changed`.
5. On launch failure, controller appends `runtime.process_start_failed` and projects the run as failed or waiting for input depending on retry policy.
6. If the process starts but event append fails, the controller must immediately attempt runtime cleanup. On next recovery, any matching live process without `runtime.process_started` becomes `run.orphaned` with cleanup evidence.
7. Repeated start requests with the same idempotency key must not spawn a second process after `runtime.process_started` exists.

### Recovery Behavior

On restart, Capo recovers the same run in place when the original process is still alive:

1. Loads live-looking `Run` records.
2. Asks `RuntimeRunner.health(...)` for each stored `RuntimeProcessRef`.
3. If the process is alive and adapter can attach, emits `run.recovered` for the existing run.
4. If the process is alive but adapter cannot attach, emits `run.orphaned` and keeps logs inspectable.
5. If the process is gone and no terminal event exists, emits `run.exited` with unknown exit detail.
6. Rebuilds read models from events before accepting new input.

`recovery_of_run_id` is reserved for a new run that relaunches or retries after restart. It is not used for simple attach/recovery of the same live process.

## Remote Runtime Abstraction

`RemoteProcessRunner` has the same controller-facing contract as `LocalProcessRunner`. Its implementation can later use SSH, a Capo worker daemon, devcontainer tooling, or a cloud workspace API, but those are hidden behind the runner.

Remote runtime responsibilities:

- Prove runtime target identity before launch.
- Resolve endpoint through `ConnectivityTunnel`.
- Start/stop/health-check remote processes.
- Stream output into Capo artifacts with the same redaction and output-limit rules as local runtime.
- Report whether process reattach is supported after Capo restart.
- Keep remote workspace identity and artifact paths explicit.

Deferred remote variants:

- `SshRemoteProcessRunner`: command execution through SSH transport.
- `CapoWorkerRunner`: a future lightweight Capo worker daemon on another machine.
- `DevcontainerRunner`: workspace execution inside a local or remote devcontainer.
- `CloudDevboxRunner`: provider-specific devbox/workspace execution.

Remote runtime is not in the first prototype because restart recovery and process observability must be proven locally first.

## Connectivity / Tunnel Boundary

### Contract

```text
resolve_endpoint(ConnectivityEndpoint, EndpointOwner, ChannelKind) -> ResolvedEndpoint
check_reachability(ConnectivityEndpoint) -> ConnectivityHealth
open_channel(ResolvedEndpoint) -> ChannelRef
close_channel(ChannelRef) -> CloseResult
exposure_report(ConnectivityEndpoint) -> ExposureReport
```

### LocalLoopbackTunnel

Prototype default.

- Resolves local dashboard/API/runtime endpoints to loopback only.
- Does not expose public listeners.
- Can be used by local CLI, local dashboard, and fake runtime tests.

### SshTunnel

Deferred.

- Provides authenticated reachability to a remote machine or port.
- May later support command transport for `SshRemoteProcessRunner`, but command execution still belongs to the runtime runner.
- Requires host identity checks, key reference storage, failure/reconnect events, and redacted logs.

### TailscaleTunnel

Deferred private connectivity path.

- Provides tailnet identity and private endpoint resolution.
- Tailscale SSH may be a transport option, but Capo still treats it as connectivity/auth plumbing.
- Tailnet ACLs become part of deployment security posture and must be reviewed before remote dogfood.
- Tailscale Funnel/public exposure is out of prototype scope and must require explicit permission, short-lived exposure, and audit events.

### ReverseTunnel

Deferred.

- For public webhooks, demo dashboards, or mobile access when private connectivity is unavailable.
- Public exposure is high risk and disabled by default.

## Event Model Additions

Add events:

- `runtime.target_registered`
- `runtime.target_updated`
- `runtime.start_requested`
- `runtime.prepared`
- `runtime.process_started`
- `runtime.process_start_failed`
- `runtime.output_delta`
- `runtime.output_artifact_recorded`
- `runtime.stdin_written`
- `runtime.interrupt_sent`
- `runtime.terminate_sent`
- `runtime.kill_sent`
- `runtime.process_exited`
- `runtime.health_changed`
- `runtime.cleanup_completed`
- `connectivity.endpoint_registered`
- `connectivity.endpoint_updated`
- `connectivity.endpoint_resolved`
- `connectivity.health_changed`
- `connectivity.channel_opened`
- `connectivity.channel_closed`
- `connectivity.exposure_changed`

`run.*` events remain user/session lifecycle facts. `runtime.*` events are lower-level execution facts projected into run/session state.

## State Model Additions

Add tables:

```text
runtime_targets(runtime_target_id, project_id, name, runner_kind, workspace_root, artifact_root, default_cwd, env_policy_json, capability_profile_id, connectivity_endpoint_id, status, created_at, updated_at)
runtime_process_refs(runtime_process_ref_id, runtime_target_id, run_id, external_pid, process_group_ref_json, remote_process_ref_json, started_at, last_heartbeat_at, status, redaction_state)
connectivity_endpoints(connectivity_endpoint_id, project_id, name, tunnel_kind, address_ref_json, identity_ref_json, auth_ref, exposure, allowed_channels_json, status, created_at, updated_at)
resolved_endpoints(resolved_endpoint_id, connectivity_endpoint_id, owner_kind, owner_id, channel_kind, resolved_uri, identity_fingerprint, expires_at, redaction_state, created_at)
```

Add indexes:

- `runtime_targets(project_id, status)`
- `runtime_process_refs(run_id)`
- `runtime_process_refs(runtime_target_id, status)`
- `connectivity_endpoints(project_id, tunnel_kind, status)`
- `resolved_endpoints(owner_kind, owner_id, channel_kind)`

## Prototype Choice

Build first:

- `FakeRuntimeRunner` for deterministic controller tests.
- `LocalProcessRunner` with pipe stdio, environment allowlist, output caps, process refs, health, interrupt/terminate/kill, and cleanup.
- `FakeTunnel` for test-controlled reachability.
- `LocalLoopbackTunnel` for local CLI/dashboard/API.

Defer:

- PTY until Claude Code/Codex adapter testing proves it is needed.
- Container/devcontainer runtime until local recovery is stable.
- SSH remote runner until local runtime contract is implemented and tested.
- Tailscale private remote-control path until a dashboard/server surface exists.
- Public reverse tunnel/Funnel until explicit exposure policy and UX exist.
- Linux sandbox profiles until Capo can truthfully distinguish audited local process control from enforced sandboxing.

## Test Strategy

Prototype tests should prove:

1. A fake runtime can start, stream output, exit, and project a session/run read model.
2. A local process launch records program/argv metadata, launch mode, cwd, environment allowlist, runtime process ref, and output artifacts.
3. Output caps and redaction state are preserved before data is stored.
4. Interrupt, terminate, and kill produce distinct events and final run state.
5. Restart recovery maps alive/missing/unattachable processes to recovered/orphaned/exited states without duplicate runs.
6. Local loopback endpoints resolve only loopback/private-local addresses.
7. Public exposure requests require permission and are unavailable in the prototype profile.
8. Remote runtime/tunnel failures are representable with fake runner/tunnel fixtures without depending on SSH or Tailscale.

## Recommendation

Implement the prototype runtime in Rust with `tokio::process` or equivalent async process supervision, SQLite-backed runtime records, file artifacts for logs/output, and no hard sandbox claims. Keep remote runners and tunnels in the type model but out of the first e2e path.

Confidence: high for the local-first runtime and tunnel separation. Confidence is medium for exact PTY needs until Claude Code and Codex adapter tests exercise their real noninteractive and interactive modes.
