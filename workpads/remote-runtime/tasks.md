# Remote Runtime Tasks

## Objective

Implement a real remote `RuntimeRunner` so an agent can EXECUTE on a different
machine than the one where Capo's controller + event-sourced state live, behind
the SAME `RuntimeRunner` contract as `LocalProcessRunner`
(`prepare`/`start_process`/`write_stdin`/`interrupt`/`terminate`/`kill`/
`stream_output`/`health`/`cleanup`). This realizes the `RemoteProcessRunner` /
`SshRemoteProcessRunner` named in `runtime-tunnel.md`'s Remote Runtime
Abstraction. The runner OWNS execution; the `ConnectivityTunnel` only provides
reachability (the channel). Workspace materialization on the remote is
GIT-BASED: push/fetch + `git worktree` the target commit onto the remote, fitting
Capo's existing worktree isolation and checkpoint/rollback model; uncommitted /
untracked scratch is explicitly NOT auto-synced.

Today `crates/capo-runtime/src/lib.rs` ships a `RemoteProcessRunner` that is a
LOOPBACK-DECORATING STUB: it wraps a `LocalProcessRunner` built from
`config.local_loopback`, runs the command locally, and only rewrites the
`runtime_process_ref` string to a `remote-process:{target}:{endpoint}:{local_ref}`
shape with two prepended `runtime.remote_*` events (`lib.rs:1595-1734`), while
`RuntimeRunner`'s control dispatch for `RemoteProcess` falls through to
`FakeRuntimeRunner` (`lib.rs:94-118`). It never crosses a machine boundary, never
opens a channel, never materializes a remote workspace, and its `health` is
computed from a local status string (`lib.rs:1655`). This workpad replaces that
stub with a real remote runner whose channel is provided by the
`connectivity-tunnel` workpad.

## Status

In progress. RR0 complete. RR1 complete. RR2 complete (deterministic
fake-channel recovery suite). RR3 event-kinds landed. RR4 complete
(deterministic fake-channel output-delta + stdin streaming suite). RR5 complete
(remote OS sandbox + worktree composition with honest remote-OS-probed
enforcement claims; deterministic fake-channel suite). RR6-RR8 pending.

## Feature Set

- A `RemoteRunner` execution contract that the controller drives identically to
  `LocalProcessRunner`, with a real `SshRemoteProcessRunner` and a deterministic
  `FakeRemoteProcessRunner` behind it.
- Remote process lifecycle over a channel: start / stop / health / reattach,
  realizing `runtime-tunnel.md`'s append-first Start Sequence + Recovery Behavior
  across a machine boundary.
- Git-based remote workspace materialization (push/fetch + worktree a commit on
  the remote, results mapped back by git).
- Remote output-delta + stdin streaming over the channel, reusing the
  `streaming-transport` event model and `RuntimeProcessRef`.
- Composition of the OS sandbox tier + worktree isolation ON the remote, with
  honest enforcement claims evaluated against the remote OS.
- Crash-safe remote runs + recovery/reattach events.
- A deterministic fake-remote suite that proves every invariant before any live
  path, plus one opt-in live SSH smoke paired with its deterministic fixture.

## Boundaries This Workpad Owns

- The `RemoteRunner` trait/contract surface and the real `SshRemoteProcessRunner`
  plus the deterministic `FakeRemoteProcessRunner` behind it.
- Remote process lifecycle over a channel: start / stop / health / reattach.
- Git-based remote workspace materialization and git-based map-back.
- Remote output-delta + stdin streaming over the channel.
- Composition of OS sandbox + worktree isolation ON the remote, with honest
  enforcement claims.
- Crash-safe remote runs + recovery/reattach events.

## Boundaries This Workpad Defers

- The CHANNEL itself (endpoint resolution, SSH/Tailscale/reverse transport,
  reachability health, exposure policy, `auth_ref` handling) belongs to
  `connectivity-tunnel`. This workpad consumes a resolved channel; it does not
  build one.
- The turn loop / `AgentAdapter` contract (`real-turn-loop`).
- The SSE/HTTP streaming wire (`streaming-transport`) — reused, not redefined.
- `PermissionPolicy` / grant lifecycle / `VerificationRunner` / checkpoint
  mechanics (`safety-gates`) — composed with, not reimplemented.
- The OS sandbox tier and the local worktree primitive (`depth` DP7/DP8) — reused
  and lifted onto the remote, not re-authored.
- The goal model (`goal-autonomy`).
- Container / devcontainer / cloud-devbox runners and a Capo worker daemon
  (`runtime-tunnel.md` deferred variants) — named, not built.

## RR0 - Workpad, Routing, Scope, Per-Task Prerequisite + Verification Invariant

Status: complete.

Scope: Establish the workpad, its place in the sequence, the runner/channel
separation, the injected git-sync decision, and the verification invariant.

Acceptance criteria:

- Record that `remote-runtime` DEPENDS ON `connectivity-tunnel` (the channel) and
  builds on the existing local runtime substrate (`real-turn-loop` confinement +
  `LocalProcessRunner` + `depth` sandbox/worktree). The runner owns execution; the
  tunnel owns reachability — restated as the workpad's cardinal rule.
- Record the INJECTED workspace-sync decision: remote workspace materialization is
  GIT-BASED (push/fetch + worktree the target commit on the remote), and
  uncommitted / untracked scratch is NOT auto-synced. Record the rationale
  (content-addressed, auditable, reuses worktree isolation + checkpoint/rollback)
  and the explicit consequence (a run only sees committed state plus whatever the
  agent itself produces on the remote).
- List the boundaries this workpad owns vs. defers (as above), naming the existing
  stub `RemoteProcessRunner` (`lib.rs:1595`) and its `RemoteProcess` control
  fall-through to `FakeRuntimeRunner` (`lib.rs:94-118`) as the things being
  replaced, and the `connectivity-tunnel` workpad as the channel provider. (Line
  numbers observed 2026-06-02 against `crates/capo-runtime/src/lib.rs`; they are an
  anchor, not a contract — RR1 MUST re-locate the stub def, the `RemoteProcess`
  control fall-through, and the local-status `health` by symbol before accepting any
  drifted line number as authoritative.)
- Record the per-task prerequisites: RR1-RR2 (process lifecycle + reattach) and
  RR4 (stream/stdin) need a resolved channel from `connectivity-tunnel` plus the
  local runtime substrate; RR3 (git materialization) needs RR1 + the `depth` DP8
  worktree primitive; RR5 (sandbox+worktree on remote) needs RR3 + `depth` DP7/DP8
  + `safety-gates`; RR6 (crash-safe + recovery) needs RR1-RR4 + `safety-gates`
  checkpoint/recovery; RR7 (determinism consolidation) needs RR1-RR6; RR8 (live
  smoke) needs RR1-RR7 green and a real SSH host.
- Record the verification invariant: NO task completes on operator
  self-attestation; a deterministic `FakeRemoteProcessRunner` / fake-channel test
  lands BEFORE any real-network path for that task; every manual/live smoke is
  paired with a deterministic assertion (process-ref shape, exit status, event
  sequence, or restart/replay); all live remote paths stay behind explicit opt-in
  env gates mirroring `CAPO_SERVER_RUN_CODEX_LIVE` /
  `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` (i.e.
  `CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE` + `CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT`)
  and `#[ignore]` / skip cleanly when no host is configured.
- Record the SAFETY-BOUNDARY acceptance criterion as first-class: a remote runner
  is a remote-control capability and MUST be auditable + revocable; the agent
  credential (Codex/Claude subscription) is a PRIVILEGED CONNECTOR carried by
  HANDLE (`auth_ref`), never logged or stored raw on the remote; channel auth,
  remote stdout/stderr, and the git transport URL pass redaction before any
  artifact/event is persisted; no API keys, OAuth/subscription tokens, cookies,
  session files, or transcripts-with-secrets are ever written to a remote-runtime
  artifact or event.

Verification:

- `workpads/remote-runtime/tasks.md`, `knowledge.md`, `references.md` exist and
  encode the decisions above. No code in RR0.

Dependencies: none (planning). Cross-workpad: names `connectivity-tunnel`,
`real-turn-loop`, `depth`, `safety-gates` as downstream/upstream relationships.

## RR1 - Remote Process Lifecycle Over The Channel (start/stop/health) + Start Sequence

Status: complete.

Evidence: `crates/capo-runtime/src/lib.rs` `RemoteProcessRunner` (real
`start_process`/`interrupt`/`terminate`/`kill`/`health`/`cleanup` over an injected
`RemoteChannel`), `RuntimeRunner::{interrupt,terminate,kill,health}_local`
dispatching `RemoteProcess` to the real runner (not the `FakeRuntimeRunner`
fall-through). `RuntimeRunnerContract` trait gives a compile-time check that the
local + remote control surfaces share the SAME method shapes (aligned
`kill(reason)`, unified `CleanupOutcome`). Idempotency is ENFORCED by a launched-run
ledger keyed by run id (a duplicate start returns the recorded outcome and never
calls `transport.launch` again); the fake channel exposes a real `spawn_count()` and
mints a distinct pid per spawn so the no-double-spawn invariant is asserted directly,
not by a constant pid. 13 new `runtime.remote_*` `EventKind`s in
`crates/capo-state/src/event.rs` round-trip (`rr1_rr2_remote_runtime_event_kinds_round_trip`).
Tests: `remote_start_appends_request_then_resolve_then_started_in_order`,
`remote_start_with_same_idempotency_key_keeps_a_stable_remote_ref` (now asserts
`transport_spawn_count() == 1`), `remote_start_launch_failure_yields_typed_retryability`,
`remote_runner_performs_no_endpoint_resolution`,
`remote_cleanup_after_spawn_is_idempotent_and_emits_completed`,
`remote_health_probe_overrides_a_stale_running_status` (probe overrides stored status).

Scope: A real remote runner whose `start_process` / `interrupt` / `terminate` /
`kill` / `health` cross a machine boundary over a `connectivity-tunnel`-provided
channel, implementing `runtime-tunnel.md`'s append-first Start Sequence. Replaces
the loopback-decorating stub and its `FakeRuntimeRunner` control fall-through.

Acceptance criteria:

- Define a `RemoteRunner` execution contract that the controller drives with the
  SAME method shapes as `LocalProcessRunner`
  (`start_process`/`write_stdin`/`interrupt`/`terminate`/`kill`/`stream_output`/
  `health`/`cleanup`, `lib.rs:568-1011`), and re-point `RuntimeRunner::RemoteProcess`
  so its control methods dispatch to the real remote runner instead of the current
  `FakeRuntimeRunner` fall-through (`lib.rs:94-118`).
- The remote runner consumes a RESOLVED channel handle from `connectivity-tunnel`;
  it MUST NOT open sockets, resolve endpoints, or handle `auth_ref` itself. A test
  asserts the remote runner is constructed from an already-resolved channel and
  performs no endpoint resolution. (The existing stub's `config.local_loopback`
  construction path is removed; the runner takes the resolved channel, not a
  loopback config.)
- `start_process` over the channel: prove remote target identity (channel
  fingerprint from `connectivity-tunnel`) BEFORE launch, launch the program with
  the explicit argv / launch_mode / cwd / workspace-roots / env-allowlist on the
  remote, and return a `RuntimeProcessRef` whose `remote_process_ref` is populated
  (remote pid + remote boot/host identity) — not the local
  `external_pid`/`boot_id` path.
- Implement the append-first Start Sequence across the boundary
  (`runtime-tunnel.md` Start Sequence 1-7): `runtime.start_requested` (with
  idempotency key, status pending) is appended LOCALLY before the remote spawn;
  on success `runtime.process_started` then `run.started`; on remote launch
  failure `runtime.process_start_failed` with retryability; if the remote process
  starts but the local event append fails, the runner attempts remote cleanup and
  the next recovery classifies a live remote process without
  `runtime.process_started` as `run.orphaned`. Repeated start with the same
  idempotency key never spawns a second remote process.
- `interrupt` / `terminate` / `kill` escalate over the channel and produce the
  distinct `runtime.interrupt_sent` / `runtime.terminate_sent` / `runtime.kill_sent`
  events; `health` returns liveness derived from an ACTUAL remote probe over the
  channel (remote pid/process-group liveness), not from a local status string as
  the stub does (`lib.rs:1655`).
- Add the remote-specific event kinds to `crates/capo-state` where absent: the
  stub's bare `runtime.remote_target_resolved` / `runtime.remote_process_started`
  `RuntimeEvent`s are promoted/aligned to real `EventKind`s alongside the existing
  `runtime.*` family (`capo-state/src/event.rs`), each round-trippable through the
  codec.

Verification:

- Deterministic `FakeRemoteProcessRunner` + fake-channel tests (NO network): start
  records remote process-ref shape + the append-first event order; duplicate
  idempotency key does not double-spawn; interrupt/terminate/kill yield distinct
  events; a fake-channel "remote launch failed" yields `runtime.process_start_failed`
  with retryability; a fake "append failed after spawn" path triggers a remote
  cleanup attempt; a test asserts the runner does no endpoint resolution.
- Focused `cargo test -p capo-runtime -p capo-state`. `cargo fmt`. `git diff --check`.
- Live remote start deferred to RR8.

Dependencies: RR0. Cross-workpad: `connectivity-tunnel` (resolved channel +
fingerprint), `real-turn-loop` (confinement + start-sequence event contract),
`capo-state` runtime event family.

## RR2 - Reattach-After-Restart + Recovery Behavior Across The Boundary

Status: complete.

Evidence: `RemoteProcessRunner::recover_run` re-probes a stored remote ref over the
(re-resolved) channel and classifies it as `Recovered` / `Orphaned` / `Exited` /
`RecoveryPending` (`RemoteRecoveryClassification`), mapping each to a distinct
`runtime.remote_run_*` / `runtime.remote_recovery_pending` event after an append-first
`runtime.remote_recovery_attempted`. A remote-reboot (boot-id mismatch) is `Exited`,
never silently recovered. `reattach_supported` reports truthfully from the
`:pid=...:boot=...` tail; the ref parser (`parse_remote_ref`) reads the tail from the
END so an embedded `:pid=` substring cannot mislead it, and recovers the host segment
so a probe after channel re-resolution carries the host recorded at launch.
Deterministic fake-channel tests (NO network):
`remote_recovery_alive_reattachable_recovers_in_place`,
`remote_recovery_alive_but_unattachable_is_orphaned`,
`remote_recovery_reboot_boot_id_mismatch_is_exited_never_recovered`,
`remote_recovery_gone_is_exited_unknown_detail`,
`remote_recovery_channel_unreachable_is_pending_then_recovers_on_return` (re-resolves
the SAME stored ref against a now-reachable channel — proves the retry path, not two
independent runners), `remote_recovery_is_replay_stable_across_repeated_restarts`,
`remote_recovery_is_in_place_not_a_relaunch_with_recovery_of_run_id`,
`remote_reattach_unsupported_for_bare_ref_without_pid_boot`,
`parse_remote_ref_is_robust_to_pid_marker_inside_fingerprint`,
`probe_carries_host_from_stored_ref_not_the_reresolved_channel`. Recovery is exercised
from the `-p capo-server` gate in `crates/capo-server/src/tests/remote_recovery.rs`
(restart-with-live-remote, channel-unreachable-then-return on the SAME ref,
replay-stable rebuild).

Scope: Realize `runtime-tunnel.md`'s Recovery Behavior for remote runs: on Capo
restart, recover a remote run in place when the remote process is still alive, and
honestly classify alive / unattachable / gone.

Acceptance criteria:

- On restart, for each stored `RuntimeProcessRef` whose runner kind is
  remote_process, re-resolve the channel via `connectivity-tunnel` and call
  `health(...)` over the channel; map results to events exactly as the local path:
  alive + reattachable -> `run.recovered`; alive but not reattachable ->
  `run.orphaned` with remote logs left inspectable; gone with no terminal event ->
  `run.exited` (unknown exit detail). Mirror `LocalProcessRunner::recover_orphan`
  / `probe_run_health` (`lib.rs:1013-1162`) semantics, but the liveness signal
  comes from the remote probe.
- Report `reattach supported?` truthfully per `runtime-tunnel.md`'s Remote runtime
  responsibility: the remote runner declares whether a given remote launch can be
  reattached to (e.g. a detached/recorded remote PID + boot identity), and a run
  whose remote machine rebooted (boot-id mismatch) is classified gone, never
  silently "recovered".
- Channel loss during recovery is representable: if the channel is unreachable at
  recovery time, the run is left in a `recovery_pending` / unknown state with an
  event, NOT forced to recovered or exited; recovery retries when the channel
  returns.
- `recovery_of_run_id` is used ONLY for a relaunch/retry after restart, never for
  a simple in-place reattach (per `runtime-tunnel.md`).

Verification:

- Deterministic fake-channel recovery tests: alive remote -> recovered; alive but
  unattachable -> orphaned; remote-reboot (boot-id mismatch) -> exited; channel
  unreachable -> recovery_pending then recovered on channel return. A restart/replay
  test proves recovered projections rebuild identically.
- Focused `cargo test -p capo-runtime -p capo-server`. `cargo fmt`. `git diff --check`.

Dependencies: RR1, `connectivity-tunnel` (resolved channel, re-resolve on
restart), `safety-gates` (restart recovery + checkpoint substrate).
Cross-workpad: `connectivity-tunnel` (resolved channel + re-resolve + reachability
health).

## RR3 - Git-Based Remote Workspace Materialization (push/fetch + worktree the commit)

Status: pending.

Scope: Materialize the run's workspace ON the remote by git, then map results
back. This is the INJECTED design decision.

Acceptance criteria:

- Before a remote launch, materialize the target commit on the remote: push the
  required commit from Capo's host (or have the remote fetch it from a shared
  origin), then `git worktree add` that commit into a dedicated remote worktree
  root, reusing the `depth` DP8 worktree primitive
  (`crates/capo-runtime/src/worktree.rs`) semantics but executed remotely via the
  channel. The remote run's cwd / workspace confinement is scoped to that remote
  worktree root.
- Materialization is content-addressed and auditable: record the source commit
  SHA, the remote worktree path/key, and the resulting remote `HEAD` as a
  `runtime.remote_workspace_materialized` event; the git transport URL / remote ref
  passes redaction before the event is persisted (no embedded credentials in the
  recorded URL).
- Uncommitted / untracked scratch is NOT auto-synced: assert (and document in the
  event/read-model) that a remote run sees only the materialized commit; a test
  proves a dirty local file is absent on the remote worktree. The non-sync is an
  explicit, recorded fact, not a silent gap.
- Map results back by git: the remote worktree's produced commit(s) / branch are
  fetched back into Capo's host as a named ref (mirroring DP8's reconcile/merge-back
  point) and recorded as `runtime.remote_workspace_reconciled`; teardown of the
  remote worktree is an event (`runtime.remote_workspace_torn_down`), never silently
  abandoned, so a remote worktree is reconstructable/inspectable after restart.
- A failed remote `git worktree add` / push / fetch is a TYPED error surfaced as a
  failed materialization event, never a silent fall-through to running in the wrong
  directory (mirroring `WorktreeError`'s no-silent-fallthrough rule).

Verification:

- Deterministic tests against a LOCAL bare-repo + a "remote" that is a second local
  checkout reached through the FAKE channel (no network): materialize a known commit
  -> the worktree HEAD matches the SHA; a dirty/untracked local file is absent on the
  materialized worktree; produced remote commit fetches back as a named ref;
  materialization failure is a typed event. Restart/replay proves the materialization
  + reconcile events rebuild identically.
- Focused `cargo test -p capo-runtime`. `cargo fmt`. `git diff --check`.

Dependencies: RR1, `depth` DP8 (worktree primitive). Cross-workpad:
`connectivity-tunnel` (channel for the git transport control path).

## RR4 - Remote Output-Delta + Stdin Streaming Over The Channel (reuse RuntimeProcessRef)

Status: complete.

Evidence: `RemoteProcessRunner::stream_output`/`stream_output_with` forward remote
stdout/stderr over the injected `RemoteChannel` as ordered, REDACTED,
offset-tagged `runtime.remote_output_delta` events terminated by a single
`runtime.remote_stream_finalized` (Eof / CapReached / ChannelDropped reason);
`write_stdin` writes bytes to the remote over the channel and emits
`runtime.remote_stdin_written` (byte count only, never the payload). The SAME
opaque `LocalRuntimeProcessRef` identifies the run (only `remote_process_ref`
populated) — no parallel remote-only stream type. Redaction is applied at the
remote boundary via the existing `RedactionPolicy` credential-shape scan BEFORE
any delta/event, and output is bounded by `REMOTE_OUTPUT_LIMIT_BYTES` (mirrors
`output_limit_bytes`). Deltas carry a MONOTONIC `offset` + `raw_len` and the
outcome's `next_offset` is the reconnect resume point (reuses the
`streaming-transport` `from_sequence` discipline). The fake channel
(`FakeRemoteChannel::with_streamed_output` / `with_stream_drop_after` /
`stdin_written` / `stream` / `write_stdin`) models the stream deterministically
(NO network). 3 new round-trippable `EventKind`s in `crates/capo-state`
(`runtime.remote_output_delta` / `runtime.remote_stdin_written` /
`runtime.remote_stream_finalized`,
`rr1_rr2_remote_runtime_event_kinds_round_trip` extended).
Deterministic fake-channel tests (NO network):
`remote_stream_projects_ordered_deltas_once_with_monotonic_offsets`,
`remote_stream_redacts_a_credential_before_any_delta_or_artifact`,
`remote_stream_channel_drop_finalizes_with_a_recorded_reason`,
`remote_stream_is_bounded_by_the_output_cap`,
`remote_stdin_write_reaches_the_fake_remote_process`,
`remote_stream_reconnect_resumes_from_last_offset_without_duplicates`,
`remote_stream_is_replay_stable_across_repeated_reads`.

Scope: Stream remote stdout/stderr deltas and write stdin over the channel,
reusing the `streaming-transport` event model and the existing piped-process
surface, so a remote run is observable/steerable exactly like a local one.

Acceptance criteria:

- Implement `stream_output(RuntimeProcessRef)` and `write_stdin(RuntimeProcessRef,
  Bytes)` over the channel, reusing the `streaming-transport` `runtime.output_delta`
  / `runtime.stdin_written` event model and the existing piped-process line-protocol
  surface (`LocalProcessRunner::spawn_piped_process` / `PipedRunningProcess`,
  `lib.rs:740`; async surface `AsyncLocalProcessRunner` / `StreamSource` in
  `async_runner.rs`) rather than a parallel remote-only stream type. The
  `RuntimeProcessRef` is the SAME opaque reference; only its `remote_process_ref`
  is populated.
- Output deltas are bounded + redacted BEFORE leaving the remote / before being
  persisted as artifacts: apply the existing `RedactionPolicy` (credential-shape
  scan, `lib.rs:234-291`) and `output_limit_bytes` cap to remote output, and stamp
  `redaction_state` on the artifact and delta events. A test feeds a credential-shaped
  token through the remote stream and asserts it is scrubbed before any artifact/event.
- Backpressure / partial-frame safety over the channel: a stalled or slow remote does
  not hang the controller turn (reuse the read-deadline discipline the ACP wire client
  established); a channel drop mid-stream finalizes the delta stream with a recorded
  reason rather than a silent truncation.
- Ordering + idempotency: remote output deltas carry monotonic offsets so a reconnect
  replays from the last acknowledged offset without duplicating already-projected
  deltas (reuse the `streaming-transport` `from_sequence` discipline).

Verification:

- Deterministic fake-channel streaming tests: ordered deltas project once; a
  credential token is redacted before persistence; a mid-stream channel drop finalizes
  with a reason; stdin write reaches the fake remote process; a reconnect resumes from
  the last offset with no duplicate deltas.
- Focused `cargo test -p capo-runtime -p capo-state`. `cargo fmt`. `git diff --check`.

Dependencies: RR1, `streaming-transport` (output-delta/stdin event model +
from_sequence). Cross-workpad: `connectivity-tunnel` (stdio channel kind).

## RR5 - Compose OS Sandbox + Worktree Isolation ON The Remote (honest claims)

Status: complete.

Evidence: `RemoteProcessRunner::plan_remote_sandbox` /
`start_process_sandboxed` (`crates/capo-runtime/src/lib.rs`) compose the `depth`
DP7 `SandboxTier` + `SandboxProfile` + `SandboxRefusal` + `SandboxEnforcement`
(reused, not re-authored) with the remote worktree root as the confined cwd. The
enforcement claim is decided by a REMOTE-OS probe over the channel
(`RemoteChannel::sandbox_probe` -> `RemoteSandboxProbe { os_family, tier_enforceable }`),
NOT `SandboxTier::is_enforced_here()` (the controller host): `RemoteOsFamily::enforces`
is the remote analogue of DP7's `is_enforced_here`.

ENFORCEMENT IS APPLIED, NOT JUST CLAIMED (review finding 1 + 3): when the plan is
`Enforced`, `plan_remote_sandbox` REWRITES the request to launch the original
program under the remote OS sandbox launcher (`bwrap` on a linux remote,
`/usr/bin/sandbox-exec -f <policy>` on a macOS remote) via the reused
`OsSandbox::wrap_command_for_remote` argv-builder, and carries that wrapped argv on
`RemoteSandboxPlan::wrapped_request`. `start_process_sandboxed` launches the WRAPPED
request, so the transport actually receives the `bwrap`/`sandbox-exec` command — the
additional enforcement layer over the path-prefix confinement, not merely an event
label. `remote_sandbox_is_enforced_when_the_remote_os_supports_the_tier` asserts
`wrapped_request.program == "bwrap"`, that the original `/bin/sh` is an argv token
under it, that a network-forbidding profile injects `--unshare-net`, and that
`transport_last_launched_request().program == "bwrap"` (the enforcement reached the
transport). No assertion rests on self-attestation of the event label.

LOOPBACK HONESTY (review finding 2 + 7): a loopback / fake channel never crossed a
machine boundary, so `plan_remote_sandbox` short-circuits an `is_loopback()`
transport to `SandboxEnforcement::Unenforced` (`reason` names the loopback) even
when the channel scripts an enforcing remote OS — Capo never claims a
`bwrap`/`sandbox-exec` confinement it could not apply over a boundary it did not
cross. The default `FakeRemoteChannel` is therefore HONESTLY unenforceable
(`sandbox_unenforceable: true`, `cross_machine: false`); a test that exercises the
ENFORCED wrapping path opts in explicitly with
`with_cross_machine_boundary().with_enforceable_remote_sandbox()` (modelling a real
SSH remote; the live cross-machine proof is RR8).
`remote_sandbox_loopback_channel_is_never_enforced_even_with_enforcing_remote_os`
pins this.

An un-granted critical scope (network egress under a forbidding profile, or a cwd
outside the confined remote worktree root) is REFUSED before any spawn
(`SandboxEnforcement::Refused` + `sandbox.launch_refused`, `transport_spawn_count()==0`);
an unsupported remote OS (or a matching family that lacks the mechanism, or a
loopback channel) is `SandboxEnforcement::Unenforced` + `sandbox.unenforced` (Capo
does NOT claim sandboxing); a cross-machine remote whose OS enforces the tier is
`SandboxEnforcement::Enforced` + `sandbox.enforced`. The three sandbox events are
promoted to typed `EventKind`s in `crates/capo-state` (`SandboxEnforced` /
`SandboxUnenforced` / `SandboxLaunchRefused`), each round-tripping through the codec
(`rr1_rr2_remote_runtime_event_kinds_round_trip` extended) — consistent with the
RR1-RR4 event-kind pattern (review finding 6). A successful confined run carries the
reversible checkpoint (`checkpoint_ref`, the RR3 git-materialized commit ref) so the
sandbox is additive to rollback. `RuntimeRunnerContract` now includes `start_process`
so the spawn-path shape is compile-time parity-checked across both runners; the
divergent `stream_output`/`write_stdin` surfaces are documented on the trait rather
than forced into a false parity (review finding 5). The `FakeRemoteChannel` scripts
the remote OS + boundary deterministically (`with_remote_os` /
`with_unenforceable_remote_sandbox` / `with_enforceable_remote_sandbox` /
`with_cross_machine_boundary`, NO network). Deterministic tests (NO network):
`remote_sandbox_refuses_ungranted_network_egress_before_launch`,
`remote_sandbox_refuses_cwd_outside_confined_remote_root_before_launch`,
`remote_sandbox_is_enforced_when_the_remote_os_supports_the_tier` (asserts the
`bwrap` wrapping reached the transport),
`remote_sandbox_loopback_channel_is_never_enforced_even_with_enforcing_remote_os`,
`remote_sandbox_is_unenforced_and_recorded_when_remote_os_cannot_enforce`,
`remote_sandbox_unenforced_when_remote_lacks_the_mechanism_even_on_matching_family`,
`remote_sandbox_enforcement_reads_the_remote_os_not_the_controller_host`,
`remote_sandbox_plan_is_replay_stable`. The platform-gated REMOTE refusal-mode
smoke (the bwrap/sandbox-exec launcher ACTUALLY refusing an out-of-root write on a
real remote host) stays deferred to RR8: the deterministic suite proves the wrapped
command is composed + handed to the transport, but a real launcher refusing a real
escape requires a real remote OS and is behind the opt-in gate.

Scope: Run the remote process inside the `depth` OS sandbox tier and the git
worktree on the REMOTE host, and claim enforcement only where the remote OS
actually enforces it.

Acceptance criteria:

- The remote launch composes the `depth` DP7 sandbox tier (`OsSandbox` /
  `SandboxTier`, `crates/capo-runtime/src/sandbox.rs`) and the DP8 worktree
  (`crates/capo-runtime/src/worktree.rs`) executed on the REMOTE host: the remote
  process runs confined to the remote worktree root with the granted
  filesystem-write / network-egress scopes, as an additional enforcement layer over
  the path-prefix confinement.
- Enforcement honesty: the remote runner reports `SandboxEnforcement::{Enforced,
  Unenforced}` based on what the REMOTE OS supports (probed over the channel), NOT
  what the controller's host supports. If the remote OS cannot enforce the requested
  tier, the runner records `sandbox.unenforced` for the remote run and Capo does NOT
  claim sandboxing — mirroring DP7's `is_enforced_here()` rule but evaluated for the
  remote host.
- The sandbox decision is wired to the `safety-gates` capability scopes exactly as
  DP7: an un-granted critical scope (e.g. network egress under a forbidding profile,
  or a cwd outside the confined remote root) is REFUSED before the remote sandbox
  launches, recorded as `sandbox.launch_refused`, with no remote process spawned.
- A successful confined remote run still produces a reversible checkpoint
  (git-fetched-back ref from RR3) so the sandbox is additive, not a replacement for
  rollback.

Verification:

- Deterministic tests: a remote launch requesting an un-granted critical scope is
  refused pre-launch (platform-independent); the remote-OS enforcement claim is
  `Unenforced` + recorded when the fake remote reports an unsupported tier; the
  refusal/unenforced facts are events, not silent failures.
- Platform-gated REMOTE refusal-mode smoke (out-of-root write refused on the remote)
  deferred to RR8 behind the opt-in gate and skipping cleanly.
- Focused `cargo test -p capo-runtime`. `cargo fmt`. `git diff --check`.

Must not do: do not claim sandboxing on a remote OS where Capo cannot enforce it;
record the remote-host limitation instead.

Dependencies: RR3, `depth` DP7 + DP8, `safety-gates` (capability scopes + grants).

## RR6 - Crash-Safe Remote Runs + Recovery Events

Status: pending.

Scope: Make a remote run crash-safe end to end: the controller can crash, the
remote can crash, or the channel can drop, and Capo recovers to a truthful,
auditable state.

Acceptance criteria:

- Enumerate and handle the remote-specific failure modes as recorded states/events
  (extending `runtime-tunnel.md`'s local failure list): remote process survives a
  controller restart; channel dropped mid-run; remote host rebooted; remote git
  worktree left dangling; remote output continued after a local timeout. Each maps to
  a distinct recovery classification + event, never a silent loss.
- Compose with `safety-gates` checkpoint/rollback: a remote run's pre-write checkpoint
  is the materialized commit (RR3); a rollback restores the remote worktree to that
  checkpoint and records it. A run interrupted by a channel drop can be resumed or
  cleanly failed, with the workspace recoverable from git.
- Cleanup is idempotent + auditable: `cleanup(RuntimeProcessRef, CleanupPolicy)` over
  the channel removes the remote worktree + reaps the remote process group, emits
  `runtime.cleanup_completed`, and is safe to re-run after a partial failure (mirroring
  `LocalProcessRunner::cleanup`, `lib.rs:1001`).
- No orphaned remote capability: a revoked channel / revoked remote-control grant must
  stop the remote run and the runner must not be able to re-establish execution without
  a fresh grant (auditable + revocable per the safety boundary).

Verification:

- Deterministic fake-channel crash-matrix tests: controller-restart-with-live-remote
  -> recovered; channel-drop-mid-run -> resumable/failed cleanly; remote-reboot ->
  exited; dangling remote worktree -> reaped on cleanup; revoked grant -> remote run
  stopped and not re-establishable. Restart/replay proves identical rebuilds.
- Focused `cargo test -p capo-runtime -p capo-server`. `cargo fmt`. `git diff --check`.

Dependencies: RR1-RR4, `safety-gates` (checkpoint/rollback + grant revoke).

## RR7 - Deterministic Fake-Remote Determinism Consolidation

Status: pending.

Scope: Consolidate the deterministic suite that must pass with NO network and NO
real remote, proving every remote-runtime invariant before any live work.

Acceptance criteria:

- A `FakeRemoteProcessRunner` + fake-channel harness that exercises the full
  contract (start/stop/health/reattach, git materialization against a local
  bare-repo "remote", output/stdin streaming, sandbox/worktree composition,
  crash-matrix recovery) with deterministic, replay-stable outputs.
- Assert the cross-cutting invariants end to end: the remote runner performs no
  endpoint resolution (channel is injected); the append-first start sequence holds;
  uncommitted scratch is never materialized; output is redacted + bounded before
  persistence; sandbox enforcement claims match the (fake) remote OS; recovery
  classifications are truthful; cleanup is idempotent.
- Every assertion is replay-stable: a restart/rebuild reproduces identical projected
  state for the remote process-lifecycle, materialization, streaming, and
  recovery-event paths.

Verification:

- Restart/replay tests across the lifecycle / materialization / streaming /
  recovery paths proving identical rebuilds.
- Focused `cargo test -p capo-runtime -p capo-state -p capo-server`, widening to
  `cargo test` if shared behavior changes broadly. `cargo fmt`. `git diff --check`.

Dependencies: RR1-RR6.

## RR8 - Live Opt-In Remote SSH Smoke (Secrets Stripped) Paired With Deterministic Assertions

Status: pending.

Scope: One real `SshRemoteProcessRunner` smoke against a real SSH host, behind an
explicit opt-in env gate, paired with the deterministic fixture and skipping
cleanly when no host is configured.

Acceptance criteria:

- Add a live opt-in remote smoke behind explicit env gates mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE` / `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` (i.e.
  `CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE` + `CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT` +
  a host/endpoint config), `#[ignore]` by default, that against a REAL SSH host:
  resolves the channel via `connectivity-tunnel`, materializes a known commit by git,
  runs one real confined process, streams real stdout deltas, and recovers a
  controller-restart-with-live-remote.
- The channel auth is carried strictly by HANDLE (`auth_ref` resolved by
  `connectivity-tunnel`); the smoke MUST NOT read or log raw SSH keys / known_hosts
  secrets / subscription tokens. Remote stdout/stderr and the git transport URL pass
  the credential scan (`scan_artifacts_for_sensitive_markers` discipline) and any
  `unknown` / `contains_sensitive` artifact is quarantined or dropped.
- Pair every live smoke with a deterministic assertion: the same fake-remote fixture
  pins the identical process-ref shape, materialized-HEAD-matches-SHA, redacted-output,
  and recovery-classification shape, so completion is never solely operator-attested.
- Confirm the safety floors engage on the live path: the remote run executes inside the
  remote sandbox + worktree, under the `safety-gates` `PermissionPolicy` and a revocable
  remote-control grant; revoking the grant stops the live remote run.
- Skips cleanly (test reports skipped, not failed) when no SSH host / gate is configured.

Verification:

- `cargo fmt`. Focused `cargo test -p capo-runtime -p capo-server`, widening if
  shared behavior changes. Live SSH smoke behind the opt-in gate with secrets
  stripped, each paired with its deterministic fixture assertion. `git diff --check`.
- Review notes: runner-owns-execution / tunnel-owns-reachability held; git-only sync
  honored; remote sandbox claims honest; remote-control auditable + revocable;
  decision whether to deepen (Tailscale transport, Capo worker daemon, container
  runner) or close `remote-runtime`.

Dependencies: RR1-RR7 green, a real SSH host, `connectivity-tunnel` live channel.
