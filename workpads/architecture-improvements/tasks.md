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

## AI2 - Real Codex `AgentAdapter` for the chat surface (binding-respecting, fail-closed-fast)

Status: done (controller seam). The PRODUCTION ROUTE that makes this seam
reachable by a user end-to-end is AI4 (also done) -- before AI4 the
`CodexLiveAdapter` had no production caller: `capo server serve` always built the
controller with the fake adapter and `capo server agent register` rejected
non-fake adapters. Priority: high. Source: real-turn-loop arch review.

Problem:

- Chat/steer (`SendTask` -> `send_task`, `SteerAgent` -> `redirect`) drives the
  agent's bound `AgentAdapterHandle` through the `AgentAdapter` trait, but the
  only handle variants were the deterministic `Fake` and `ScriptedMock`
  implementations; `CodexExecAdapter`/`ClaudeCodeAdapter`/`AcpAdapter` were
  PARSER-only structs that did not implement the trait. Real Codex output was
  produced only by the separate `run_live_provider_local` stdout-parsing dispatch
  path — never via chat. So a Codex-profile agent's chat turn still produced a
  canned fake summary.

CORRECTED design (the prior attempt was reverted): the prior attempt made real
Codex the GLOBAL default chat backend, so `SendTask`/`SteerAgent` for FAKE/mock
agents routed into real Codex and HUNG
`capo_planner_tracks_decisions_as_server_state_and_steers_mock_agent` (and would
hang any fake-agent chat). The corrected design RESPECTS THE AGENT'S ADAPTER
BINDING and FAILS CLOSED FAST:

- Real Codex chat applies ONLY to agents explicitly bound to the Codex adapter.
  Fake/scripted/mock agents keep their fake/scripted handle and run
  deterministically, EXACTLY as before AI2 — real Codex is NEVER a global default
  for unbound/mock agents.
- For a Codex-bound agent: when `codex_live_chat_gate_open()` is TRUE the chat
  `send_turn` drives the real read-only one-shot `codex exec --json`; when FALSE
  it returns an IMMEDIATE typed error (no process spawn, no blocking, no waiting),
  mirroring operator-control's fail-closed posture. No chat path blocks the
  server request handler.

Acceptance (met):

- A real Codex `AgentAdapter` implementation
  (`crates/capo-adapters/src/codex_live.rs` `CodexLiveAdapter`) whose chat turn
  spawns + parses the read-only one-shot Codex and returns provider-neutral
  `TurnOutput`. Added as a third `AgentAdapterHandle::Codex` variant; the trait
  gained a fallible `try_send_turn` seam (default-impls to infallible `send_turn`
  for the fake/scripted handles; the Codex handle overrides it with the
  gate-respecting fail-closed path).
- Routing is BY BINDING: the controller chat path
  (`crates/capo-controller/src/fake_session.rs` `send_task`,
  `crates/capo-controller/src/session_control.rs` `redirect`) drives
  `self.adapter.try_send_turn(...)` — the agent's own handle — and maps a typed
  failure to `StateError::CodexLiveChat`. Fake/mock agents never reach the Codex
  path, so nothing "defaults" to Codex.
- The gate (`codex_live_chat_gate_open()` in `codex_live.rs`) requires BOTH
  `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` AND `CAPO_SERVER_RUN_CODEX_LIVE=1`,
  matching the live-provider dispatch opt-ins. With the gate off, a Codex-bound
  chat fails closed FAST (immediate `CodexLiveChatError::GateClosed`); no fake
  output masquerades as real. Claude live stays blocked (no Claude chat handle).
- Production seam: `RealBoundaryController::open_codex_chat(...)` binds the Codex
  chat handle for a Codex-bound agent (honoring an absolute `CAPO_CODEX_BIN`
  override); the default `open`/`open_with_adapter` keep the fake/scripted handle.

Verification (met):

- Real-Codex-chat deterministic test (no live provider):
  `codex_bound_chat_drives_the_real_adapter_through_a_codex_stub_with_gate_open`
  (`crates/capo-controller/src/tests.rs`) writes an executable `codex` STUB pinned
  by absolute path via `with_codex_program_override` (the runtime spawns with
  `env_clear`, so the stub uses only POSIX builtins — `read`/`printf` — and
  streams fixed JSONL from an absolute-path fixture), opens a Codex-bound
  controller with the gate ON, and asserts the chat summary is the STUB's parsed
  `agent_message` text (`CODEX_STUB_CHAT_SUMMARY`) through the `AgentAdapter`
  trait — not a fake-adapter summary.
- Fail-closed test:
  `codex_bound_chat_fails_closed_fast_when_gate_is_off` opens a Codex-bound
  controller pointed at a NON-EXISTENT codex path with the gate OFF and asserts
  `send_task` returns an immediate `StateError::CodexLiveChat` (naming the missing
  opt-in) in well under a second — proving NO spawn and NO hang.
- The existing `capo_planner_tracks_decisions_as_server_state_and_steers_mock_agent`
  passes UNCHANGED (the mock agent's chat still routes through the fake adapter).
- Gate run (worktree, 2026-05-31): `cargo fmt --check` (exit 0); `cargo clippy
  --all-targets --all-features -- -D warnings` (exit 0); `cargo test --workspace`
  COMPLETED green (exit 0, 494 tests passed across binaries, 0 failed; no hang).
  No live Codex smoke required (the deterministic stub path satisfies the
  verification). Did NOT git commit (workflow commits after review).

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

## AI4 - Make AI2 real-Codex chat reachable END-TO-END through the running server

Status: done. Priority: high. Source: AI2 boundary review (the `CodexLiveAdapter`
seam had no production route).

Problem:

- AI2 built a correct, fail-closed-fast `CodexLiveAdapter` and a binding-respecting
  controller seam (`RealBoundaryController::open_codex_chat` /
  `CapoServer::open_with_controller_and_adapter`), but NO production caller wired
  it: `capo server serve` (`serve_tcp` -> `CapoServer::open`) always built the
  controller with the default fake adapter, and `capo server agent register`
  rejected any non-fake `--adapter` (`require_fake_arg` in
  `crates/capo-cli/src/server_client.rs`: "--adapter only supports fake in SV1").
  So real-Codex chat via `SendTask`/`SteerAgent` was UNREACHABLE by a user.

Acceptance (met):

- `capo server agent register --adapter codex` registers a CODEX-BOUND agent. The
  CLI's `require_fake_arg("--adapter")` was relaxed to `require_chat_adapter_arg`,
  which accepts `fake` (the default) and `codex` and rejects everything else
  (`crates/capo-cli/src/server_client.rs`). `--runtime` is still `fake`-only.
  Mock/fake agents are unchanged.
- The `--adapter` binding travels to the server on the wire:
  `ServerCommand::RegisterAgent { name, adapter }` (`crates/capo-server/src/types.rs`),
  encoded/decoded with a back-compatible default (`adapter` omitted => `fake`) in
  `crates/capo-server/src/transport/codec.rs`; the published schema enum gained the
  new `unsupported_chat_adapter` error kind (regenerated
  `contract/jsonrpc-schema.json`).
- The server binds the adapter PER AGENT (not a global default). `CapoServer` holds
  `CodexChatBindings` (an `Arc<Mutex<HashSet<String>>>` of codex-bound agent names +
  the workspace/artifact roots under `<state_root>/codex-chat` + an absolute
  `CAPO_CODEX_BIN` override). `RegisterAgent { adapter: "codex" }` records the
  binding (and rejects any other value with `ServerError::UnsupportedChatAdapter`).
  At `SendTask`/`SteerAgent` the new `CapoServer::chat_controller(agent_name)`
  routes a bound agent through `ControllerRoute::new_codex_bound` (a
  `RealBoundaryController::with_adapter(codex_handle)` view over a CLONE of the
  shared core -- the SQLite store is a path handle, so only the chat handle
  changes), and every other agent through the ordinary `command_controller`. Codex
  is never the global default; the `Fake`/rollback selection keeps the fake adapter
  even for a bound agent. `with_adapter` was added to `FakeBoundaryController` and
  `RealBoundaryController` (`crates/capo-controller/`).
- Fail-closed-fast preserved: a codex-bound chat with the gate OFF
  (`codex_live_chat_gate_open()==false`) returns an IMMEDIATE typed error
  (`StateError::CodexLiveChat` -> wire) -- no spawn, no block -- exactly as the AI2
  adapter already enforced; the server merely routes to it.

Verification (met):

- DETERMINISTIC END-TO-END (always-on): `crates/capo-server/src/tests/codex_chat.rs`
  `codex_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server`
  drives the REAL server path (`serve_tcp` over loopback + `send_tcp` client) with
  a deterministic absolute-path `codex` STUB pinned via `CAPO_CODEX_BIN` and the
  live gate open. It asserts `SendTask`'s returned `external_session_ref` is the
  codex-live binding ref AND the persisted session summary is the STUB's parsed
  `agent_message` text (`CODEX_STUB_E2E_CHAT_SUMMARY`) -- NOT a fake summary -- and
  that a FAKE-bound agent on the SAME server still routes through the fake adapter.
- FAIL-CLOSED-FAST END-TO-END: `codex_bound_chat_fails_closed_fast_end_to_end_when_gate_is_off`
  (gate OFF, codex pinned to a non-existent path) asserts the codex-bound `SendTask`
  returns a typed fail-closed transport error in well under a second (no spawn, no
  hang).
- CLI PROCESS PATH: `crates/capo-cli/tests/server_transport/live.rs`
  `cli_registers_codex_agent_and_gets_real_stub_chat_through_running_server` spawns
  a real `capo server serve` process (gate + `CAPO_CODEX_BIN` stub in the SERVER
  env), runs `capo server agent register --adapter codex` + `capo server task send`,
  and asserts the rendered agent-status `latest_summary` is the stub's real text;
  `cli_rejects_unsupported_chat_adapter_on_register` asserts `--adapter claude` is
  rejected client-side.
- LIVE OPT-IN SMOKE: `codex_live_chat_smoke` (`#[ignore]` + both env gates) sends a
  trivial goal to a codex agent through the real server and asserts real Codex
  output; it skips cleanly when the gates are unset or `codex` is unavailable.
- The mock/planner tests stay GREEN: `capo_planner_tracks_decisions_*` and the
  whole CLI/server suites pass unchanged (a fake agent still routes to the fake
  adapter). The AI1 consolidation kept the planner connection count at 28; AI4
  adds no connections to that flow.
- Gate run (worktree, 2026-05-31): `cargo fmt --all --check` (exit 0); `cargo
  clippy --all-targets --all-features -- -D warnings` (exit 0); `cargo test
  --workspace` COMPLETED green (exit 0, 0 failed; the two new deterministic E2E
  tests pass, the live smoke is `#[ignore]`d). No live Codex required (the
  deterministic stub path satisfies the verification). Did NOT git commit (workflow
  commits after review).

Acceptance: met (real-Codex chat is now reachable end-to-end through the running
server: register `--adapter codex`, then `SendTask`/`SteerAgent` drive the real
read-only one-shot Codex per the agent's binding, fail-closed-fast when the gate is
off, with fake agents unchanged).

## AI5 - Close the autonomy loop: a `Continue` continuation decision must drive the next turn through the single production path

Status: open. Priority: medium. Source: goal-autonomy boundary review (2026-06-01).

Problem:

- The GA4 continuation scheduler (`crates/capo-controller/src/continuation_scheduler.rs`)
  is a sound pure state machine, and `evaluate_and_record_continuation` durably
  records the `continue | pause | block | budget-limit | no-progress-suppress`
  decision (event + `GoalContinuationProjection`) and wires the terminal
  `budget-limit -> run.aborted` path. But a `Continue` decision is NOT wired to
  drive the next turn: there is no `ServerCommand` that evaluates continuation,
  and `evaluate_*continuation` is reached only from the GA e2e/reattach tests
  (`crates/capo-controller/src/goal_autonomy_e2e.rs`, `.../reattach.rs`), which
  simulate "decide continue, then the next turn happens" by manually driving the
  follow-on turn rather than by the scheduler triggering it.
- Net: the autonomy loop is BUILT but not CLOSED on the production path. This is
  acceptable for goal-autonomy (M2-on-substrate, opt-in, off by default, with
  `depth`/integration following), and it correctly did NOT create a second
  orchestration shape. The loop-closing wire is the next architectural step and is
  larger than an inline fix.

Why this is the right boundary:

- The single production orchestration path is `run_dispatch_turn` (AI1). A
  `Continue` decision MUST re-enter THAT path (issue a `ServerCommand::RunDispatchTurn`
  for the goal's attempt run), never a parallel turn driver, so "the loop is the
  loop" holds for autonomous continuation exactly as it does for an operator turn.

Acceptance:

- A production server command (e.g. `ServerCommand::ContinueGoal { goal_id }` or a
  controller-owned tick) evaluates continuation through
  `evaluate_and_record_continuation` and, ONLY on `Continue` and ONLY when
  continuation is explicitly enabled, issues exactly one `RunDispatchTurn` for the
  goal's attempt run through the AI1 single path.
- Opt-in preserved: with continuation disabled the command never dispatches a turn;
  the off-by-default invariant from `goal-autonomy/knowledge.md` Non-Goals holds.
- No second orchestration path: the continued turn produces the SAME dispatch event
  sequence + `TurnFinished` as an operator turn.
- A deterministic test drives the PRODUCTION command (not the bespoke e2e harness)
  and asserts: enabled + safe-boundary -> one continued `RunDispatchTurn` +
  `TurnFinished`; disabled -> no dispatch; `budget-limit` -> `run.aborted`, no
  dispatch.

Verification:

- `cargo fmt --all --check`; `cargo clippy --all-targets --all-features -- -D warnings`;
  `cargo test --workspace`.
