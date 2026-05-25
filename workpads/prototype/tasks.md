# Prototype Tasks

## Objective

Build and verify the minimal e2e Capo that proves the architecture with a real agent loop, durable state, a control surface, and markdown-compatible evidence.

Prototype work starts after the architecture gate unless explicitly authorized as a spike.

## P0 - Workspace Scaffold And Toolchain

Status: completed

Acceptance:

- Rust/Python/hybrid scaffold decision recorded.
- Initial Cargo workspace and package layout created.
- Format/lint/test commands recorded.
- `capo --help` skeleton runs without requiring provider credentials.

Evidence:

- `Cargo.toml`
- `crates/capo-cli`
- `crates/capo-core`
- `crates/capo-state`
- `crates/capo-adapters`
- `crates/capo-runtime`
- `crates/capo-tools`
- `crates/capo-memory`
- `crates/capo-eval`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo run -p capo-cli -- --help`

## P1 - Core Domain And Boundary Skeleton

Status: completed

Acceptance:

- Define typed IDs, command envelopes, status vocabulary, and core agent/session/run/turn/tool/memory/evidence records.
- Add static dispatch wrappers for fake adapter, runtime, tunnel, provider, permission, tool, memory, and evaluation variants.
- Unit tests prove fake variants can be wired through the controller without persistence.

Evidence:

- `crates/capo-core/src/lib.rs`
- `crates/capo-adapters/src/lib.rs`
- `crates/capo-runtime/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-tools/src/lib.rs`
- `crates/capo-memory/src/lib.rs`
- `crates/capo-eval/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo run -p capo-cli -- --help`

## P2 - SQLite Event Store And Projections

Status: completed

Acceptance:

- Persist controller-owned events, agents, sessions, tasks, runs, summaries, capability grants, tool calls, memory packet refs, and evidence refs.
- Store large payloads as artifacts referenced by SQLite rows.
- Define and test projection rebuild and restart recovery shape.

Evidence:

- `crates/capo-state/src/lib.rs`
- `crates/capo-state/Cargo.toml`
- `Cargo.lock`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P3 - Fake Boundary E2E

Status: completed

Acceptance:

- Drive `FakeAdapter`, `FakeRuntimeRunner`, fake provider, trusted-local permission policy, fake memory packet, and fake tools through the real controller.
- Create task/session, send work, track status/latest summary/recent events, and interrupt/stop the fake run.
- Verify all observations come from read models, not live fake state.

Evidence:

- `crates/capo-controller/src/lib.rs`
- `crates/capo-controller/Cargo.toml`
- `crates/capo-adapters/src/lib.rs`
- `crates/capo-runtime/src/lib.rs`
- `crates/capo-tools/src/lib.rs`
- `crates/capo-memory/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P4 - First CLI Control Surface

Status: completed

Acceptance:

- Provide CLI commands for init, agent register/spawn, task send, session status, interrupt/stop, recover, and evidence export.
- Show active agents, status, current goal, recent events, latest summary, blockers, confidence, and evidence refs.
- CLI commands use command envelopes and read models.

Evidence:

- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/Cargo.toml`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Manual CLI smoke with temporary state/evidence directories:
  - `cargo run -p capo-cli -- init --state <tmp>`
  - `cargo run -p capo-cli -- agent spawn --name fake-codex --adapter fake --runtime fake --state <tmp>`
  - `cargo run -p capo-cli -- task send --agent fake-codex --goal "Inspect the project and write a short status summary" --scenario tool-memory --state <tmp>`
  - `cargo run -p capo-cli -- session status --agent fake-codex --state <tmp>`
  - `cargo run -p capo-cli -- recover --state <tmp>`
  - `cargo run -p capo-cli -- evidence export --session session-fake-codex --out <tmp-evidence> --state <tmp>`
  - `cargo run -p capo-cli -- session stop --agent fake-codex --reason "smoke stop" --state <tmp>`

## P5 - Local Process Runtime

Status: completed

Acceptance:

- Spawn and stop one local process through `LocalProcessRunner`.
- Capture stdout/stderr as redacted artifacts and normalized runtime events.
- Implement interrupt, terminate, kill, health, cleanup, and orphan recovery behavior.
- Preserve runtime/connectivity separation.

Evidence:

- `crates/capo-runtime/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P6 - Adapter Fixture Parsers

Status: completed

Acceptance:

- Add non-secret golden streams for Codex `exec --json`, Claude Code `-p --output-format stream-json`, and ACP replay fixtures.
- Map fixtures into normalized adapter events without making provider-specific fields controller truth.
- Include duplicate/replay fixture tests where stable identifiers exist.

Evidence:

- `crates/capo-adapters/src/lib.rs`
- `crates/capo-adapters/fixtures/codex-exec.jsonl`
- `crates/capo-adapters/fixtures/claude-code-stream.jsonl`
- `crates/capo-adapters/fixtures/acp-replay.jsonl`
- `crates/capo-adapters/Cargo.toml`
- `Cargo.lock`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P7 - Real Local Adapter Smoke

Status: waiting_on_opt_in

Acceptance:

- Run a harmless opt-in Codex local adapter smoke after fixture parsing and redaction pass.
- Add Claude Code opt-in smoke when its non-interactive path is equally safe.
- Subscription-backed connector code remains local-only and never reads vendor credential material.
- Use restrictive adapter launch defaults: isolated temp workspace, no MCP configs, no browser tools, no provider-native write/network tools unless explicitly scoped, Codex read-only sandbox by default, and Claude restricted `--allowedTools` / permission mode.
- Fail the smoke if persistent artifacts are unclassified or contain credentials, cookies, tokens, raw sensitive transcripts, or unredacted provider/session material.

Partial evidence:

- `crates/capo-adapters/src/lib.rs`
- `crates/capo-adapters/Cargo.toml`
- `Cargo.lock`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Remaining:

- Run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` or equivalent once the user explicitly opts in to using the local Codex subscription connector.
- Run the Claude Code smoke only after the restricted `--permission-mode` / tool-disabling arguments are verified against the installed Claude Code CLI.

## P8 - Capo Tools And Permission Audit

Status: completed

Acceptance:

- Implement `capo.task_status`, `capo.agent_status`, `capo.session_summary`, `capo.workpad_read`, `capo.evidence_record`, and `capo.capability_request`.
- Prove tool request, permission decision, grant use, invocation, output artifact, and adapter result-delivery events.
- Trusted local policy allows broadly but still emits auditable permission records.

Evidence:

- `crates/capo-tools/src/lib.rs`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P9 - Memory Packet And Context Provenance

Status: completed

Acceptance:

- Build a source-linked memory packet from local events and markdown/workpad pointers.
- Store a replayable packet artifact and attach it to a run/turn.
- Inspect packet inclusion/exclusion reasons through read models.

Evidence:

- `crates/capo-memory/src/lib.rs`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P10 - Restart Recovery And Replay

Status: completed

Acceptance:

- Restart Capo against an existing SQLite store.
- Rebuild projections without duplicate read-model rows.
- Define and enforce event idempotency behavior for replay, either with a partial unique index on `idempotency_key` or explicit duplicate lookup semantics.
- Recover, orphan, or exit-mark active-looking runs with durable events.
- Include ACP fixture replay/dedupe tests before claiming broad ACP compatibility.

Evidence:

- `crates/capo-state/src/lib.rs`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P11 - Workpad Evidence Export

Status: completed

Acceptance:

- Export workpad-like markdown evidence for completed and interrupted runs.
- Preserve human-auditable fallback and avoid corrupting existing project workpads.
- Include evidence refs back to state/artifact IDs.

Evidence:

- `crates/capo-cli/src/main.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P12 - Prototype E2E Smoke

Status: completed

Acceptance:

- Execute the smoke path from `workpads/architecture/prototype-plan.md`.
- Start Capo, register/spawn two fake agents, send work, inspect status/events/summary, redirect one session, interrupt/stop one session, restart, recover, and export evidence.
- Force at least one Capo tool request, permission audit event, adapter result delivery, and memory packet artifact through the fake smoke scenario.
- Confirm logs/artifacts contain no provider credentials, subscription tokens, cookies, or sensitive raw transcripts.

Evidence:

- `crates/capo-cli/src/main.rs`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P13 - Dashboard/TUI Slice

Status: completed

Acceptance:

- Add the smallest dashboard or TUI that reads the same projections as the CLI.
- Show active agents, sessions, current goals, blockers, recent events, and evidence refs.
- Record whether this is required before dogfood or can follow the first dogfood migration.

Evidence:

- `crates/capo-cli/src/main.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P14 - Conversational Voice Spike

Status: completed

Acceptance:

- Define the voice command/read-model contract for asking Capo about agent status and steering sessions.
- Use dummy transcript/input data only.
- Record transcript retention, redaction, and memory-ingestion decisions before any real voice capture.

Evidence:

- `crates/capo-voice/src/lib.rs`
- `crates/capo-voice/Cargo.toml`
- `Cargo.toml`
- `Cargo.lock`
- `crates/capo-core/src/lib.rs`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## P15 - Prototype Gate Review

Status: completed

Acceptance:

- Review evidence from P0-P12 against `workpads/prototype/spec.md`.
- Gate result recorded in `knowledge.md`.
- Dogfood readiness gaps listed.
- Feature workpads split from findings.

Evidence:

- `workpads/prototype/knowledge.md` Prototype Gate section
- `workpads/prototype/spec.md`
- `workpads/architecture/prototype-plan.md`
- `crates/capo-cli/src/main.rs`
- `workpads/features/tasks.md`
- `workpads/features/agent-connectors.md`
- `workpads/features/dogfood-bridge.md`
- `workpads/features/dashboard.md`
- `workpads/features/permissions-tools.md`
- `workpads/features/memory-eval.md`
- `workpads/features/voice.md`
- `workpads/features/remote-runtime.md`
- Focused review subagent: prototype gate pass with constraints; targeted smoke passed
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Notes:

- P13 dashboard/TUI and P14 conversational voice are post-smoke MVP/spike tasks unless A8 or the user makes either one a dogfood prerequisite.
