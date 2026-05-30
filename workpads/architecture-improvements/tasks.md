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

Status: pending. Priority: high. Source: real-turn-loop arch review.

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
