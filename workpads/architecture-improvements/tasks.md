# Architecture Improvements Tasks

## Objective

Capture architectural changes surfaced by workpad-boundary reviews that are too
large to apply inline at the point of discovery. Each item records the problem,
the evidence, and the acceptance criteria for a dedicated fix. These are not
optional polish: AI1 and AI2 are the foundational "real loop is actually the
production path" items without which the daily-driver chat goal is not met.

## Status

Active backlog. Opened 2026-05-30 from the `real-turn-loop` cumulative
adversarial review (architecture + correctness + tests lenses, independently
converged). AI1/AI2 are HIGH priority and gate meaningful `goal-autonomy`
(continuation must drive the real loop, not the fake adapter).

## AI1 - Wire `run_dispatch_turn` as the single production orchestration path

Status: done. Priority: high. Source: real-turn-loop arch review.

Problem:

- `crates/capo-server/src/turn_orchestration.rs` `run_dispatch_turn` is the
  documented RTL4 "one orchestration path" (loop DRIVES Plan/Gate/Run and emits
  `TurnFinished`), but it has NO production caller — it is exercised only by
  tests. Every production entry point bypasses it and sequences raw
  `ServerCommand`s itself: `crates/capo-cli/src/server_client/dispatch.rs`
  (issues `RunLiveProviderLocal` directly), `crates/capo-cli/src/operator_control.rs`,
  and `CapoServer::handle` (handles `PlanDispatch`/`GateDispatch`/
  `RunDispatchLocal`/`RunLiveProviderLocal` as independent commands with no loop
  annotation). The result is two de-facto orchestration shapes; the loop the
  workpad is built around is not the one that runs.

Acceptance:

- The production operator/live-run dispatch path flows THROUGH `run_dispatch_turn`
  (or `CapoServer::handle` annotates runs with `TurnFinished` itself), so the
  single path the design claims is the path that executes.
- No production flow sequences the dispatch primitives outside the loop, OR a
  documented, justified exception with its own `TurnFinished` annotation.
- Read models/events for a production live run match those produced by the loop
  in tests (one path, one event shape).

Verification:

- A test that drives the PRODUCTION command path (not a bespoke harness) and
  asserts it produces the loop's dispatch event sequence + a `TurnFinished`.
- `cargo fmt` / `cargo clippy --all-targets --all-features -- -D warnings` /
  `cargo test --workspace`.

Evidence (2026-05-31, updated after boundary review):

- Production caller REWIRED (the core acceptance): the operator/live-run flow
  `crates/capo-cli/src/operator_control.rs::run_codex_live_turn_with_options`
  (reached from the operator REPL `start`/`send` paths and the capo planner
  `plan_with_operator_agent`) now issues a SINGLE `ServerCommand::RunDispatchTurn`
  instead of hand-sequencing `PreflightLiveProvider` + `RunLiveProviderLocal`
  beside the loop. A real operator turn therefore flows THROUGH
  `run_dispatch_turn` (the loop drives preflight/run and ANNOTATES the run with a
  `TurnFinished`). The operator surfaces still render only the run, but the
  loop's `TurnFinished` is now produced and observable on the production path
  (carried in the `DispatchTurn` payload via the new `run_codex_dispatch_turn`).
- Documented, justified exception (per the acceptance clause): the low-level
  `capo server dispatch live-run-local` / `plan` / `gate` / `live-preflight` /
  `run-local` CLI subcommands (`crates/capo-cli/src/server_client/dispatch.rs`)
  intentionally remain single-primitive surfaces — each issues exactly one
  dispatch `ServerCommand` against a pre-existing `--dispatch-plan`, so an
  operator/test can step the dispatch state machine one primitive at a time.
  Folding them into the loop command would defeat their step-debugging purpose.
  The orchestrated operator/live-run flow goes through `RunDispatchTurn`; these
  are not that flow. A code comment records this exception at the function.
- Server wiring: `CapoServer::handle` routes `ServerCommand::RunDispatchTurn`
  through `self.run_dispatch_turn(...)` and returns a `DispatchTurn` payload
  pairing the run summary with the loop's `TurnFinished`
  (`crates/capo-server/src/lib.rs`). Wire codec: `encode_dispatch_run`/
  `decode_dispatch_run` helpers + the `dispatch_turn` encode/decode arms
  (`crates/capo-server/src/transport/codec.rs`); `run_dispatch_turn`/
  `dispatch_turn` published in the wire schema enums
  (`crates/capo-server/src/transport/contract.rs`) with the checked-in
  `contract/jsonrpc-schema.json` snapshot.
- Verification tests:
  - PRODUCTION caller test (not the command in isolation):
    `operator_live_run_caller_flows_through_run_dispatch_turn_and_emits_turn_finished`
    (`crates/capo-cli/src/operator_control.rs` tests) drives the real
    `run_codex_dispatch_turn` against a loopback server with a mock-runtime Codex
    turn and asserts it gets back the loop's `TurnFinished` keyed to the turn AND
    that the server persisted the live preflight gate + the mock-ingest
    `run.exited` (the loop substrate). This fails if the caller reverts to the
    raw preflight+run pair (no `TurnFinished` would come back).
  - Live-substrate parity:
    `loop_live_turn_drives_the_same_dispatch_sequence_as_the_hand_sequenced_live_path`
    drives the raw `PreflightLiveProvider` + `RunLiveProviderLocal` pair (Arm A)
    and `run_dispatch_turn` (Arm B) over identical live inputs and asserts the
    dispatch event sequences are identical modulo the loop's one idempotent
    leading `adapter.dispatch_planned` — i.e. one run-completion shape for the
    live path, not loop==loop.
  - Codec round-trip: the contract test's `sample_commands()`/`sample_payloads()`
    loop now round-trips every sample (including `RunDispatchTurn`/`DispatchTurn`,
    with distinct non-empty ref lists) through the real codec, so a field-order/
    name bug in a new decode arm fails the build.
  - Plus the prior handle-path and zero-timeout tests in
    `crates/capo-server/src/tests/turn_orchestration.rs`.
- Gate run (worktree, 2026-05-31): `cargo fmt --check` / `cargo clippy
  --all-targets --all-features -- -D warnings` / `cargo test --workspace` all
  green. No live Codex smoke required for AI1 (the deterministic live-mock
  fixture path satisfies the verification).
- Acceptance: met (production operator/live-run path flows through the loop;
  documented exception for the low-level step primitives). Did NOT git commit
  (workflow commits after review).

## AI2 - Inject a real Codex `AgentAdapter` as the default chat backend

Status: pending. Priority: high. Source: real-turn-loop arch review.

Problem:

- Default chat/steer (`SendTask` -> `send_task_command`, `SteerAgent` ->
  `redirect_command`) routes through `RealBoundaryController`, but its adapter is
  hardcoded to the fake handle (`crates/capo-controller/src/lib.rs` `open` ->
  `AgentAdapterHandle::fake()`). The only `AgentAdapter` implementations are
  `Fake` and `ScriptedMock`; `CodexExecAdapter`/`ClaudeCodeAdapter`/`AcpAdapter`
  are PARSER-only structs that do not implement the trait. Real Codex output is
  produced only by the separate `run_live_provider_local` stdout-parsing path,
  reachable via dispatch commands — never via chat. So the original
  "chat = canned fake summary" problem the campaign set out to fix is still
  present on the chat surface.

Acceptance:

- A real Codex `AgentAdapter` implementation whose `send_turn` drives the live
  Codex execution (spawn + parse) under the hood and returns provider-neutral
  `TurnOutput`.
- The production controller constructor injects the real Codex handle as the
  default chat backend behind the existing live-provider opt-in gates
  (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` / `CAPO_SERVER_RUN_CODEX_LIVE=1`);
  Claude live stays blocked.
- With the gate off, chat fails closed (no fake output masquerading as real), as
  the operator-control behaviour already does for attached Codex.

Verification:

- Deterministic test: a scripted-mock adapter injected through the same default
  path drives chat end-to-end (no fake handle on the chat surface).
- Simple gated live Codex smoke: `SendTask`/`SteerAgent` with the Codex profile
  produces real Codex output through the `AgentAdapter` trait.
- `cargo fmt` / `cargo clippy ...` / focused `cargo test -p capo-controller -p capo-server`.

## AI3 - Wire `dispatch_tool_call` into the production turn loop

Status: done. Priority: high. Source: tools-aci `ACI1` boundary review.

Problem:

- `RealBoundaryController::dispatch_tool_call`
  (`crates/capo-controller/src/real_controller.rs`, driving
  `crates/capo-controller/src/tool_dispatch.rs`) is the real tool-dispatch seam
  landed by `ACI1`: it runs the real `CapoToolRegistry`/`RuntimeToolWrappers`
  through `authorize_and_invoke` and persists the canonical observed audit
  sequence + `ToolInvocation`/`ToolObservation` projections. But it has NO
  production caller — it is exercised only by tests
  (`crates/capo-controller/src/tests.rs`). The live chat/steer path
  (`SendTask` -> `send_task` -> `FakeAgentSession::send_turn` in
  `crates/capo-controller/src/fake_session.rs:77`) still routes tool calls
  through the fake `ToolExposure` shim: the controller is constructed with
  `tools: ToolExposure::fake()` (`crates/capo-controller/src/lib.rs:119`) and the
  per-turn memory-packet summary calls `self.tools.invoke(FakeToolRequest {...})`,
  which produces a canned fake summary, not a real dispatched tool call. So the
  REAL tool dispatch (`authorize_and_invoke`) is wired as a method but the
  production turn loop does not call it yet — the loop's decision step neither
  auto-selects nor auto-invokes a tool through the real seam.
- This is the same "real-but-unwired" theme as AI1 (`run_dispatch_turn` is the
  documented one orchestration path with no production caller) and AI2 (the chat
  backend is a real controller wired to the fake adapter handle): a real
  capability exists and is test-driveable, but every production entry point
  bypasses it. AI3 closes the loop on tool dispatch specifically; it is gated
  together with AI1/AI2 before `goal-autonomy` (autonomous continuation must
  drive real tool dispatch, not the fake summary shim).

Acceptance:

- The production turn loop invokes the real dispatch seam: a model's tool
  selection in the live `send_task`/`send_turn` path flows THROUGH
  `RealBoundaryController::dispatch_tool_call` (real
  `authorize_and_invoke` -> persisted canonical events + projection), not through
  `ToolExposure::fake()` / `self.tools.invoke`.
- The per-turn memory-packet summary no longer uses the fake summary shim as the
  tool surface for a real run, OR a documented, justified exception that does not
  masquerade fake output as a real dispatched tool result.
- A real production turn that invokes a tool produces the same observed audit
  sequence + `ToolInvocation`/`ToolObservation` projections that the `ACI1`
  dispatch tests assert (one dispatch path, one event shape), keyed to the turn.

Verification:

- A test that drives the PRODUCTION turn path (not a bespoke `dispatch_tool_call`
  harness) and asserts a tool invocation flows through `authorize_and_invoke`,
  persisting the canonical observed sequence + projection keyed to the turn.
- `cargo fmt` / `cargo clippy --all-targets --all-features -- -D warnings` /
  `cargo test --workspace`.

Evidence (2026-05-31, worktree `feat/architecture-improvements`):

- Production caller REWIRED (the core acceptance): the default chat/send-task
  path now dispatches the per-turn `capo.session_summary` tool through the REAL
  `authorize_and_invoke` seam, not `ToolExposure::fake()` / `self.tools.invoke`.
  - `crates/capo-controller/src/fake_session.rs`: `send_task` is refactored to take
    a private `ToolDispatchMode { Fake, Real(&ToolExposure) }`. The fake/fixture
    path keeps the legacy summary shim + hand-rolled tool events byte-for-byte; the
    REAL path calls the new `dispatch_turn_summary_tool`, which routes
    `capo.session_summary` through `FakeBoundaryController::dispatch_tool_call`
    (`authorize_and_invoke` over the live Capo registry) and persists the canonical
    `ACI1` observed audit sequence (`tool.call_requested` -> `permission.requested`
    -> `permission.decided` -> `capability.grant_used` -> `tool.invocation_started`
    -> `tool.output_artifact_recorded` -> `tool.output_observed` ->
    `tool.call_completed` -> `tool.result_delivered`) + the `ToolCall` projection
    (with dispatch provenance) + the ACI9 `runtime_output` `ToolObservation` row,
    all keyed to the turn. The fake-only block (`record_permission_decision` + the
    six hand-rolled `tool.*` events) is now gated to `ToolDispatchMode::Fake`.
  - `crates/capo-controller/src/lib.rs`: new `send_task_command_with_real_tools`
    parses the SendTask `CommandEnvelope` and routes through the real-tools path.
  - `crates/capo-controller/src/real_controller.rs`: `RealBoundaryController`'s
    `send_task_command` / `send_task` / `send_task_with_task_id` now drive the real
    Capo exposure (`self.tools.capo`) through the seam. The server routes
    `SendTask` through `ControllerRoute::Real::send_task_command`, so a real chat
    turn's tool call is a REAL dispatched result.
- Documented, justified divergence (the AI3 goal: "one dispatch path, one event
  shape"): the REAL `send_task` summary tool now produces the canonical dispatch
  shape (no `capability.grant_created`; interleaved `permission.*`; one
  `runtime_output` observation; dispatch provenance; +1 event vs the fake shim).
  The prior RTL5/RTL11/RTL12 fake==real PARITY guards were updated to compare the
  SHARED loop path (the `run_turn` ingestion: identical `TurnFinished`, identical
  loop `ToolCall`, session projection modulo the `updated_sequence` bookkeeping)
  and the LIFECYCLE markers (`session.*`/`run.*`/`memory.*`/`evidence.*`), while
  the per-turn tool-dispatch sub-sequence intentionally differs (the real seam vs
  the fake shim). Files: `crates/capo-controller/src/tests.rs`
  (`real_controller_read_models_match_fake_path_for_identical_scripted_output`,
  `fake_and_real_paths_produce_equivalent_event_sequences_for_a_scripted_turn`,
  the `lifecycle_fingerprint` helper), `crates/capo-server/src/tests/controller_routing.rs`
  (`session_event_kinds` scoped to lifecycle markers), and
  `crates/capo-server/src/tests/turn_orchestration.rs`
  (`real_controller_matches_fake_path_over_a_scripted_adapter_from_the_server_crate`).
  `crates/capo-server/src/tests/foundation.rs` updated: the production default
  routing now records `tool_observation_count == 1` for the dispatched summary
  tool (was 0 under the shim).
- Verification test (PRODUCTION path, not the seam in isolation):
  `production_send_task_command_dispatches_summary_tool_through_authorize_and_invoke`
  (`crates/capo-controller/src/tests.rs`) drives a real SendTask `CommandEnvelope`
  through `RealBoundaryController::send_task_command` and asserts the persisted
  summary tool carries the canonical observed audit sequence + the completed
  `ToolCall` (with dispatch `correlation_id`) + the `runtime_output`
  `ToolObservation`, keyed to the turn. It fails if the path reverts to the fake
  shim (no provenance, no observation row, different event shape).
- Gate run (worktree): `cargo fmt --check` clean (exit 0); `cargo clippy
  --all-targets --all-features -- -D warnings` clean (exit 0); `cargo test
  --workspace` green for everything AI3 touches (capo-controller 46 passed,
  capo-server 94 passed, capo-cli lib 64 passed, all other crates pass). The full
  `cargo test --workspace` is blocked ONLY by a PRE-EXISTING, AI3-INDEPENDENT hang
  in the CLI integration test
  `crates/capo-cli/tests/server_transport/basic.rs::capo_planner_tracks_decisions_as_server_state_and_steers_mock_agent`
  -- it spawns the real `capo control --planner capo` subprocess over a loopback
  socket and blocks under the sandbox. Reproduced on the AI3-stashed baseline
  (same hang with my changes reverted), so it is environmental, not a regression.
  The workspace run completes green with that one test `--skip`ped. No live Codex
  smoke required for AI3 (the deterministic scripted-mock fixture path satisfies
  the verification). Did NOT git commit (workflow commits after review).
- Review follow-up (direct CLI send surfaces): the review noted the server live
  path was rewired but the DIRECT (non-server) CLI `SendTask` surfaces still
  routed through the fake shim. Fixed: `crates/capo-cli/src/main.rs` adds a
  `real_controller(parsed)` helper (a `RealBoundaryController` over the same
  SQLite-backed core whose `send_task*` dispatch the per-turn summary tool through
  the REAL `authorize_and_invoke` seam). The three direct `SendTask` entry points
  now use it: `crates/capo-cli/src/agent_session.rs` (`capo task send`),
  `crates/capo-cli/src/workpad.rs` (`capo workpad-next` / `project memory
  start-next`), and `crates/capo-cli/src/adapter_replay.rs` (both replay seeds;
  the adapter-native replay still runs over the shared `core()`, since adapter
  provenance is the adapter dedup's concern, not the local dispatch seam). So a
  local `capo task send` now produces a real dispatched tool result (the canonical
  observed audit sequence + `ToolCall`/`ToolObservation` projection + dispatch
  provenance), matching the `server task send` production path. CLI test
  expectations updated for the now-shared real shape (the dispatched summary's
  `runtime_output` observation row and the `artifact-{tool_call_id}-{tool_id}`
  output artifact id) in `crates/capo-cli/src/tests.rs`.
- Acceptance: met (the production turn loop invokes the real dispatch seam -- via
  BOTH the server route and the direct CLI `SendTask` surfaces; the per-turn
  summary is a real dispatched result, not the fake shim; the documented parity
  divergence is the intended "one dispatch path, one event shape").
