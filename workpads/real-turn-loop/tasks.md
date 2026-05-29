# Real Turn Loop Tasks

## Objective

Replace `FakeBoundaryController` with a genuine controller turn loop that
observes normalized adapter events, updates projections, and emits
`TurnFinished`, while driving the existing dispatch primitives
(`PlanDispatch`/`PreflightLiveProvider`/`GateDispatch`/`RunDispatchLocal`/
`RunLiveProviderLocal`) as a single orchestration path. The loop executes one
real workspace-write adapter (Codex) end-to-end, keys artifacts per `turn_id`,
and gates the first real write behind a confinement/kill/checkpoint/
resource-ceiling/dry-run safety floor. This is the substrate every later
workpad (`streaming-transport`, `safety-gates`, `goal-autonomy`, `depth`)
attaches to.

## Status

Active. Phase 1 - Make the loop real. `RTL0` defines routing and scope; all
implementation tasks remain pending.

## Feature Set

- Provider-neutral session/turn/result types and an `AgentAdapter` trait that no
  longer speaks `Fake*` vocabulary at the seam.
- A normalized turn-loop contract (observe -> project -> `TurnFinished`) that
  drives, rather than duplicates, the dispatch state machine.
- `RealBoundaryController` behind the unchanged typed server boundary, reaching
  parity with `FakeBoundaryController` before the default routing flip.
- One real Codex workspace-write adapter with a live tool-result round-trip,
  opt-in gated and dry-run by default.
- A minimal safety floor: enforced path confinement, controller hard-kill, a
  single-snapshot pre-write checkpoint, and a per-run resource ceiling.
- Per-turn artifact keying by `turn_id`, reconciled with dispatch run-exit
  events and execution-status projections.
- Crash-safe in-flight run handling for real long-running processes.
- A single-switch parity cutover with documented rollback.

## RTL0 - Workpad, Routing, Scope, And Per-Task Verification Invariant

Status: pending.

Acceptance:

- Decide that the real turn loop is its own workpad and not an extension of
  `operator-control` or `goal-orchestration`; record why in `knowledge.md`.
- Write a reconciliation note distinguishing this workpad from
  `goal-orchestration`: `goal-orchestration` owns the durable goal model and
  continuation; `real-turn-loop` owns the single-turn observe/decide/emit
  substrate that goal continuation will later drive.
- List the boundaries this workpad owns (turn loop, `AgentAdapter` trait, Codex
  workspace-write adapter, per-turn artifact keying, RTL safety floor) and the
  ones it explicitly defers (streaming/SSE to `streaming-transport`; full
  `PermissionPolicy`/`VerificationRunner`/shadow-git to `safety-gates`; goal
  model/continuation/auditor to `goal-autonomy`; live ACP/Claude/memory to
  `depth`).
- State the dependency posture: no new workpad prerequisites beyond
  `operator-control` complete, the event-sourced SQLite state core, the typed
  `ServerCommand` boundary, and the existing dispatch state machine.
- Record the workpad-wide verification invariant: no task completes on operator
  self-attestation alone; every manual smoke is paired with a deterministic
  assertion (wire snapshot, exit status, or restart/replay).

Evidence:

- `workpads/real-turn-loop/tasks.md`
- `workpads/real-turn-loop/knowledge.md`
- `workpads/real-turn-loop/references.md`

## RTL1 - Provider-Neutral Session/Turn/Result Types And The AgentAdapter Trait

Status: done (gate green; autostart flake in `server_transport` repaired - see
Evidence). `AgentAdapter` is now a provider-neutral trait
(`crates/capo-adapters/src/adapter.rs`) over `AdapterSessionRequest`/
`TurnRequest`/`TurnOutput`/`AdapterSession`; `FakeAdapter` and
`ScriptedMockAgent` are its first two implementations. The old `AgentAdapter`
dispatch enum is renamed `AgentAdapterHandle` (a thin dispatch enum over trait
impls, also implementing the trait) so RTL2's "enum becomes trait object or thin
dispatch enum" stays open. No non-fake call site names a `Fake*` request/output
type anymore; the controller imports the renamed neutral types. The turn output
shape (`turn_id`/`external_session_ref`/`summary`/`confidence`/`status`/
`tool_name`) is unchanged, and `scripted_turn_events` still returns
`Vec<NormalizedAdapterEvent>` feeding `apply_normalized_adapter_events_with_turn`.

Evidence:

- Trait + neutral types + handle: `crates/capo-adapters/src/adapter.rs`;
  exports updated in `crates/capo-adapters/src/lib.rs`;
  `ScriptedMockAgent` now `impl AgentAdapter` in
  `crates/capo-adapters/src/scripted_mock_agent.rs`.
- Callers migrated off `Fake*`-named seam types:
  `crates/capo-controller/src/{lib.rs,fake_session.rs,session_control.rs,adapter_replay.rs}`
  and `crates/capo-adapters/src/tests.rs` (now `AgentAdapterHandle::fake()`).
- New deterministic trait-construction/dispatch tests in `adapter.rs`
  (`fake_adapter_implements_provider_neutral_trait`,
  `handle_dispatches_through_the_trait`,
  `scripted_mock_routes_through_handle_and_trait`).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-adapters` -> ok, 27 passed / 2 ignored.
  `cargo fmt --check` -> clean. `git diff --check` -> clean.
  Objective gate: `cargo fmt --check` clean; `cargo clippy --all-targets
  --all-features -- -D warnings` clean; `cargo test --workspace` -> all
  binaries ok, 0 failed (capo-adapters 27, capo-controller 8, capo-server 29,
  capo-state 31, capo-runtime 12, capo cli 63 + server_transport 11, etc.).

- Gate repair (autostart flake): the objective gate intermittently failed in
  `crates/capo-cli/tests/server_transport/basic.rs`
  (`bare_capo_starts_control_and_autostarts_server_when_needed`,
  `assertion failed: output.contains("Dashboard")`). Root cause was a port race
  in the control-REPL autostart
  (`crates/capo-cli/src/operator_control/server_process.rs`): the throwaway-bind
  probe could see the env address transiently held by a peer loopback test
  server and wrongly conclude an already-running server existed, then connect to
  a port whose owner had exited, so the `dashboard` command rendered `error:`
  instead of a snapshot. Fix: `ensure_server_running` now distinguishes an
  explicit `--connect` (never autostart, never probe -> never consume a
  `--max-requests` budget) from an env/default address we own; for owned
  addresses it retries the bind with a bounded deadline so transiently held
  ports (peer servers, `TIME_WAIT`) ride out and only persistent occupancy is
  treated as a real server to connect to. `AutoServer` also keeps the child
  server's stdout pipe attached for the session. The probe stays a bare
  `TcpListener::bind`, so budgeted `--connect` transport tests are unaffected.
  Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-cli --test server_transport` -> ok, 11 passed, run 80x
  with 0 failures; full objective gate (`cargo fmt --check` && `cargo clippy
  --all-targets --all-features -- -D warnings` && `cargo test --workspace`) ->
  passed twice, exit 0, server_transport 11/11 each time.

Acceptance:

- Define an `AgentAdapter` trait in `crates/capo-adapters/src/adapter.rs` with
  provider-neutral methods (`open_session`, `send_turn`, `attach_session`,
  `interrupt`, `stop`, `binding`) expressed in provider-neutral types.
- Rename or wrap the `Fake*` request/output structs (`FakeAdapterSessionRequest`,
  `FakeAdapterTurnRequest`, `FakeAdapterTurnOutput`, `FakeAdapterSession` at
  `adapter.rs:147-175`) into provider-neutral trait types such as
  `AdapterSessionRequest`, `TurnRequest`, `TurnOutput`, `AdapterSession`; no
  non-fake call site references a `Fake*`-named type for these.
- Keep the turn output shape carrying `turn_id`, `external_session_ref`,
  `summary`, `confidence`, `status`, and the observed tool name so the existing
  controller projection wiring keeps working.
- Express normalized adapter output as `NormalizedAdapterEvent` so the trait
  feeds the existing `apply_normalized_adapter_events_with_turn` path rather than
  a new ingestion route.
- Document the trait as the single seam every future provider (Codex now;
  Claude/ACP later in `depth`) implements; the trait must not signal
  "fake-first" in its vocabulary.

Verification:

- Focused `cargo test -p capo-adapters` for trait construction and dispatch.
- `cargo fmt`
- `git diff --check`

## RTL2 - Reimplement Fake/ScriptedMock Against The Trait And Migrate Callers

Status: pending.

Acceptance:

- Reimplement `FakeAdapter` and `ScriptedMockAgent` as implementations of the
  `AgentAdapter` trait from RTL1; the `AgentAdapter` enum either becomes the
  trait object or remains a thin dispatch enum over trait impls.
- Migrate `crates/capo-controller/src/lib.rs` and `fake_session.rs` so the
  controller holds the adapter behind the trait and no longer names concrete
  `Fake*` request/output types at the call site (currently imported at
  `lib.rs:9-13`).
- Update `crates/capo-controller/src/tests` and any adapter-replay tests to use
  the provider-neutral types without behavior change.
- Preserve the existing deterministic outputs of `FakeAdapter` and
  `ScriptedMockAgent` so prior tests pass byte-for-byte where they assert
  summary/status/confidence.
- Keep `scripted_turn_events` available for deterministic multi-turn fixtures.

Verification:

- Focused `cargo test -p capo-adapters -p capo-controller`.
- `cargo fmt`
- `git diff --check`

## RTL3 - Normalized Turn-Loop Contract: Observe -> Project -> TurnFinished

Status: pending.

Acceptance:

- Define the turn-loop contract in `capo-controller`: a turn opens, the adapter
  produces normalized events, the controller projects them, and the loop emits a
  `TurnFinished` outcome with stop reason, summary refs, and observed tool refs.
- Map the loop's lifecycle onto existing event kinds (`turn.started`/`item.*`/
  `turn.completed`/`run.exited`) rather than inventing a parallel turn vocabulary;
  if a new `EventKind` is required, add it in `crates/capo-state/src/event.rs`
  with an idempotency key and projection.
- Make the loop pure and synchronous for phase 1 (no streaming): one observe ->
  project -> emit cycle per turn, deterministic over scripted input.
- Define how the loop consumes `NormalizedAdapterEvent` batches through the
  existing `apply_normalized_adapter_events_with_turn` path.
- Specify `TurnFinished` semantics for normal completion, interrupt, and stop so
  the existing `interrupt`/`stop` controller commands map onto the loop.

Verification:

- Focused `cargo test -p capo-controller` for a scripted single-turn cycle.
- Restart/replay test proving the projected turn rebuilds identically.
- `cargo fmt`

## RTL4 - Reconcile The Turn Loop With The Existing Dispatch Pipeline

Status: pending.

Acceptance:

- Decide and document that the loop's emit step DRIVES the existing dispatch
  primitives (`PlanDispatch` -> `PreflightLiveProvider` -> `GateDispatch` ->
  `RunDispatchLocal`/`RunLiveProviderLocal`) as its execution substrate; the
  loop is one orchestration path, not a second pipeline beside the dispatch
  state machine.
- Make the live Codex execution that already flows through
  `run_live_provider_local` (`crates/capo-server/src/live_provider.rs:357`) a
  step inside the loop rather than a separately invoked command sequence.
- Decide the call shape: the loop invokes the dispatch `ServerCommand`s/server
  methods, or the dispatch primitives are demoted to internal loop functions;
  record the chosen shape in `knowledge.md` (the decision leans toward the loop
  driving the commands).
- Ensure the loop reuses the existing gate (`dispatch_gate_for_plan`,
  `dispatch.rs:62`) and preflight (`preflight_live_provider`) checks; it must not
  bypass the gate to run a provider.
- Add a non-goal assertion in code review notes: no `RealBoundaryController`
  method runs a provider without passing through the existing gate/preflight.

Verification:

- Focused `cargo test -p capo-server -p capo-controller` proving a loop turn
  produces the same dispatch plan/gate/execution event sequence as the direct
  command path for a scripted run.
- `cargo fmt`
- `git diff --check`

## RTL5 - Implement RealBoundaryController Behind The Server Boundary

Status: pending.

Acceptance:

- Add `RealBoundaryController` in `crates/capo-controller/src/lib.rs` mirroring
  the `FakeBoundaryController` constructor surface
  (`open`/`open_with_permission_policy`) and the typed return types
  (`FakeRunRefs`-shaped or renamed run refs, read-model observations).
- Implement the same controller methods the server calls
  (`send_task_command`, `redirect_command`, `interrupt_command`, `stop_command`,
  `recover_command`, `register_agent`) using the RTL3 loop and the RTL1 trait.
- Keep the typed `ServerCommand`/`ServerResponse` boundary in
  `crates/capo-server/src/types.rs` unchanged; the controller swap must be
  invisible to clients.
- Persist Capo-owned events/projections through `SqliteStateStore::append_event`
  exactly as the fake path does, so read models are byte-compatible where the
  scripted adapter output is identical.
- Coexist with `FakeBoundaryController` until RTL12 flips the default; do not
  delete the fake path in this task.

Verification:

- Focused `cargo test -p capo-controller -p capo-server` for the real controller
  over a scripted adapter.
- Restart/replay test proving real-controller projections rebuild identically.
- `cargo fmt`

## RTL6 - Safety Floor: Confinement, Hard-Kill, Pre-Write Checkpoint, Dry-Run Default

Status: pending.

Acceptance:

- Enforce workspace path confinement on the write path by wiring the existing
  path-containment engine (the `normalize_policy_path` confinement logic in
  `crates/capo-server/src/live_provider.rs:686-783`, and `capo-tools` path
  containment) so a write outside the confined workspace is rejected before any
  process runs.
- Add a controller-owned hard kill that terminates the run's process group
  (reusing the runtime process-group kill path) and records the abort as an
  event; the kill must work mid-run.
- Add a single-snapshot pre-write checkpoint (git-stash, tar copy, or worktree
  snapshot for phase 1; full shadow-git stays in `safety-gates`) recorded via a
  `checkpoint.created`-style event so any RTL live write is reversible by one
  command.
- Make diff-preview/dry-run the default for the write adapter: a live write
  requires an explicit opt-in env gate AND is not unattended.
- Add a deterministic test proving an out-of-confinement write is rejected and
  no process is spawned.
- Add a deterministic test proving the checkpoint is created before the write and
  a documented restore command returns the workspace to the pre-write state.

Verification:

- Focused `cargo test -p capo-server -p capo-tools` for confinement rejection and
  checkpoint create/restore.
- Restart/replay test proving the checkpoint event survives restart.
- `cargo fmt`
- `git diff --check`

Must not do:

- Do not implement full `PermissionPolicy` enforcement, `VerificationRunner`, or
  full shadow-git here; those belong to `safety-gates`.
- Do not allow an unconfined or irreversible real write to exist between this
  workpad and `safety-gates`.

## RTL7 - Per-Run Resource Ceiling With Controller-Enforced Abort

Status: pending.

Acceptance:

- Add a per-run resource ceiling enforced by the controller: max turns, max
  wall-clock, and a hard token/cost ceiling per run.
- Wire the wall-clock ceiling to the existing timeout path
  (`wait_running_with_timeout` in the live provider execution at
  `live_provider.rs:572-576`) and add max-turn and token/cost accounting in the
  loop.
- Emit a `run.aborted`-style event (add the `EventKind` in
  `crates/capo-state/src/event.rs` if absent, with an idempotency key and
  projection) when any ceiling is exceeded, and stop the run through the RTL6
  hard-kill.
- Make the ceiling a prerequisite for any live-provider task: the live Codex
  path (RTL9) must run inside an active ceiling, never without one.
- Scope this as a strict subset of `goal-autonomy`'s `GoalBudget`; record that
  `goal-autonomy` extends this enforcement floor rather than replacing it.
- Add a deterministic test proving a scripted run that exceeds max-turns aborts
  with a `run.aborted` event and no further turns are projected.

Verification:

- Focused `cargo test -p capo-controller -p capo-state` for ceiling-triggered
  abort and the `run.aborted` projection.
- Restart/replay test proving the aborted run stays aborted after rebuild.
- `cargo fmt`

## RTL8 - Per-Turn Artifact Keying By turn_id Reconciled With Dispatch Run-Exit

Status: pending.

Acceptance:

- Thread `turn_id` through `LocalProcessRequest` and the runtime
  `run_dir`/artifact path in `crates/capo-runtime/src/lib.rs:322-325`, which today
  keys `run_dir = artifact_root/run_id` and reuses a single `stdout.txt`, so
  multiple turns in one run no longer overwrite each other's stdout/stderr
  artifacts.
- Ensure `TurnFinished` events and per-turn artifacts reconcile with the existing
  dispatch run-exit events (`append_dispatch_run_exit` /
  `append_dispatch_run_exit_with_metadata` at
  `crates/capo-server/src/dispatch.rs:485-547`) and execution-status projections
  (`AdapterDispatchExecutionProjection`); there must be no duplicate
  run-completion semantics.
- Define a single authoritative notion of run/turn completion: the loop's
  `TurnFinished` annotates the dispatch execution/run-exit events, it does not
  fork a second completion model.
- Add a multi-turn-per-run test asserting distinct stdout/stderr artifacts per
  `turn_id`.
- Add a replay test proving every turn's artifact is reconstructable after
  rebuild.

Verification:

- Focused `cargo test -p capo-runtime -p capo-server` for per-turn artifact
  keying and run-exit reconciliation.
- Restart/replay test reconstructing every turn artifact.
- `cargo fmt`
- `git diff --check`

## RTL9 - Codex Workspace-Write Adapter With Live Tool-Result Round-Trip

Status: pending.

Acceptance:

- Implement the Codex workspace-write adapter as the sole real
  `AgentAdapter` for phase 1, building on the existing
  `CodexExecAdapter::local_launch_plan` and the live execution path in
  `crates/capo-server/src/live_provider.rs:550-683`.
- Move Codex off `--sandbox read-only --ephemeral` one-shot reads to a
  workspace-write profile that can apply edits inside the confined workspace,
  with a live tool-result round-trip parsed into `NormalizedAdapterEvent`s.
- Gate the live write behind an explicit opt-in env gate mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE`/`live_execution_opt_in`, the RTL6 confinement and
  checkpoint, and the RTL7 ceiling; dry-run/diff-preview remains default.
- Persist an observed tool-result event distinct from any agent-reported claim,
  using the existing tool observation events (`tool.observation_recorded`,
  `tool.output_artifact_recorded`).
- Add a deterministic mock-output test (`mock_provider_output_jsonl` path) for
  the write round-trip so the adapter is fully testable without a live provider.
- Note in acceptance that one real workspace-write provider (Codex) is sufficient
  to declare the loop real; Claude is breadth and deferred to `depth`.

Verification:

- Focused `cargo test -p capo-server -p capo-adapters` for the deterministic mock
  write round-trip.
- Restart/replay test for the ingested write turn.
- `cargo fmt`
- `git diff --check`

Must not do:

- Do not implement the Claude write adapter or live ACP wire here; both are
  `depth`.

## RTL10 - Crash-Safe In-Flight Runs: Persist Before Spawn, Reap Orphans On Restart

Status: pending.

Acceptance:

- Persist `runtime.start_requested` plus the pid/process-group reference before
  the process is spawned (the runtime already emits `start_requested` and
  records `external_pid`/`process_group` at `lib.rs:344-369`); ensure the server
  persists this before `spawn_process` returns for the live path.
- On restart, reap orphaned process groups using the proven descendant reaper
  rather than blindly marking all live-looking runs `exited_unknown` (improving
  on `mark_active_runs_exited_unknown` in
  `crates/capo-controller/src/lib.rs:166`).
- Emit `run.orphaned`/`run.recovered`/`run.exited` consistent with the state
  model's restart-recovery order (`state-model.md` Restart Recovery section),
  keyed by `(run_id, recovery_observation_kind, observed_runtime_state_hash)`.
- Add a deterministic test simulating a restart mid-turn that reaps the orphaned
  process group and leaves the thread read model consistent (no corrupted
  half-open turn).
- Keep full liveness-probe reattach in `safety-gates`; phase 1 reaps and records,
  it does not reattach.

Verification:

- Focused `cargo test -p capo-runtime -p capo-controller` for the orphan-reaping
  restart path.
- Restart/replay test proving recovery events are idempotent across repeated
  restarts.
- `cargo fmt`

## RTL11 - Route Default Chat/Steer Through The Real Loop With Scripted-Mock Fallback

Status: pending.

Acceptance:

- Route the server's `SteerAgent`/`SendTask` handling so it can dispatch to
  either `FakeBoundaryController` or `RealBoundaryController` behind a single
  typed config switch.
- Keep the scripted-mock adapter as an explicit fallback so deterministic tests
  and the parity suite (RTL13) can run the real controller over scripted input.
- Default chat must not silently route to a fake echo once the switch flips;
  before the flip, the fake remains default and the real path is opt-in.
- The switch must be a single typed value (not scattered booleans) with a
  documented rollback in `knowledge.md`.
- Keep the `ServerCommand` surface unchanged; only the controller selection
  changes.

Verification:

- Focused `cargo test -p capo-server` proving both routings handle
  `send`/`steer`/`interrupt`/`stop`.
- `cargo fmt`
- `git diff --check`

## RTL12 - Deterministic Multi-Turn Edit Tests, Restart/Replay, And Parity-Equivalence

Status: pending.

Acceptance:

- Add deterministic fake/scripted multi-turn edit tests over the real loop:
  open session, run two turns that each produce a distinct workspace edit, and
  assert distinct per-turn artifacts and projected items.
- Add a restart/replay test proving the multi-turn thread, per-turn artifacts,
  dispatch executions, and run-exit events rebuild identically.
- Define and implement the parity criterion: `RealBoundaryController` passes the
  identical deterministic suite (`send`/`steer`/`interrupt`/`stop`,
  restart/replay) that `FakeBoundaryController` passes.
- Add a parity-equivalence test asserting that for a scripted turn the fake and
  real paths produce equivalent event sequences (modulo adapter-identity fields).
- Drive the RTL11 single-switch cutover from this suite: the default flips only
  after parity passes, and the flip has a documented rollback (RTL11).

Verification:

- Focused `cargo test -p capo-controller -p capo-server` for the multi-turn and
  parity-equivalence suites.
- Restart/replay test across both controllers.
- `cargo fmt`
- `git diff --check`

## RTL13 - Live Opt-In Codex Workspace-Write Smoke Paired With Deterministic Assertions

Status: pending.

Acceptance:

- Add a live opt-in Codex workspace-write smoke behind an explicit env gate
  (mirroring `CAPO_SERVER_RUN_CODEX_LIVE`), separate from ordinary test runs, that
  performs one real confined edit and ingests the tool-result round-trip.
- Pair the live smoke with deterministic assertions: the same scripted/mock
  fixture must assert the identical normalized-event and artifact shape so
  completion is never solely operator-attested.
- Strip secrets from all smoke evidence: artifacts pass the existing credential
  scan (`scan_artifacts_for_sensitive_markers`) and any
  `unknown`/`contains_sensitive` artifact is quarantined or dropped per the
  artifact privacy contract in `state-model.md`.
- Confirm the RTL6 confinement and checkpoint, RTL7 ceiling, and RTL10
  crash-safety all engage on the live path during the smoke.
- Run the focused E2E gate: scripted multi-turn edit, parity check, and the
  gated live smoke, with review notes on architecture fit and the cutover
  decision.

Verification:

- `cargo fmt`
- Focused `cargo test -p capo-server -p capo-controller`, widening to
  `cargo test` if shared controller/state behavior changes broadly.
- Live Codex smoke behind explicit opt-in, with secrets stripped, paired with the
  deterministic fixture assertion.
- `git diff --check`
