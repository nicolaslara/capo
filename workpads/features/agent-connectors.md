# Agent Connectors Feature

## Objective

Prove that Capo can safely dispatch work to real local subscription-backed coding agents, starting with Codex and then Claude Code, without reading or persisting credential material.

## Prototype Inputs

- P6 parsed Codex, Claude Code, and ACP fixtures into normalized adapter events.
- P7 built restrictive local smoke plans and artifact scanning, but real provider smoke tests were not run.
- P12 proved the controller/evidence path with fake agents.

## Dependencies

- Use `LocalProcessRunner`; do not spawn provider CLIs directly from adapter code.
- Keep subscription connectors local-only and user-owned.
- Preserve read-model ownership: provider streams are adapter inputs, not controller truth.

## Tasks

### AC1 - Codex Opt-In Smoke

Status: waiting_on_opt_in

Acceptance:

- Run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` only after explicit user opt-in.
- Use restrictive defaults: isolated workspace, read-only sandbox, ephemeral mode, ignored user config/rules, no provider-native write/network tools.
- Scan stdout/stderr artifacts and state/evidence trees for credential/session markers.
- Record whether the local Codex connector is safe enough for first dogfood.

Evidence:

- `codex --version`: `codex-cli 0.133.0`
- `codex exec --help` observed 2026-05-25 and supports the planned `--json`, `--sandbox read-only`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, and `--cd` arguments.
- `cargo test -p capo-adapters local_smoke_plan`: passed.
- `cargo test -p capo-adapters local_adapter_smoke_runner`: passed.
- `cargo test -p capo-adapters artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets`: passed.
- `cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`: passed without `CAPO_RUN_CODEX_LOCAL_SMOKE=1`, proving the ignored real smoke remains opt-in gated.

Skipped verification:

- The real Codex subscription-backed smoke was not run because the task requires explicit user opt-in before setting `CAPO_RUN_CODEX_LOCAL_SMOKE=1`.

Decision:

- The Codex connector is not yet safe enough for first dogfood because the actual subscription-backed process has not run. The harness shape remains appropriate: local process runtime, isolated temporary workspace, read-only sandbox, ephemeral mode, ignored user config/rules, bounded/redacted artifacts, and marker scanning.

Review:

- Focused review found no blockers. It confirmed the real Codex smoke remains opt-in gated and the workpad does not claim real-agent readiness.

### AC2 - Claude Code Restricted Args Verification

Status: completed

Acceptance:

- Verify installed Claude Code CLI restricted permission/tool arguments before running a subscription-backed smoke.
- Keep empty MCP config and disallowed tools unless the user explicitly scopes more access.
- Record unsupported or drifting CLI args as a compatibility issue, not a product failure.

Evidence:

- `claude --version`: `2.1.150 (Claude Code)`.
- `claude --help` observed 2026-05-25 and supports `-p`, `--output-format stream-json`, `--permission-mode plan`, `--tools`, `--disallowedTools`, `--mcp-config`, `--strict-mcp-config`, `--no-session-persistence`, and `--disable-slash-commands`.
- `crates/capo-adapters/src/lib.rs` now includes `--no-session-persistence`, `--disable-slash-commands`, and `--tools ""` in the Claude smoke plan, in addition to `--permission-mode plan`, `--disallowedTools *`, empty MCP config, and strict MCP config.
- `cargo test -p capo-adapters local_smoke_plan`: passed.

Decision:

- Claude Code restricted-argument compatibility is ready for a future opt-in smoke. The smoke itself remains gated behind `CAPO_RUN_CLAUDE_LOCAL_SMOKE=1` and should not run until explicitly authorized.

Review:

- Focused review found no blockers. It confirmed the documented Claude flags match the installed help surface, including `--tools ""`, `--no-session-persistence`, and `--disable-slash-commands`.

### AC3 - Real-Agent Controller Path

Status: in_progress

Acceptance:

- Route at least one successful real local adapter event stream through Capo state/read models.
- Export markdown evidence with no credential material.
- Keep fake fixtures available as deterministic regression tests.

Progress:

- Deterministic replay support is completed for normalized Codex and Claude fixture streams. This does not claim real-agent readiness, but it proves parsed provider events can flow through controller-owned state/read models without launching subscription-backed CLIs.
- CLI fixture replay is completed for deterministic local evidence export. This gives operators a non-subscription-backed e2e command for replaying normalized provider fixtures through Capo state/read models and markdown evidence.

Evidence:

- `FakeBoundaryController::apply_normalized_adapter_events` replays normalized adapter events into session summary, native tool-call, and evidence projections.
- `cargo test -p capo-controller replay -- --nocapture`: passed.
- Codex replay regression covers `fixtures/codex-exec.jsonl`, updates `session.summary_updated`, `tool_calls`, and `evidence`, and asserts event payloads do not persist raw provider message/tool text.
- Claude replay regression covers `fixtures/claude-code-stream.jsonl`, preserves tool name across tool-result updates, updates evidence, and asserts event payloads do not persist raw provider message/tool text.
- `capo adapter replay-fixture --adapter codex|claude|acp --fixture PATH --agent NAME --goal GOAL [--out DIR]` routes fixture replay through the controller and optional evidence export.
- `cargo test -p capo-cli adapter_fixture -- --nocapture`: passed.

Skipped verification:

- A real local Codex or Claude adapter process has still not been run because subscription-backed smokes require explicit user opt-in.

### AC4 - Connector Readiness Surface

Status: completed

Acceptance:

- Add a deterministic operator command that reports configured Codex/Claude smoke gates without launching provider CLIs.
- Report opt-in env vars, restrictive smoke-plan metadata, redaction configuration, and dogfood blocker status.
- Do not read provider credentials, inspect vendor subscription state, create smoke workspaces, or run real smokes.

Evidence:

- `capo adapter readiness [--state PATH]` in `../../crates/capo-cli/src/main.rs`.
- The command reports `credential_policy=not_inspected`, the Codex and Claude opt-in env vars, smoke markers, env allowlist counts, redaction rule counts, and `ready_for_real_agent_dogfood=false` until a real subscription smoke is recorded separately.
- `cargo test -p capo-cli adapter_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- This is an operator/readiness surface only. It does not satisfy AC1 or AC3 because no subscription-backed Codex or Claude process is run.

### AC5 - Durable Connector Readiness State

Status: completed

Acceptance:

- Persist adapter readiness rows when the operator explicitly records a readiness check.
- Show recorded readiness in the shared dashboard/read-model path.
- Keep the recorded status honest: readiness state may show smoke-plan gates and opt-in status, but it must not mark real-agent dogfood ready before a real smoke is recorded.

Evidence:

- `AdapterReadinessProjection`, `EventKind::AdapterReadinessChecked`, SQLite migration, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- `capo adapter readiness --record` writes Codex/Claude readiness rows without launching provider CLIs.
- `capo dashboard` renders recorded adapter readiness rows with opt-in status, credential policy, smoke status, redaction/env policy counts, and dogfood blocker.
- `cargo test -p capo-state adapter_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_readiness -- --nocapture`: passed.

Decision:

- Recorded readiness keeps `dogfood_blocker=real_subscription_smoke_not_recorded`. Only a future explicit real-smoke evidence path should clear it.
