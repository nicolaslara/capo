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

Status: done (gate green). `FakeAdapter` and `ScriptedMockAgent` are
`AgentAdapter` trait impls (landed in RTL1) and the controller now holds the
adapter behind the trait: it stores the thin `AgentAdapterHandle` dispatch enum
(the explicitly-allowed shape over trait impls) and drives it only through the
`AgentAdapter` trait methods (`open_session`/`send_turn`/`attach_session`/
`interrupt`/`stop`). No non-fake call site names a `Fake*` request/output type
(`rg FakeAdapterSessionRequest|FakeAdapterTurnRequest|FakeAdapterTurnOutput|
FakeAdapterSession` -> none). The adapter is now injectable via
`open_with_adapter` / `open_with_permission_policy_and_adapter` so the
scripted-mock fallback can be substituted for the parity suites; `open` /
`open_with_permission_policy` still inject `AgentAdapterHandle::fake()` so the
fake path is byte-for-byte unchanged. `scripted_turn_events` and
`apply_scripted_mock_turn` remain available for deterministic multi-turn
fixtures.

Evidence:

- Controller injection behind the trait:
  `crates/capo-controller/src/lib.rs` adds `open_with_adapter` and
  `open_with_permission_policy_and_adapter(.., adapter: AgentAdapterHandle)`;
  `open`/`open_with_permission_policy` delegate with `AgentAdapterHandle::fake()`
  so existing construction is unchanged.
- New deterministic controller tests in `crates/capo-controller/src/tests.rs`:
  `controller_drives_injected_scripted_mock_adapter_behind_the_trait` (scripted
  mock substituted via the neutral `TurnOutput`: external_session_ref/summary/
  confidence=88/status="completed" all from the injected adapter, and
  interrupt routes through the scripted-mock trait method) and
  `controller_default_open_keeps_fake_adapter_output_byte_for_byte` (fake
  default summary/confidence=82/status="active" unchanged).
- Migration check: `rg 'FakeAdapterSessionRequest|FakeAdapterTurnRequest|
  FakeAdapterTurnOutput|FakeAdapterSession' crates/` -> no matches; the
  controller imports only neutral seam types (`AdapterSessionRequest`,
  `AgentAdapter`, `AgentAdapterHandle`, `TurnRequest`).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-adapters -p capo-controller` -> ok (capo-adapters 27
  passed / 2 ignored; capo-controller 10 passed). Objective gate
  (`cargo fmt --check` && `cargo clippy --all-targets --all-features -- -D
  warnings` && `cargo test --workspace`) -> all green, exit 0: fmt clean,
  clippy clean, every binary ok / 0 failed (capo-adapters 27, capo-controller
  10, capo-server 29, capo-state 31, capo-runtime 12, capo-tools 18, capo-query
  21, capo-voice 19, capo cli 63 + server_transport 11, etc.).
  `git diff --check` -> clean.

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

Status: done (gate green). The turn-loop contract lives in
`crates/capo-controller/src/turn_loop.rs`: `run_turn(refs, turn_id, batch)` is
one pure, synchronous observe -> project -> emit cycle. It observes a
`&[NormalizedAdapterEvent]` batch, projects it through the EXISTING
`apply_normalized_adapter_events_with_turn` path (keyed to `turn_id`, no new
ingestion route), and emits a `TurnFinished` carrying `stop_reason`
(`TurnStopReason::{Completed,Interrupted,Stopped,Failed}`), `summary_refs` (item
event refs), `observed_tool_refs` (deduped tool event refs), and the reused
`AdapterReplayReport`. The lifecycle maps onto existing event kinds only -
terminal `adapter.turn_completed`/`turn_interrupted`/`turn_failed` already
project onto `evidence.recorded`/`session.interrupted`/`run.exited`, and
item/tool events onto `session.summary_updated`/`tool.*` - so NO new `EventKind`
was added (the ceiling/recovery kinds remain RTL7/RTL10). `interrupt_turn` /
`stop_turn` drive the existing `interrupt`/`stop` controller commands and
annotate them with `Interrupted`/`Stopped` outcomes, so the existing commands
map onto the loop without a second completion model. `TurnFinished` is derived
purely from the batch, and projection re-application is idempotent (idempotency
keys), so a restart/replay rebuilds identical read models and re-derives an
identical outcome.

Evidence:

- Contract + types: `crates/capo-controller/src/turn_loop.rs` (`TurnFinished`,
  `TurnStopReason`, `run_turn`, `interrupt_turn`, `stop_turn`, pure
  `finish_turn`); module wired and types re-exported in
  `crates/capo-controller/src/lib.rs`; `AdapterReplayReport` gained `Default`
  for the interrupt/stop outcomes.
- New deterministic tests in `crates/capo-controller/src/tests.rs`:
  `turn_loop_runs_a_scripted_single_turn_observe_project_emit_cycle` (scripted
  single-turn cycle: Completed outcome, `summary_refs=[msg-1,msg-2]`,
  `observed_tool_refs=[tool-1]`, per-turn-keyed `session.summary_updated`/
  `evidence.recorded`/tool projections),
  `turn_loop_interrupt_and_stop_commands_map_onto_finished_outcomes` (interrupt
  -> Interrupted/canceled, stop -> Stopped/completed), and
  `turn_loop_projected_turn_rebuilds_identically_after_restart_replay`
  (reopen state + `rebuild_projections` yields byte-identical
  session/tool_calls/observations/evidence and event count; re-running the loop
  appends 0 events and re-derives the same `TurnFinished`).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-controller` -> ok, 15 passed / 0 failed (the 3 new RTL3
  tests included). Objective gate: `cargo fmt --check` -> clean;
  `cargo clippy --all-targets --all-features -- -D warnings` -> clean;
  `cargo test --workspace` -> 0 failed across all binaries (capo-controller 15,
  capo-adapters/capo-server 29, capo-state 31, capo-runtime 12, capo-tools 18,
  capo-query 21, capo cli 63 + server_transport 11, etc.; 259 passed total).

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

Status: done (gate green). The reconciliation point is
`CapoServer::run_dispatch_turn` (`crates/capo-server/src/turn_orchestration.rs`):
the loop's emit step DRIVES the existing dispatch primitives as its single
execution substrate. It invokes the typed dispatch `ServerCommand`s through
`CapoServer::handle` (deterministic: `PlanDispatch` -> `GateDispatch` ->
`RunDispatchLocal`; live: `PreflightLiveProvider` -> `RunLiveProviderLocal`) and
then ANNOTATES the run it drove with a `TurnFinished` derived from the SAME
normalized batch the dispatch run ingested, via the new public
`FakeBoundaryController::derive_turn_finished` (the outcome classifier `run_turn`
already uses, so loop and dispatch agree by construction -- one completion
model, no parallel pipeline, no new `EventKind`). The live Codex execution that
flows through `run_live_provider_local` is now a step inside the loop (live arm),
not a separately invoked command sequence. The chosen call shape (loop invokes
commands) and the non-goal assertion (no path runs a provider without the
existing gate/preflight) are recorded in `knowledge.md`.

Evidence:

- Reconciliation orchestrator + types:
  `crates/capo-server/src/turn_orchestration.rs` (`run_dispatch_turn`,
  `DispatchTurnRequest`/`DispatchTurnMode`/`DispatchTurnOutcome`,
  `turn_finished_for_run`, `dispatch_plan_id_for_turn`); module wired and types
  re-exported in `crates/capo-server/src/lib.rs`.
- Shared outcome classifier: `crates/capo-controller/src/turn_loop.rs` exposes
  `FakeBoundaryController::derive_turn_finished` (was the private free `finish_turn`),
  now called by both `run_turn` and the server's dispatch path.
- Decision + non-goal assertion recorded in
  `workpads/real-turn-loop/knowledge.md` ("The Loop Drives Dispatch Rather Than
  Running Beside It" and "Open Questions" RESOLVED).
- New deterministic tests:
  `crates/capo-server/src/tests/turn_orchestration.rs` --
  `loop_turn_drives_the_same_dispatch_sequence_as_the_direct_command_path`
  (the loop-driven turn produces the IDENTICAL dispatch plan/gate/
  prompt-materialization/execution-request/executed/run.exited/replayed event
  sequence as the direct `PlanDispatch`/`GateDispatch`/`RunDispatchLocal`
  command path, with matching run status/counts/provider flags),
  `loop_turn_does_not_run_provider_without_passing_the_gate` (a gate-blocked
  turn passes through the gate, ingests no batch, runs no provider, and emits a
  no-ref `TurnFinished`), and
  `loop_turn_drives_the_live_substrate_through_preflight_and_run` (the live arm
  goes through `PreflightLiveProvider` and ingests the mock provider output).
  `crates/capo-controller/src/tests.rs` --
  `turn_loop_dispatch_derivation_matches_run_turn_for_the_same_batch`
  (the dispatch-path derivation equals the in-loop `run_turn` outcome).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-controller -p capo-server` -> ok (capo-controller 17,
  capo-server 32, 0 failed). Objective gate: `cargo fmt --check` -> clean;
  `cargo clippy --all-targets --all-features -- -D warnings` -> clean (exit 0);
  `cargo test --workspace` -> 0 failed across all binaries (capo-server 32,
  capo-controller 17, capo-adapters 27, capo-state 31, capo-runtime 12,
  capo-tools 18, capo-query 21, capo-voice 19, capo cli 63 + server_transport
  11, etc.). `git diff --check` -> clean (exit 0).

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

Status: implemented. `RealBoundaryController`
(`crates/capo-controller/src/real_controller.rs`) is the production consumer of
the RTL3 loop and the RTL1 trait. It mirrors the `FakeBoundaryController`
constructor surface and the server-called methods, persists through the same
`append_event`/projection path (read models byte-compatible with the fake path
for identical scripted output), and coexists with the fake handle. Verified by
the parity + restart/replay tests in `crates/capo-controller/src/tests.rs`
(`real_controller_*`) and the server-crate parity test in
`crates/capo-server/src/tests/turn_orchestration.rs`
(`real_controller_matches_fake_path_over_a_scripted_adapter_from_the_server_crate`).

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

Status: done (gate green; symlinked-temp-prefix confinement regression repaired -
see Evidence). The RTL safety floor lives in
`crates/capo-server/src/safety_floor.rs` and wires the four floor requirements
onto the write path. Confinement reuses the existing path-containment engine:
the new public `capo_tools::confine_write_path`
(`crates/capo-tools/src/runtime_wrapper_paths.rs`) wraps the same
`ensure_under_workspace` + nearest-existing-ancestor logic the runtime tool
wrappers use, so a write that escapes the confined workspace (via `..`, an
unrelated absolute path, or a symlinked prefix) is rejected BEFORE any process
is spawned (`CapoServer::confine_workspace_write` /
`run_workspace_write_turn`). Dry-run/diff-preview is the DEFAULT
(`WriteMode::DryRun`); a live write requires the caller opt-in AND the
`CAPO_SERVER_RUN_CODEX_LIVE` env gate AND an attended run (`resolve_write_mode`),
and is the only branch that touches the workspace and therefore the only branch
that takes a checkpoint. The single-snapshot pre-write checkpoint
(`WorkspaceCheckpoint`, a directory-copy snapshot under the artifact root;
full shadow-git stays in `safety-gates`) is recorded via a new
`checkpoint.created` `EventKind` and is reversible by one documented command
(`restore_command()` / `restore()`). The controller-owned hard kill
(`CapoServer::hard_kill_run`) terminates the run's process group mid-run via the
new runtime `LocalProcessRunner::kill_running_process_group` (reusing the
existing `SIGTERM`/`SIGKILL` process-group teardown) and records the abort as a
new `run.hard_killed` `EventKind` (distinct from RTL7 `run.aborted` and RTL10
`run.orphaned`/`run.recovered`). The Codex workspace-write profile
(`CodexExecAdapter::local_workspace_write_launch_plan`) moves off
`--sandbox read-only --ephemeral` to `--sandbox workspace-write` (no
`--ephemeral`) while staying subscription-safe and confined via `--cd`.

Evidence:

- Floor module + types: `crates/capo-server/src/safety_floor.rs`
  (`WriteMode`/`resolve_write_mode`, `WorkspaceCheckpoint`,
  `WorkspaceWriteRequest`/`WorkspaceWriteOutcome`, `RunTurnRef`,
  `confine_workspace_write`, `create_pre_write_checkpoint`, `hard_kill_run`,
  `run_workspace_write_turn`); module wired and types re-exported in
  `crates/capo-server/src/lib.rs`; `capo-tools` added as a dependency in
  `crates/capo-server/Cargo.toml`.
- Reused containment engine made public: `confine_write_path` in
  `crates/capo-tools/src/runtime_wrapper_paths.rs` (exported in
  `crates/capo-tools/src/lib.rs`).
- New runtime process-group hard-kill: `kill_running_process_group` in
  `crates/capo-runtime/src/lib.rs` (reuses `terminate_process_group`).
- New event kinds: `EventKind::CheckpointCreated` (`checkpoint.created`) and
  `EventKind::RunHardKilled` (`run.hard_killed`) in
  `crates/capo-state/src/event.rs`; both append through
  `SqliteStateStore::append_event` with idempotency keys so they survive
  restart/replay.
- Codex workspace-write profile:
  `CodexExecAdapter::local_workspace_write_launch_plan` in
  `crates/capo-adapters/src/local_subscription.rs`.
- Deterministic tests (no live provider):
  `crates/capo-server/src/tests/safety_floor.rs` --
  `out_of_confinement_write_is_rejected_before_any_process_runs` (an out-of-
  workspace `..`-escape and an unrelated absolute path are rejected, no
  checkpoint snapshot dir or `checkpoint.created` event is produced, and a
  confined target is accepted),
  `write_adapter_defaults_to_dry_run_and_takes_no_checkpoint` (dry-run default;
  unattended never reaches a live write),
  `pre_write_checkpoint_is_created_and_one_command_restores_the_workspace`
  (checkpoint is taken before the write; after the write mutates/adds/deletes
  files the recorded one-command restore returns the workspace to its pre-write
  state),
  `create_pre_write_checkpoint_is_idempotent_on_unchanged_state`,
  `checkpoint_event_survives_restart_and_replay` (reopen + `rebuild_projections`
  preserves the `checkpoint.created` event and event count), and
  `controller_hard_kill_terminates_the_process_group_mid_run_and_records_the_abort`
  (a live child with a backgrounded descendant is hard-killed mid-run, the
  descendant marker never appears, and a `run.hard_killed` event is recorded).
  `crates/capo-tools/src/tests.rs` --
  `confine_write_path_accepts_targets_under_the_workspace_and_rejects_escapes`
  and `confine_write_path_rejects_symlinked_prefix_escaping_the_workspace`.
  `crates/capo-adapters/src/tests.rs` --
  `codex_workspace_write_launch_plan_uses_workspace_write_sandbox_without_ephemeral`.
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-server safety_floor` -> ok, 6 passed;
  `cargo test -p capo-tools confine` -> ok, 2 passed. Objective gate:
  `cargo fmt --check` -> clean; `cargo clippy --all-targets --all-features -- -D
  warnings` -> clean (exit 0); `cargo test --workspace` -> 276 passed, 0 failed
  across all binaries (capo-server 39, capo-adapters 28, capo-tools 20,
  capo-state 31, capo-runtime 19, capo cli 63 + server_transport 11, etc.).
  `git diff --check` -> clean (exit 0). No live Codex smoke is required for RTL6
  (the live workspace-write smoke is RTL13); all proofs are deterministic.

- Gate repair (symlinked temp-prefix confinement): the objective gate failed in
  three `safety_floor` tests
  (`pre_write_checkpoint_is_created_and_one_command_restores_the_workspace`,
  `create_pre_write_checkpoint_is_idempotent_on_unchanged_state`,
  `checkpoint_event_survives_restart_and_replay`) whenever the system temp dir
  resolves through a symlink (on macOS `/tmp` -> `/private/tmp`, e.g. under
  `TMPDIR=/tmp`). Root cause was in `capo_tools::confine_write_path`
  (`crates/capo-tools/src/runtime_wrapper_paths.rs`): it canonicalized the
  workspace root (`/private/tmp/...`) but then ran the lexical
  `ensure_under_workspace` check against the UN-resolved candidate (`/tmp/...`),
  which is not lexically "under" the canonical root, so a legitimate confined
  target (including `target == workspace_root`, as the pre-write checkpoint
  passes it) was rejected before any process ran. Fix: when the normalized
  candidate exists, canonicalize it FIRST and confine the symlink-resolved form
  (the lexical pre-check is skipped for existing paths precisely because it
  compares an unresolved candidate against a resolved root); for not-yet-created
  targets, re-anchor the tail onto the canonical nearest-existing-ancestor and
  confine that symlink-resolved candidate. `..`-escapes and symlinked-prefix
  escapes still reject (the ancestor confinement and credential-component rules
  are unchanged), and returned paths remain `..`-free and symlink-resolved. New
  deterministic regression that does not depend on the ambient temp dir
  (it builds its own symlink standing in for `/tmp`):
  `crates/capo-tools/src/tests.rs` ::
  `confine_write_path_accepts_a_target_reached_through_a_symlinked_workspace_prefix`.
  Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `TMPDIR=/tmp cargo test -p capo-server safety_floor` -> ok, 6 passed (was 3
  failed before the fix); `cargo test -p capo-tools confine` -> ok, 3 passed.
  Full objective gate run twice -- with `TMPDIR=/tmp` (the failing scenario) and
  with the default temp dir: `cargo fmt --check` clean; `cargo clippy
  --all-targets --all-features -- -D warnings` clean (exit 0); `cargo test
  --workspace` exit 0, 0 failed across all binaries (capo-tools 21, capo-server
  39, capo-adapters 28, capo-state 31, capo-runtime 19, capo cli 63 +
  server_transport 11, etc.). Files changed:
  `crates/capo-tools/src/runtime_wrapper_paths.rs`,
  `crates/capo-tools/src/tests.rs`.

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

Status: done (gate green). The per-run resource ceiling lives in
`crates/capo-controller/src/resource_ceiling.rs`: `RunResourceCeiling`
(`max_turns`/`max_wall_clock`/`max_token_cost`) plus `RunResourceUsage`
accounting. `RunResourceCeiling::breach(usage)` is the single pure classifier the
loop and the live arm both consult, returning the FIRST breach in a fixed
priority order (turns -> wall-clock -> token/cost) so the abort reason is
deterministic. The controller enforces the ceiling IN THE LOOP ON EVERY PATH -- the deterministic
loop and the live server arm both consult the single `RunResourceCeiling::breach`
classifier:
`FakeBoundaryController::run_turn_within_ceiling` accounts the turn about to run
(one more turn + its token cost) BEFORE projecting, and if that trips the ceiling
it aborts via `abort_run_for_ceiling` and returns `CeilingTurnOutcome::Aborted`
WITHOUT projecting the turn. The live-provider arm of
`CapoServer::run_dispatch_turn` (`crates/capo-server/src/turn_orchestration.rs`)
carries a per-run `RunResourceUsage` accumulator (`usage_before` +
`turn_token_cost`) alongside the `RunResourceCeiling`: it accounts the turn about
to run and aborts BEFORE spawning the provider if the turns or token/cost ceiling
trips, returns `usage_after` so the loop carries usage across turns, and on a
wall-clock timeout (`run.status == "timed_out"`, after the runtime's
process-group hard-kill in `wait_running_with_timeout`) pairs the kill with a
`run.aborted` event via `abort_run_for_ceiling`. All three dimensions are
therefore enforced on the same substrate the live Codex provider runs through.
Exceeding any ceiling appends a durable
`run.aborted` event (new `EventKind::RunAborted` -> `run.aborted` in
`crates/capo-state/src/event.rs`, idempotent on `(project, run_id, breach.code)`)
and writes the SAME COORDINATED terminal projection set every other terminal stop
writes (run + session `aborted`, agent freed to `available`/no session, task
`aborted`), so a ceiling abort leaves the read model in the same shape as
interrupt/stop and the run rebuilds identically on replay. Token/cost on the
live path is a pre-turn BOUND (a live provider's real cost is only known after
the turn): `DispatchRunSummary` carries an `observed_token_cost` the loop folds
into usage when present (`None` until RTL9 wires the real Codex token
round-trip), so the hard token ceiling fires on the next turn boundary once
observed cost is available. The live-provider turn REJECTS a ceiling that does
not bound wall-clock -- so the live Codex path always runs inside an active
ceiling, never without one. The
ceiling is documented as a strict SUBSET of `goal-autonomy`'s `GoalBudget` (which
extends this enforcement floor rather than replacing it), per `knowledge.md`'s
"The RTL Safety Floor" section.

Evidence:

- Ceiling module + types: `crates/capo-controller/src/resource_ceiling.rs`
  (`RunResourceCeiling`/`RunResourceUsage`/`CeilingBreach`/`CeilingTurnOutcome`,
  `breach`, `run_turn_within_ceiling`, `abort_run_for_ceiling`); module wired and
  types re-exported in `crates/capo-controller/src/lib.rs`.
- New event kind: `EventKind::RunAborted` (`run.aborted`) in
  `crates/capo-state/src/event.rs`; it appends through
  `SqliteStateStore::append_event` with an idempotency key and a `Run` projection
  so it survives restart/replay (no exhaustive `EventKind` match elsewhere needed
  updating -- all consumers match on `kind.as_str()`).
- Whole-path ceiling enforcement + wall-clock-to-timeout wiring + active-ceiling
  prerequisite: `crates/capo-server/src/turn_orchestration.rs`
  (`DispatchTurnMode::LiveProvider` carries a boxed `LiveProviderTurn` with
  `ceiling: RunResourceCeiling`, `usage_before: RunResourceUsage`, and
  `turn_token_cost`; `run_dispatch_turn` accounts the turn, breach-checks
  turns/token BEFORE preflight, derives `timeout_seconds` from the ceiling,
  rejects a live turn with no wall-clock bound, and on a `timed_out` run routes
  to `abort_live_turn_for_ceiling` -> `FakeBoundaryController::abort_run_for_ceiling`;
  returns `DispatchTurnOutcome { usage_after, ceiling_breach, .. }`).
  `crates/capo-server/src/types.rs` adds `DispatchRunSummary.observed_token_cost`
  (codec round-trip updated in `crates/capo-server/src/transport/codec.rs`).
- Coordinated terminal projection set on abort:
  `crates/capo-controller/src/resource_ceiling.rs`
  (`abort_run_for_ceiling` now writes Task + Agent + Session + Run projections,
  matching interrupt/stop -- agent freed, session/run `aborted`).
- Deterministic tests (no live provider):
  `crates/capo-controller/src/tests.rs` --
  `resource_ceiling_classifies_the_first_breach_in_priority_order` (the pure
  classifier: within bounds = no breach; turns win priority; then wall-clock; then
  token/cost; unbounded ceiling has no wall-clock timeout),
  `run_that_exceeds_max_turns_aborts_with_run_aborted_event_and_projects_no_further_turn`
  (RTL7 acceptance: turn 1 within `max_turns=1` projects and completes; turn 2
  aborts BEFORE projecting -- a `run.aborted` event keyed to the aborting turn is
  recorded, the run projection is `aborted`, none of turn 2's batch reaches the
  read models, and exactly one event is appended for the over-ceiling turn),
  `wall_clock_and_token_cost_breaches_abort_with_their_reason_code_and_terminal_projections`
  (the two dimensions max_turns does not cover: WallClock and TokenCost breaches
  each record a `run.aborted` with the right reason code AND the coordinated
  terminal projection set -- run/session `aborted`, agent freed), and
  `aborted_run_stays_aborted_after_restart_replay_and_abort_is_idempotent`
  (restart/replay: reopen + `rebuild_projections` leaves the run `aborted` with no
  new events; re-recording the same breach is idempotent).
  `crates/capo-server/src/tests/turn_orchestration.rs` --
  `live_turn_without_a_wall_clock_bound_is_rejected_before_any_provider_runs`
  (the negative active-ceiling prerequisite: an unbounded and a max-turns-only
  ceiling are both rejected before any provider runs or aborts),
  `live_turn_over_max_turns_aborts_on_the_loop_path_without_running_the_provider`
  (the turns ceiling is enforced in `run_dispatch_turn`: the over-ceiling turn
  aborts before the provider spawns, emits `run.aborted`, projects nothing, and
  frees the agent / marks the session aborted -- no `run.exited`), and
  `live_turn_over_token_cost_aborts_on_the_loop_path_without_running_the_provider`
  (the token/cost ceiling is enforced from the pre-turn estimate on the live
  path), plus a `usage_after`/`ceiling_breach` assertion on the existing
  `loop_turn_drives_the_live_substrate_through_preflight_and_run`.
  `crates/capo-state/src/tests.rs` --
  `run_aborted_event_projects_aborted_status_and_rebuilds_identically` (the
  `run.aborted` event projects an `aborted` Run, an aborted run is not
  active-looking, the abort is idempotent, and the status rebuilds identically
  from the event log).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-controller -p capo-state` -> ok (capo-controller 22 passed,
  capo-state 32 passed; the 3 new RTL7 controller tests + 1 state test included).
  Objective gate: `cargo fmt --check` -> clean; `cargo clippy --all-targets
  --all-features -- -D warnings` -> clean (exit 0); `cargo test --workspace` ->
  281 passed, 0 failed across all binaries (capo-controller 22, capo-state 32,
  capo-server 39, capo-adapters 28, capo-runtime 12, capo-tools 21, capo-query 21,
  capo-voice 19, capo cli 63 + server_transport 11, etc.). `git diff --check` ->
  clean (exit 0). No live Codex smoke is required for RTL7 (the live
  workspace-write smoke is RTL13); all proofs are deterministic.

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

Status: done (gate green; codex-program-override drift repaired - see Evidence).
`turn_id` is now threaded through
`LocalProcessRequest` (a new `Option<String>` field, plus `new()`/`with_turn_id`
helpers) and the runtime `run_dir`/artifact path in
`crates/capo-runtime/src/lib.rs`: when a turn key is present the runtime keys the
artifact directory and artifact ids per `(run_id, turn_id)`
(`run_dir = artifact_root/run_id/turns/<turn_id>`, id
`artifact-runtime-{run_id}-turn-{turn_id}-{stream}`), so multiple turns in one
run no longer overwrite each other's `stdout.txt`/`stderr.txt`; with no turn key
the legacy single-turn layout (`artifact_root/run_id/stdout.txt`, id
`artifact-runtime-{run_id}-{stream}`) is preserved byte-for-byte for callers with
no turn (tool wrappers, single-turn dispatch runs). The Codex workspace-write
launch plan gained `runtime_request_for_turn`
(`crates/capo-adapters/src/local_subscription.rs`), and the live provider
(`execute_codex_live_provider`, `crates/capo-server/src/live_provider.rs`) now
spawns with the per-turn request keyed to `context.turn_id`, so the recorded
`stdout_artifact_id`/`stderr_artifact_id` carry the turn key. There is no
duplicate run-completion model: `run_dispatch_turn` derives the dispatch plan id
per turn (`dispatch_plan_id_for_turn` folds in `turn_id`), so each turn keys its
own dispatch execution + `RunExited` (`append_dispatch_run_exit_with_metadata`)
event, and the loop's `TurnFinished` ANNOTATES that single run-exit truth -- it
does not fork a second completion event kind.

Evidence:

- Runtime threading: `crates/capo-runtime/src/lib.rs` adds
  `LocalProcessRequest.turn_id: Option<String>` (+ `new`/`with_turn_id`),
  `LocalRunningProcess.turn_id`, `run_dir_for`, `artifact_id_for`, and
  `sanitize_artifact_key`; `spawn_process`/`wait_running`/`start_process` key the
  per-turn dir and artifact id off the optional turn.
- Launch-plan per-turn request: `runtime_request_for_turn` in
  `crates/capo-adapters/src/local_subscription.rs`; live provider spawns with it
  in `crates/capo-server/src/live_provider.rs`.
- Callers with no turn updated to `turn_id: None` (legacy layout preserved):
  `crates/capo-tools/src/runtime_wrappers.rs`,
  `crates/capo-server/src/tests/safety_floor.rs`, and the runtime in-file tests.
- New deterministic tests (no live provider):
  `crates/capo-runtime/src/lib.rs` --
  `multiple_turns_in_one_run_keep_distinct_per_turn_artifacts` (two turns in one
  run produce distinct stdout/stderr paths + ids + content, nested under
  `run_id/turns/<turn_id>`, and every turn is reconstructable by enumerating the
  run directory after the processes exit) and
  `run_without_a_turn_id_keeps_the_legacy_single_turn_artifact_layout` (no turn
  key -> legacy `run_id/stdout.txt` path and `artifact-runtime-{run_id}-{stream}`
  id, no `turns/` dir).
  `crates/capo-server/src/tests/per_turn_artifacts.rs` (new module, wired in
  `tests.rs`) --
  `workspace_write_turns_in_one_run_keep_distinct_per_turn_artifacts` (the REAL
  Codex workspace-write launch plan, with codex stubbed by `/bin/sh` emitting the
  fixture JSONL, run twice in one run via `runtime_request_for_turn`: distinct
  per-turn artifact paths/ids, each turn's batch reconstructable from disk, and
  the run dir enumerates both turns) and
  `turn_finished_annotates_dispatch_run_exit_without_a_second_completion_model`
  (the loop's `TurnFinished` annotates exactly one dispatch `run.exited` + one
  `adapter.dispatch_executed` per ingested turn, zero forked turn/run-completion
  event kinds, and the reconciliation survives restart + `rebuild_projections`).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-runtime -p capo-server` -> ok (capo-runtime 14 passed / 0
  failed incl. the 2 new RTL8 runtime tests; capo-server 44 passed / 0 failed
  incl. the 2 new RTL8 server tests). Objective gate: `cargo fmt --check` ->
  clean; `cargo clippy --all-targets --all-features -- -D warnings` -> clean
  (exit 0); `cargo test --workspace` -> exit 0, 0 failed across all binaries
  (capo-runtime 14, capo-server 44, capo-cli 63 + server_transport 11,
  capo-adapters 28 / 2 ignored, capo-controller 23, capo-state 32, capo-tools 21,
  capo-query 21, capo-voice 19, etc.). `git diff --check` -> clean. No live Codex
  smoke is required for RTL8 (the live workspace-write smoke is RTL13); all
  proofs are deterministic.

- Gate repair (codex-program-override drift): the objective gate failed on two
  fronts after the `LiveProviderTurn.codex_program_override` field landed. (1)
  fmt: `crates/capo-server/src/lib.rs:753` materialized the field with a
  multi-line `codex_program_override: codex_program_override.as_deref().map(...)`
  expression that rustfmt collapses to a single line. (2) clippy/test: the RTL8
  `live_mock_turn` helper in `crates/capo-server/src/tests/per_turn_artifacts.rs`
  built `LiveProviderTurn` without the new `codex_program_override` field, a
  missing-field compile error that blocked both clippy and the test run. Fix:
  collapsed the lib.rs expression to one line and added
  `codex_program_override: None` to the mock-turn initializer (the mock path
  never spawns codex, so no override is needed). Files changed:
  `crates/capo-server/src/lib.rs`,
  `crates/capo-server/src/tests/per_turn_artifacts.rs`. Commands run from
  `/Users/nicolas/devel/capo-wt/real-turn-loop`: `cargo fmt --check` -> clean;
  `cargo clippy --all-targets --all-features -- -D warnings` -> clean (exit 0);
  `cargo test --workspace` -> exit 0, 0 failed across all binaries (capo-server
  44 incl. both RTL8 per-turn-artifact tests, capo-runtime 14, capo-cli 63 +
  server_transport 11, capo-adapters 28 / 2 ignored, capo-controller 23,
  capo-state 32, capo-tools 21, capo-query 21, capo-voice 19, etc.).

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

Status: done (gate green). The live execution path now selects the Codex profile
from a resolved [`WriteMode`]: the spawn arm in
`CapoServer::run_live_provider_local`
(`crates/capo-server/src/live_provider.rs`) builds
`CodexExecAdapter::local_workspace_write_launch_plan` (the RTL6 workspace-write
profile, already off `--sandbox read-only --ephemeral`) for a `LiveWrite` and the
read-only `local_launch_plan` for the `DryRun` default. The write mode is
resolved in the `RunLiveProviderLocal` handler through the RTL6
`resolve_write_mode(live_execution_opt_in, unattended)` gate, so a live write
requires the caller opt-in AND the `CAPO_SERVER_RUN_CODEX_LIVE` env AND an
attended run; anything short stays read-only/dry-run. Before a live write spawns,
the path engages the RTL6 confinement (`confine_workspace_write`) and captures the
single pre-write checkpoint (`create_pre_write_checkpoint` ->
`checkpoint.created`), so the first real edit is confined and reversible by one
command; the live arm of `run_dispatch_turn` (RTL7) already runs inside an active
resource ceiling. The Codex parser
(`crates/capo-adapters/src/provider_parsers.rs`) now recognizes the
`patch_apply.begin`/`patch_apply.end` workspace-write family (named `apply_patch`)
and captures the OBSERVED tool result (the applied diff/output:
`unified_diff`/`output`/`aggregated_output`/`formatted_output`/`changes`) into the
event content, so the existing projection records a `tool.observation_recorded`
(with a content-anchored `artifact_id`) distinct from the agent's reported
`item.completed` message claim (`session.summary_updated`). The new
`mock_provider_output_jsonl` write round-trip makes the adapter fully testable
without a live provider. One real workspace-write provider (Codex) is sufficient
to declare the loop real; the Claude write adapter is breadth, deferred to
`depth` (the deterministic mock path proves the round-trip; the live opt-in smoke
is RTL13).

Evidence:

- Codex workspace-write parse + observed tool-result capture:
  `crates/capo-adapters/src/provider_parsers.rs`
  (`patch_apply.begin`/`patch_apply.end` -> `apply_patch`;
  `codex_tool_result_content` reduces the applied-changes shapes to one observed
  string in `event.content`). New write fixture
  `crates/capo-adapters/fixtures/codex-exec-workspace-write.jsonl` (thread
  started -> agent message claim -> apply_patch begin/end with the applied diff ->
  turn completed).
- Profile selection + confinement + checkpoint on the live arm:
  `crates/capo-server/src/live_provider.rs` (`LiveProviderLocalRunRequest` gains
  `write_mode: WriteMode`; the spawn arm picks the workspace-write vs read-only
  launch plan, creates the confined workspace, and takes the pre-write checkpoint
  before the spawn for a `LiveWrite`). Write-mode resolution in the handler:
  `crates/capo-server/src/lib.rs` (`resolve_write_mode(live_execution_opt_in,
  unattended)`). Typed boundary: `RunLiveProviderLocal` gains an `unattended`
  flag (`crates/capo-server/src/types.rs`, codec round-trip in
  `crates/capo-server/src/transport/codec.rs` with a safe `unattended == true`
  default), threaded through `LiveProviderTurn`
  (`crates/capo-server/src/turn_orchestration.rs`) and the CLI
  (`--attended` opt-in in `crates/capo-cli/src/server_client/dispatch.rs`;
  operator-control planner stays read-only in
  `crates/capo-cli/src/operator_control.rs`).
- New deterministic tests (no live provider):
  `crates/capo-adapters/src/tests.rs` ::
  `codex_workspace_write_fixture_maps_a_tool_result_round_trip` (the observed
  apply_patch result carries the applied diff/output, distinct from the agent
  message claim).
  `crates/capo-server/src/tests/codex_workspace_write.rs` (new module, wired in
  `tests.rs`): `workspace_write_mock_round_trip_records_observed_tool_result_distinct_from_agent_claim`
  (the RTL9 acceptance mock round-trip: an `apply_patch`
  `tool.observation_recorded` with a content-anchored artifact, separate from the
  agent's `session.summary_updated` claim),
  `ingested_write_turn_rebuilds_identically_after_restart_replay`
  (reopen + `rebuild_projections` -> byte-identical observations/tool calls/
  session/event count),
  `live_write_uses_workspace_write_profile_and_checkpoints_before_spawn` (a
  `WriteMode::LiveWrite` run driven through a deterministic `/bin/sh` codex stub
  via `codex_program_override`: the provider executes, a `checkpoint.created`
  event is recorded before the confined write lands in the workspace, and the
  observed apply_patch result is recorded), and
  `default_run_stays_read_only_and_takes_no_checkpoint` (the `DryRun` default
  spawns the read-only profile and takes NO checkpoint even with the caller
  opt-in).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-server codex_workspace_write` -> ok, 4 passed;
  `cargo test -p capo-adapters codex` -> ok, 6 passed / 1 ignored (the live smoke
  stays gated). Objective gate: `cargo fmt --check` -> clean; `cargo clippy
  --all-targets --all-features -- -D warnings` -> clean (exit 0); `cargo test
  --workspace` -> exit 0, 0 failed across all binaries (294 passed total;
  capo-server 48 incl. the 4 RTL9 tests, capo-adapters 29 incl. the RTL9 parse
  test, capo-cli 63 + server_transport 11, capo-controller 23, capo-state 32,
  capo-runtime 14, capo-tools 21, capo-query 21, capo-voice 19, etc.).
  `git diff --check` -> clean. No live Codex smoke is required for RTL9 (the live
  workspace-write smoke is RTL13); all proofs are deterministic.

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

Status: done (gate green). The live path now persists the in-flight marker
(start-requested + the pid/process-group reference) the instant the spawn
returns and BEFORE the run is waited on: `CapoServer::execute_codex_live_provider`
(`crates/capo-server/src/live_provider.rs`) calls the new
`CapoServer::append_run_started_inflight` (`crates/capo-server/src/dispatch.rs`)
right after `spawn_process`, recording a `run.started` event carrying
`external_pid`/`runtime_process_ref`/`marker:start_requested_inflight` (keyed per
`(run, pid)`), so a crash mid-run leaves a durable handle in the event log.
On restart the new `CapoServer::reap_orphaned_runs_on_restart`
(`crates/capo-server/src/safety_floor.rs`) replaces the blunt
`mark_active_runs_exited_unknown`: it loads in-flight runs with their persisted
pid (`SqliteStateStore::inflight_runs_for_project`), probes the process GROUP by
that pid (`kill -0 -<pid>`, so a backgrounded descendant whose leader already
exited still reads alive), and if alive reaps the whole group with the proven
`SIGTERM`/`SIGKILL` teardown via the new pid-only runtime reaper
`LocalProcessRunner::reap_orphan_process_group`
(`crates/capo-runtime/src/lib.rs`, reusing `terminate_process_group`). It then
records the outcome through `SqliteStateStore::reap_orphaned_runs`
(`crates/capo-state/src/lib.rs`), emitting `run.orphaned` (alive orphan) ->
`run.exited` (terminal, unknown exit -- phase 1 reaps and records, it does NOT
reattach) -> `run.recovered` consistent with the `state-model.md` Restart
Recovery order. New `EventKind::RunOrphaned`/`RunRecovered`
(`crates/capo-state/src/event.rs`) append through `append_event` with idempotency
keys of `(run_id, recovery_observation_kind, observed_runtime_state_hash)`
(intentionally excluding `recovery_attempt_id`), so repeated restarts that
observe the same runtime state append nothing. Full liveness-probe reattach
stays in `safety-gates`.

Evidence:

- Persist-before-wait + recovery wiring:
  `crates/capo-server/src/live_provider.rs` (in-flight marker persisted right
  after `spawn_process`, before `wait_running_with_timeout`),
  `crates/capo-server/src/dispatch.rs` (`append_run_started_inflight`),
  `crates/capo-server/src/safety_floor.rs` (`reap_orphaned_runs_on_restart`:
  loads in-flight runs, probes/reaps via the runtime, records the outcome).
- Pid-only runtime reaper: `LocalProcessRunner::reap_orphan_process_group` +
  `process_group_is_alive`/`kill_process_group`/`orphan_state_hash` +
  `OrphanReap` in `crates/capo-runtime/src/lib.rs` (reuses
  `terminate_process_group`; non-Unix records `already_gone`).
- State recovery primitive + query + types: `SqliteStateStore::reap_orphaned_runs`
  / `append_recovery_event` (`crates/capo-state/src/lib.rs`),
  `SqliteStateStore::inflight_runs_for_project` + `parse_inflight_marker`
  (`crates/capo-state/src/queries.rs`), `InFlightRun`/`RunReapObservation`/
  `RunReapKind` (`crates/capo-state/src/projections.rs`),
  `EventKind::RunOrphaned`/`RunRecovered` (`crates/capo-state/src/event.rs`).
- New deterministic tests (no live provider):
  `crates/capo-runtime/src/lib.rs` --
  `reap_orphan_process_group_kills_a_live_descendant_tree_by_pid` (a real
  backgrounded descendant whose parent already exited is reaped by the persisted
  group pid before it writes its delayed marker) and
  `reap_orphan_process_group_reports_already_gone_for_a_dead_pid` (a gone pid
  reports `already_gone` with a stable observed-state hash).
  `crates/capo-state/src/tests.rs` --
  `inflight_runs_carry_the_persisted_pid_marker`,
  `reap_orphaned_runs_records_orphan_and_exit_and_is_idempotent_across_restarts`
  (alive orphan -> `run.orphaned`/`run.exited`/`run.recovered`; run no longer
  active-looking; a repeated restart with the same state appends nothing; the
  recovered run rebuilds identically from the log), and
  `reap_orphaned_runs_records_exit_for_an_already_gone_run_without_orphan_event`.
  `crates/capo-server/src/tests/crash_recovery.rs` (new module, wired in
  `tests.rs`) --
  `restart_mid_turn_reaps_the_orphaned_process_group_and_leaves_a_consistent_read_model`
  (a real in-flight process group + persisted pid stands in for a crash mid-turn:
  `reap_orphaned_runs_on_restart` kills the orphaned descendant before its
  marker, leaves the run terminal `recovered` -- no half-open `running` -- with
  `run.orphaned`/`run.exited`/`run.recovered` recorded, a second restart appends
  nothing, and replay rebuilds the recovered run identically).
- Commands run from `/Users/nicolas/devel/capo-wt/real-turn-loop`:
  `cargo test -p capo-runtime reap_orphan` -> ok, 2 passed (run 5x, 0 failures);
  `cargo test -p capo-server crash_recovery` -> ok, 1 passed (run 5x, 0
  failures); `cargo test -p capo-state` -> ok, 35 passed (3 new). Objective gate:
  `cargo fmt --check` -> clean (exit 0); `cargo clippy --all-targets
  --all-features -- -D warnings` -> clean (exit 0); `cargo test --workspace` ->
  exit 0, 0 failed across all binaries (capo-runtime 16, capo-state 35,
  capo-server 49, capo-controller 23, capo-adapters 29 / 2 ignored, capo-tools 21,
  capo-query 21, capo-voice 19, capo cli 63 + server_transport 11, etc.). No live
  Codex smoke is required for RTL10 (the live workspace-write smoke is RTL13); all
  proofs are deterministic.

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
