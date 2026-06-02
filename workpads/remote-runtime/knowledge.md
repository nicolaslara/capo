# Remote Runtime Knowledge

## Objective

Capture decisions for the `remote-runtime` workpad: a real remote `RuntimeRunner`
(`RemoteProcessRunner` / `SshRemoteProcessRunner`) that executes an agent on a
DIFFERENT machine than the controller + event-sourced state, behind the SAME
`RuntimeRunner` contract as `LocalProcessRunner`. The runner owns execution; the
`ConnectivityTunnel` (built by `connectivity-tunnel`) only provides reachability.

## Scope Decision

`remote-runtime` is a runtime-substrate workpad that DEPENDS ON
`connectivity-tunnel` for the channel and reuses the existing local runtime
substrate. It does not unblock the loop; it broadens WHERE a run can execute:

```text
real-turn-loop -> streaming-transport / tools-aci -> safety-gates
              -> connectivity-tunnel (channel) -> remote-runtime (execution)
```

By the time `remote-runtime` runs, the loop is real (`real-turn-loop`), streaming
is real (`streaming-transport`), the loop is gated and recoverable
(`safety-gates`), the sandbox + worktree primitives exist (`depth` DP7/DP8), and a
resolved channel is available (`connectivity-tunnel`). `remote-runtime` takes the
harness from "executes only on the controller's host" to "executes on a remote
host behind the same controller-facing contract, with git-based workspace
materialization and honest remote enforcement claims." It re-architects nothing
earlier workpads own: it does not touch the turn loop, the transport protocol, the
permission engine, the sandbox tier, the worktree primitive, or the channel.

## Cardinal Rule: Runner Owns Execution, Tunnel Owns Reachability

This is the load-bearing separation from `runtime-tunnel.md` Design Rules:
`RuntimeRunner` owns process/worktree/sandbox lifecycle, start/stop/health, and
output streaming; `ConnectivityTunnel` owns endpoint resolution, private/public
reachability, tunnel health, exposure policy, and `auth_ref`. The remote runner
CONSUMES a resolved channel; it never opens sockets, resolves endpoints, or
touches credential material. This keeps connectivity concerns separate from agent
execution and controller state (an explicit AGENTS.md safety-boundary rule), and
means `remote-runtime` DEPENDS ON `connectivity-tunnel` for the channel while
remaining the sole owner of what runs over it.

## Why The Existing RemoteProcessRunner Is A Stub, Not A Remote Runner

`crates/capo-runtime/src/lib.rs:1595-1734` ships `RemoteProcessRunner` today, but
it is a LOOPBACK DECORATOR: it constructs a `LocalProcessRunner` from
`config.local_loopback`, runs `start_process` locally, rewrites the
`runtime_process_ref` to `remote-process:{target}:{endpoint}:{local_ref}`, and
prepends two synthetic `runtime.remote_target_resolved` /
`runtime.remote_process_started` events. Its `health` is computed from a local
status string (`lib.rs:1655`), and `RuntimeRunner`'s control dispatch for
`RemoteProcess` falls through to `FakeRuntimeRunner` (`lib.rs:94-118`). It never
crosses a machine boundary, opens a channel, materializes a remote workspace, or
probes a remote process. This workpad replaces that stub — including removing the
`config.local_loopback` construction and re-pointing the `RemoteProcess` control
dispatch — with a runner whose lifecycle, materialization, streaming, and health
are real and channel-backed, while keeping the same controller-facing contract so
the controller does not learn that a run is remote.

## Injected Decision: Git-Based Workspace Sync (push/fetch + worktree the commit)

Remote workspace materialization is GIT-BASED, not a file-rsync/tar copy:

- Before a remote launch, the target commit is pushed/fetched to the remote and
  `git worktree add`-ed into a dedicated remote worktree root; the run's cwd /
  confinement is scoped to that worktree root.
- Results are mapped back by git: the remote-produced commit/branch is fetched back
  to Capo's host as a named ref (the same reconcile/merge-back point the `depth` DP8
  worktree primitive already models), recorded as an event.

Rationale: git materialization is content-addressed (the run is pinned to a SHA),
auditable (the source SHA, remote HEAD, and fetched-back ref are recorded events),
and it reuses Capo's existing worktree isolation + checkpoint/rollback machinery
rather than inventing a second sync path. The materialized commit IS the run's
pre-write checkpoint, so rollback is a git ref operation.

Explicit consequence (documented, not hidden): uncommitted / untracked scratch is
NOT auto-synced. A remote run sees only the materialized commit plus whatever the
agent itself produces on the remote. This is recorded as a fact on the
materialization event and proven by a test (a dirty local file is absent on the
remote worktree), so an operator is never surprised that local uncommitted edits
did not travel. Auto-syncing dirty scratch is an explicit non-goal for the first
remote runner.

## Reuse, Don't Re-Author, The Substrate

- The `RuntimeProcessRef` is the SAME opaque reference for local and remote; only
  its `remote_process_ref` field is populated for remote runs (the field already
  exists in `runtime-tunnel.md`'s record and the `RuntimeProcessRef` shape, with
  `last_heartbeat_at`). Remote identity (remote pid + remote boot/host id) lives
  there.
- Streaming reuses the `streaming-transport` `runtime.output_delta` / stdin event
  model and the existing piped line-protocol surface
  (`LocalProcessRunner::spawn_piped_process` / `PipedRunningProcess`, and the async
  `AsyncLocalProcessRunner` / `StreamSource`), not a parallel remote stream type,
  with the `from_sequence` offset discipline for reconnect.
- Redaction reuses `RedactionPolicy` (credential-shape scan, `lib.rs:234-291`) and
  `output_limit_bytes` at the remote boundary, so remote output is scrubbed/bounded
  exactly like local, with `redaction_state` stamped before persistence.
- The remote sandbox + worktree reuse `depth` DP7 (`OsSandbox`/`SandboxTier`) and DP8
  (`WorktreeManager`/worktree), executed on the remote host.
- Recovery mirrors `LocalProcessRunner::recover_orphan` / `probe_run_health`
  semantics (`lib.rs:1013-1162`), with the liveness signal coming from a remote
  probe over the channel.

## Honest Enforcement Claims On The Remote

DP7's `is_enforced_here()` rule generalizes: a remote sandbox tier is `Enforced`
only when the REMOTE OS supports it (probed over the channel), and `Unenforced`
otherwise, recorded as `sandbox.unenforced` for that remote run. Capo never claims
sandboxing on a remote OS it cannot enforce. The sandbox decision is wired to the
`safety-gates` capability scopes: an un-granted critical scope is refused BEFORE the
remote sandbox launches, as a `sandbox.launch_refused` event, never a silent
failure.

## Safety Boundary Is First-Class

A remote runner is a remote-control capability and is treated as such:

- It MUST be auditable + revocable: every remote lifecycle step (resolve, start,
  output, stdin, interrupt/terminate/kill, materialize, reconcile, cleanup) is an
  event; a revoked remote-control grant stops the run and the runner cannot
  re-establish execution without a fresh grant. This composes with the
  `remote-control-reviewed` scope + grant lifecycle the connectivity CLI surface
  already models.
- The agent's subscription credential (Codex/Claude) is a PRIVILEGED CONNECTOR carried
  by HANDLE (`auth_ref`), never read, stored raw, or logged on the remote; channel auth
  belongs to `connectivity-tunnel` and is also handle-only.
- No API keys, OAuth/subscription tokens, cookies, session files, git-transport URLs
  with embedded secrets, or transcripts-with-secrets are ever written to a
  remote-runtime artifact or event; the credential scan runs before persistence.

## Verification Discipline

Deterministic-fake-before-live holds for every task: a `FakeRemoteProcessRunner` +
fake-channel test lands before any real-network path. Every manual/live smoke is
paired with a deterministic assertion (process-ref shape, exit status, event order,
materialized-HEAD-matches-SHA, or restart/replay). All live remote paths sit behind
explicit opt-in env gates (`CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE` +
`CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT`) mirroring the Codex/preflight gates, and
`#[ignore]` / skip cleanly when no host is configured. Nothing completes on operator
self-attestation.

## Non-Goals

- The channel itself (endpoint resolution, SSH/Tailscale/reverse transport,
  reachability health, exposure policy, `auth_ref`) — owned by `connectivity-tunnel`.
- Auto-syncing uncommitted / untracked scratch to the remote.
- Container / devcontainer / cloud-devbox runners and a Capo worker daemon (named in
  `runtime-tunnel.md`, deferred).
- Changing the turn loop, transport wire, permission engine, sandbox tier, worktree
  primitive, or goal model — those are owned by their workpads and reused here.
- Claiming sandboxing on a remote OS Capo cannot enforce.

## Open Questions

- Channel handle shape: does `connectivity-tunnel` hand the runner a duplex byte
  stream (one channel multiplexing control + stdio + git transport), or separate
  channels per `ChannelKind` (`control` / `stdio` / `artifact`)? The
  `ResolvedEndpoint`/`ChannelKind` surface suggests per-channel; leaning toward a
  control channel + a stdio channel + reusing the SSH transport for the git
  push/fetch, but recorded as open pending the `connectivity-tunnel` contract.
- Git transport for materialization: push from Capo's host to the remote worktree
  repo over the SSH channel, vs. have the remote fetch from a shared origin the
  operator already trusts? Push-over-channel avoids a third-party origin and keeps the
  run self-contained; shared-origin fetch is simpler when one exists. Likely support
  both, default to push-over-channel.
- Remote agent execution: does the remote run a provider CLI (Codex/Claude) directly,
  or a thin Capo remote shim? The first runner runs the provider CLI directly under
  the remote sandbox/worktree; a Capo worker daemon (`CapoWorkerRunner`) is a later
  variant. Open whether reattach needs a remote-side recorded-PID file or a daemon.
- Reattach fidelity: a bare SSH-launched process is hard to reattach to after a
  controller restart without a remote PID/boot record; what minimum remote-side state
  (a recorded remote pid + boot id file under the worktree) is acceptable before a
  worker daemon is justified?
- Merge-back review point: when a remote run produces a commit, what is the
  review/reconcile gate before it lands on a tracked branch (ties into `depth` DP8 and
  `goal-autonomy`)?
