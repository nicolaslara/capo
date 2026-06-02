# Remote Runtime References

## Objective

Record the local and external sources that shape the `remote-runtime` workpad: a
real remote `RuntimeRunner` (`RemoteProcessRunner`/`SshRemoteProcessRunner`),
git-based remote workspace materialization, remote streaming, remote
sandbox/worktree composition, and crash-safe remote recovery. Dated claims reflect
2026-06-02.

## Local Architecture Sources

- `workpads/architecture/runtime-tunnel.md`
  - THE design this workpad implements. Key facts: runner vs. tunnel are separate
    boundaries (runner owns process lifecycle, tunnel owns reachability);
    `RemoteProcessRunner` has the SAME controller-facing contract as
    `LocalProcessRunner` and its transport (SSH / worker daemon / devcontainer /
    cloud) is hidden behind the runner; remote runtime responsibilities = prove
    target identity before launch, resolve endpoint through `ConnectivityTunnel`,
    start/stop/health, stream output with the same redaction + output-limit rules,
    report whether reattach is supported, keep remote workspace identity + artifact
    paths explicit; deferred variants are `SshRemoteProcessRunner` /
    `CapoWorkerRunner` / `DevcontainerRunner` / `CloudDevboxRunner`; the Start
    Sequence is append-first (`runtime.start_requested` before spawn, orphan reaping
    on recovery) and the Recovery Behavior maps alive/unattachable/gone to
    recovered/orphaned/exited; `RuntimeProcessRef` carries `remote_process_ref` +
    `last_heartbeat_at`; the `runtime.*` event family and the
    `runtime_process_refs(remote_process_ref_json, last_heartbeat_at)` table already
    exist in the state-model additions; Capo never claims hard sandboxing unless the
    runtime enforces it through OS/container/VM mechanisms and tests prove it.
- `workpads/architecture/boundaries.md`
  - Key facts: `RuntimeRunner = LocalProcessRunner | RemoteProcessRunner |
    ContainerRunner | FakeRuntimeRunner`; static-dispatch enum; runner contract is
    `prepare`/`start_process`/`write_stdin`/`interrupt`/`terminate`/`kill`/
    `stream_output`/`health`/`cleanup`; runner non-responsibilities include claiming
    sandboxing it does not enforce and tunnel connection mechanics; controller
    recovers from restart by reconciling persisted state with runtime status; the
    cross-cutting event envelope (`external_ref` for process IDs, `idempotency_key`,
    `redaction_state`).

## Local Implementation Sources

- `crates/capo-runtime/src/lib.rs`
  - Key facts: `RuntimeRunner` enum is `Fake | LocalProcess | RemoteProcess` (`:63`);
    the existing `RemoteProcessRunner` (`:1595-1734`) is a LOOPBACK-DECORATING STUB
    (wraps a `LocalProcessRunner` built from `config.local_loopback`, runs locally,
    rewrites the ref to `remote-process:{target}:{endpoint}:{local_ref}`, prepends
    synthetic `runtime.remote_*` events, `health` from a local status string at
    `:1655`), and `RuntimeRunner` control dispatch for `RemoteProcess` falls through
    to `FakeRuntimeRunner` (`:94-118`). `LocalProcessRunner` is the real contract to
    mirror: `start_process` (`:568`), `spawn_piped_process` + `PipedRunningProcess`
    (`:740`) for the bidirectional line protocol, `interrupt`/`terminate`/`kill`
    (`:801-859`), `health`/`health_running` (`:981`), `cleanup` (`:1001`),
    `recover_orphan`/`probe_run_health` (`:1013-1162`) with `external_pid` + `boot_id`
    liveness, `RedactionPolicy` credential-shape scan (`:234-291`), `capped_output`
    output cap, and `LocalRuntimeProcessRef` (`run_id`/`runtime_process_ref`/
    `external_pid`/`boot_id`/`status`/`redaction_state`).
- `crates/capo-runtime/src/worktree.rs`
  - Key facts: `depth` DP8 worktree primitive — `WorktreeManager::create/reconcile/
    teardown`, `WorktreeRequest` (repo_root / worktrees_root / key), `WorktreeOutcome`,
    `WorktreeError` (typed, never silent fall-through), `worktree.created/reconciled/
    torn_down` events, `WORKTREE_ISOLATION_VARIANT`. RR3 reuses this executed on the
    remote; uses `git worktree add` sharing the origin object store and a reconcile
    merge-back point.
- `crates/capo-runtime/src/sandbox.rs`
  - Key facts: `depth` DP7 OS sandbox tier — `OsSandbox`, `SandboxTier::{None,
    MacosSeatbelt, LinuxLandlockBwrap}`, `SandboxProfile` (writable_roots /
    allow_network_egress), `SandboxEnforcement::{Enforced, Unenforced, Refused}`,
    `is_enforced_here()`, `sandbox.launch_refused` / `sandbox.unenforced` events, scopes
    wired to `safety-gates`. RR5 reuses this with enforcement claimed per the REMOTE OS.
- `crates/capo-runtime/src/async_runner.rs`
  - Key facts: `AsyncLocalProcessRunner` / `AsyncRunningProcess` / `StreamSource` /
    `StreamingOutcome` — the async streaming surface RR4 reuses for remote output
    deltas over the channel rather than authoring a parallel remote stream type.
- `crates/capo-cli/src/connectivity.rs`
  - Key facts: the connectivity CLI surface (`expose_connectivity_stub`,
    request/activate/revoke/status) over `ConnectivityTunnel`/`ConnectivityEndpointConfig`/
    `ExposureScope`/`ChannelKind`/`EndpointOwner` and the exposure approval/grant
    lifecycle (`PermissionApprovalProjection` + `CapabilityGrant`, scope
    `remote-control-reviewed`). This is the auditable + revocable exposure pattern the
    remote runner's channel + remote-control grant compose with; the CHANNEL transport
    itself is built by `connectivity-tunnel`.
- `crates/capo-state/src/event.rs`
  - Key facts: existing `EventKind`s include `ConnectivityExposure{Requested,Changed,
    Revoked}`, `ConnectivityHealthChanged`, `RuntimeTarget{Registered,StatusChanged}`
    (`:22-27`, `:161-166`); the `runtime.*` / `connectivity.*` `as_str` wire mapping
    and codec round-trip. RR1 promotes the stub's bare `runtime.remote_*`
    `RuntimeEvent`s into real round-trippable `EventKind`s alongside this family.
- `crates/capo-server/src/live_provider.rs` (+ `util.rs`)
  - Key facts: live execution gated by `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` +
    `CAPO_SERVER_RUN_CODEX_LIVE` with `mock_provider_output_jsonl` for deterministic
    tests; the opt-in gate + preflight pattern RR8's
    `CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE` + `CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT`
    mirror, and the `scan_artifacts_for_sensitive_markers` secrets-stripping discipline
    for live smoke evidence.

## Cross-Workpad Sources

- `workpads/depth/tasks.md` (DP7 sandbox, DP8 worktree, DP11 live-smoke pattern) —
  the sandbox + worktree primitives RR5 lifts onto the remote and the
  deterministic-paired-with-gated-live-smoke pattern RR8 follows.
- `workpads/streaming-transport/tasks.md` — the `runtime.output_delta` / stdin /
  `Subscribe{from_sequence}` event model RR4 reuses for remote streaming.
- `workpads/safety-gates/tasks.md` — checkpoint/rollback, single-writer lease, grant
  read-back/revoke, and `PermissionPolicy` enforcement RR2/RR5/RR6 compose with.
- `workpads/real-turn-loop/tasks.md` — the controller turn loop, `AgentAdapter`
  contract, workspace confinement, and append-first start-sequence event contract the
  remote runner plugs beneath.
- `connectivity-tunnel` workpad (the channel; dependency, not yet authored) — provides
  endpoint resolution, the SSH/Tailscale/reverse transport, reachability health,
  exposure policy, channel fingerprint, and `auth_ref` handling that the remote runner
  consumes. The remote runner builds NOTHING of the channel itself.

## External Sources

- OpenSSH (`ssh`, `ssh-keygen`, `known_hosts`) — the first real transport for
  `SshRemoteProcessRunner` (command execution + git transport), to be observed/cited
  during RR8; host identity verification is the channel's responsibility
  (`connectivity-tunnel`), consumed by the runner as a fingerprint.
- Git worktree + push/fetch over SSH (`git worktree add`, `git push`, `git fetch`) —
  the content-addressed materialization + map-back mechanism for RR3; to be cited
  against the installed git version during implementation.
- OpenAI codex `linux-sandbox` / `bwrap` / `sandboxing` crates
  (`workpads/references/repos/openai-codex/codex-rs/{sandboxing,linux-sandbox,bwrap}`,
  observed 2026-05-29 in `depth/references.md`) — the seatbelt/landlock+bwrap model
  RR5 applies on the remote host.
- OpenHands docker runtime (observed 2026-05-29 in `depth/references.md`) — peer
  baseline for remote execution + git-based workspace isolation; this workpad reaches
  comparable isolation with OS sandbox + git worktree over SSH without requiring a full
  container runtime for the first remote tier.
