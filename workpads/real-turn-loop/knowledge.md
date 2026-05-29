# Real Turn Loop Knowledge

## Objective

Stand up a genuine controller turn loop (observe normalized adapter events ->
update projections -> emit `TurnFinished`) that DRIVES the existing dispatch
primitives (`PlanDispatch`/`GateDispatch`/`RunDispatchLocal`/
`RunLiveProviderLocal`) as one orchestration path, executes one real
workspace-write adapter (Codex) end-to-end, keys artifacts per `turn_id`, and
gates the first real write behind a confinement/kill/checkpoint/
resource-ceiling/dry-run safety floor. This is Phase 1: the substrate every
later workpad attaches to.

## Scope Decision

Create a new `real-turn-loop` workpad.

This is not an extension of `operator-control`. Operator-control is a client
surface: it renders state and lowers operator commands into typed server
requests. The turn loop is controller/server behavior - it observes adapter
events, projects them, drives dispatch, and emits turn outcomes. Putting the
loop in the client would move orchestration into a surface that the boundary
model says must only submit commands and render read models.

This is not `goal-orchestration`. The reconciliation point is sharp:
`goal-orchestration` remains the authoritative design source for the durable
goal model, the GO2 agent-reporting contract, the evidence/review/validation
ledgers, story projections, and continuation. `real-turn-loop` owns the
single-turn observe/decide/emit substrate underneath all of that. Goal
continuation, the evidence-gated completion auditor, and the goal model are
explicitly deferred to `goal-autonomy`, which implements them on the now-real
substrate this workpad builds; it does not re-specify the design. One design
brain (`goal-orchestration`), one implementation of the loop (`real-turn-loop`),
no competing turn vocabularies.

This workpad is Phase 1 of the sequence and the substrate everything else
attaches to. `streaming-transport`, `tools-aci`, `safety-gates`, `goal-autonomy`,
and `depth` all assume a real observe->decide->emit loop, a real workspace-write
adapter, per-turn artifacts, and an orphan-reaping spawn path. Today none of
those exist: the daily-driver review confirmed (2026-05-29) that default chat
routes `SteerAgent` into `FakeBoundaryController::redirect` writing a canned
`latest_summary`, the `AgentAdapter` is a closed `Fake`/`ScriptedMock` enum, and
the only live path is a read-only one-shot `codex exec --sandbox read-only
--ephemeral` parsed after exit that reuses one `stdout.txt`. The loop must be
made real before anything can be streamed, gated, or made autonomous.

The boundary this workpad owns: the turn loop, the `AgentAdapter` trait, the
Codex workspace-write adapter, per-turn artifact keying, crash-safe in-flight
runs, and the RTL safety floor. It explicitly defers streaming/SSE to
`streaming-transport`; full `PermissionPolicy`/`ToolExposure` enforcement,
`VerificationRunner`/`score_run`, and full shadow-git to `safety-gates`; the
goal model/continuation/auditor to `goal-autonomy`; and the live ACP JSON-RPC
adapter, Claude write adapter, and real memory retrieval to `depth`. The
dependency posture carries no new workpad prerequisites: it builds on
`operator-control` complete (server boundary + control REPL), the event-sourced
SQLite state core, the typed `ServerCommand` boundary, and the existing dispatch
state machine.

## The Loop Drives Dispatch Rather Than Running Beside It

The single most important design decision is that the turn loop has exactly one
execution path. The dispatch state machine is already a genuine multi-step
orchestration path: `PlanDispatch` -> `PreflightLiveProvider` -> `GateDispatch`
-> `RunDispatchLocal`/`RunLiveProviderLocal` are real `ServerCommand`s
(`crates/capo-server/src/types.rs:150-186`, handled in `lib.rs:535-826`), and
`run_live_provider_local` (`crates/capo-server/src/live_provider.rs:357`) is the
path that actually spawns the live Codex run today. The hazard is obvious: a new
loop that re-runs plan/gate/run beside the existing commands would be the exact
parallel-orchestration-path failure the boundary model warns against - two
sources of run-completion truth, two gate paths, two places to drift.

Design rationale:

- The loop's emit step DRIVES the dispatch primitives as its execution
  substrate. A loop turn produces the same plan/gate/execution event sequence as
  the direct command path for a scripted run; the loop subsumes dispatch, it does
  not duplicate it.
- The loop must reuse the existing gate (`dispatch_gate_for_plan`,
  `crates/capo-server/src/dispatch.rs:62`) and preflight
  (`preflight_live_provider`, `live_provider.rs:62`). No `RealBoundaryController`
  method may run a provider without passing through that gate/preflight - this is
  recorded as a code-review non-goal assertion, not just prose.
- Open question resolved-leaning: the loop invokes the dispatch
  `ServerCommand`s/server methods rather than re-implementing them, so the typed
  boundary stays the single definition of plan/gate/run. The alternative
  (demoting the dispatch primitives to internal loop functions) is recorded as
  the fallback if the call shape proves awkward; the chosen shape is documented
  here in `knowledge.md` per RTL4.

## The AgentAdapter Trait Seam

Today the adapter surface is a closed enum that speaks `Fake*` vocabulary at
every call site. `AgentAdapter::open_session`/`send_turn`/`attach_session`
return concrete `FakeAdapterSession`/`FakeAdapterTurnOutput` and take
`FakeAdapterSessionRequest`/`FakeAdapterTurnRequest`
(`crates/capo-adapters/src/adapter.rs`), and the controller imports those
concrete types directly (`crates/capo-controller/src/lib.rs:9-13`). A real
provider cannot be added without renaming the seam.

Design rationale:

- Define an `AgentAdapter` trait with provider-neutral methods
  (`open_session`, `send_turn`, `attach_session`, `interrupt`, `stop`,
  `binding`) over provider-neutral types (`AdapterSessionRequest`, `TurnRequest`,
  `TurnOutput`, `AdapterSession`). The trait realizes the implementation-facing
  adapter contract already specified in
  `workpads/architecture/protocol-provider.md` (`Adapter Contract`).
- The turn output shape keeps carrying `turn_id`, `external_session_ref`,
  `summary`, `confidence`, `status`, and the observed tool name so the existing
  controller projection wiring keeps working unchanged.
- Normalized output is expressed as `NormalizedAdapterEvent` so the trait feeds
  the existing `apply_normalized_adapter_events_with_turn` path, not a new
  ingestion route.
- Splitting this into "define the trait" (RTL1) and "reimplement Fake/Scripted
  against it and migrate every caller" (RTL2) is deliberate: the trait forces an
  abstract-type redesign that ripples cross-crate, and folding the migration into
  one task would hide a refactor that silently doubles the workpad's size. The
  acceptance bar is concrete: no concrete `Fake*` type appears at any non-fake
  call site, and `Fake`/`ScriptedMock` simply become the first two trait impls
  with byte-for-byte identical deterministic output.

## The RTL Safety Floor

`real-turn-loop` ships a live workspace-WRITE adapter in Phase 1 while full
`PermissionPolicy`/`VerificationRunner`/shadow-git land later in `safety-gates`.
That is three workpads of a live model editing a real repo with `TrustedLocal`
allow-all still in force, no rollback, and unbounded provider spend. The hazard
was relocated earlier in the sequence, not removed - so a minimal floor must
exist the moment the first real write does. The floor makes the first write
confined, reversible, bounded, and never unattended.

Design rationale:

- Confinement: wire the existing path-containment engine
  (`normalize_policy_path` at `crates/capo-server/src/live_provider.rs:686`, and
  `ensure_under_workspace` at `crates/capo-tools/src/runtime_wrapper_paths.rs:77`)
  so a write outside the confined workspace is rejected before any process runs -
  proven by a test that asserts no process is spawned.
- Hard kill: a controller-owned kill that terminates the run's process group
  (reusing the runtime's process-group kill path) mid-run and records the abort
  as an event.
- Reversibility: a single-snapshot pre-write checkpoint (git-stash, tar copy, or
  worktree snapshot for Phase 1 - full shadow-git stays in `safety-gates`)
  recorded via a `checkpoint.created`-style event so any RTL live write is
  reversible by one documented restore command, proven by a create/restore test.
- Never unattended: diff-preview/dry-run is the default; a live write requires an
  explicit opt-in env gate mirroring `CAPO_SERVER_RUN_CODEX_LIVE` /
  `live_execution_opt_in` (`live_provider.rs:47,475`) AND is diff-previewed.
  Unattended continuation is `goal-autonomy`-only, on the `safety-gates`
  substrate.
- Resource ceiling: a per-run max turns, max wall-clock (wired to the existing
  `wait_running_with_timeout` path, `live_provider.rs:573`), and a hard
  token/cost ceiling. Exceeding any ceiling emits a `run.aborted`-style event and
  stops the run through the hard kill. The live Codex path must always run inside
  an active ceiling, never without one. This ceiling is a strict subset of
  `goal-autonomy`'s `GoalBudget`, which extends this enforcement floor rather than
  replacing it.

## Per-Turn Artifact Keying And Crash-Safe In-Flight Runs

Two correctness gaps surface the moment the loop drives real, long-running
processes across multiple turns.

Per-turn artifact keying:

- The runtime keys `run_dir = artifact_root/run_id` and reuses a single
  `stdout.txt`/`stderr.txt` (`crates/capo-runtime/src/lib.rs:322-325`), so a
  second turn in the same run overwrites the first turn's artifacts. Thread
  `turn_id` through `LocalProcessRequest` and the runtime artifact path so each
  turn keeps distinct stdout/stderr.
- `TurnFinished` and per-turn artifacts reconcile with the EXISTING dispatch
  run-exit events (`append_dispatch_run_exit` /
  `append_dispatch_run_exit_with_metadata`,
  `crates/capo-server/src/dispatch.rs:485-547`) and execution-status projections
  (`AdapterDispatchExecutionProjection`). There is a single authoritative notion
  of completion: `TurnFinished` ANNOTATES the dispatch execution/run-exit events;
  it does not fork a second run-completion model.

Crash-safe in-flight runs:

- The runtime already emits `runtime.start_requested` and records
  `external_pid`/`process_group` (`lib.rs:344-369`); the server must persist
  start-requested plus the pid/process-group reference before `spawn_process`
  returns on the live path, so a crash mid-spawn is recoverable.
- On restart, reap orphaned process groups with the proven descendant reaper
  instead of the blunt `mark_active_runs_exited_unknown`
  (`crates/capo-controller/src/lib.rs:166`, `crates/capo-state/src/lib.rs:251`),
  which orphans children. Emit `run.orphaned`/`run.recovered`/`run.exited`
  consistent with the `state-model.md` Restart Recovery section, keyed by
  `(run_id, recovery_observation_kind, observed_runtime_state_hash)` for
  idempotency.
- Phase 1 reaps and records only; full liveness-probe reattach stays in
  `safety-gates`.

## New Event Kinds And The Read-Model Contract

The loop maps onto existing event kinds wherever possible:
`session.started`/`session.summary_updated`/`run.started`/`run.exited` and the
`adapter.dispatch_*` and `tool.*` families already exist
(`crates/capo-state/src/event.rs:59-107`). Where a new `EventKind` is genuinely
required - `run.aborted` for the resource ceiling (RTL7), and
`run.orphaned`/`run.recovered` for crash recovery (RTL10), all currently absent
from `event.rs` though `state-model.md` already designs the latter two - it is
added with an idempotency key and a projection, and exercised by a replay test.
The loop must not invent a parallel turn vocabulary when the existing kinds
suffice.

The whole loop is pure and synchronous for Phase 1: one observe -> project ->
emit cycle per turn, deterministic over scripted input. Streaming is
`streaming-transport`. The `RealBoundaryController` mirrors the
`FakeBoundaryController` constructor surface
(`open`/`open_with_permission_policy`) and the controller methods the server
calls, persists through `SqliteStateStore::append_event` exactly as the fake
path does, and keeps the typed `ServerCommand`/`ServerResponse` boundary in
`crates/capo-server/src/types.rs` unchanged so the controller swap is invisible
to clients.

## Parity Cutover And The Verification Invariant

The default routing flip is gated, not assumed. `RealBoundaryController` must
pass the IDENTICAL deterministic suite (`send`/`steer`/`interrupt`/`stop`,
restart/replay) that `FakeBoundaryController` passes, plus a parity-equivalence
test asserting the fake and real paths produce equivalent event sequences for a
scripted turn (modulo adapter-identity fields). Routing selects the controller
behind a single typed config switch (not scattered booleans); the scripted-mock
adapter stays an explicit fallback so the parity suite can run the real
controller over scripted input. Before the flip, the fake remains default and
the real path is opt-in; the flip happens only after parity passes and carries a
documented rollback recorded here.

The workpad-wide verification invariant: no task completes on operator
self-attestation alone. Every manual smoke is paired with a deterministic
assertion - a wire/event snapshot, an exit status, or a restart/replay - and the
live Codex write smoke is paired with the same scripted/mock fixture asserting
the identical normalized-event and artifact shape. Live-provider work stays
behind explicit opt-in env gates mirroring
`CAPO_SERVER_RUN_CODEX_LIVE`/`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`, and all smoke
artifacts pass the existing credential scan
(`scan_artifacts_for_sensitive_markers`) with `unknown`/`contains_sensitive`
artifacts quarantined per the `state-model.md` artifact privacy contract.

## Non-Goals

- Do not create a second execution pipeline beside the dispatch state machine;
  the loop subsumes/drives it.
- RTL live writes are not unattended - an opt-in env gate AND a diff-preview
  default are both required; unattended continuation is `goal-autonomy` only.
- Do not stream or add SSE here; that is `streaming-transport`.
- Do not wire full `PermissionPolicy` enforcement, the `VerificationRunner`, or
  full shadow-git here; those are `safety-gates`.
- Do not implement the goal model or continuation; that is `goal-autonomy`.
- No web client.

## Open Questions

- Does the loop call the dispatch `ServerCommand`s directly, or are the dispatch
  primitives demoted to internal loop functions? The decision leans toward the
  loop driving the commands so the typed boundary stays the single definition;
  RTL4 records the final chosen shape.
- Is the pre-write checkpoint a git-stash, a tar copy, or a worktree snapshot for
  Phase 1? Full shadow-git is `safety-gates`; this floor only needs one
  reversible single-snapshot mechanism with a documented restore.
