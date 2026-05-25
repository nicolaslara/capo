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

### AC6 - Real Smoke Evidence Contract

Status: completed

Acceptance:

- Add a durable record shape for future real subscription-backed smoke outcomes.
- Allow skipped/failed/passed smoke reports, but only allow a passed report when the expected marker is present and credential scan is clean.
- Show smoke reports in the shared dashboard/read-model path.
- Do not run provider CLIs as part of the report command.

Evidence:

- `AdapterSmokeReportProjection`, `EventKind::AdapterSmokeRecorded`, SQLite migration, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- `capo adapter smoke-report record --adapter codex|claude --status skipped|passed|failed --credential-scan clean|blocked|not_run --reason TEXT [--marker-found] [--artifact-root PATH]`.
- `capo dashboard` renders smoke report status, credential scan status, marker status, dogfood effect, artifact root, and reason.
- `cargo test -p capo-state adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.

Decision:

- AC6 is an evidence contract, not a real smoke. A skipped report preserves `dogfood_readiness_effect=real_subscription_smoke_not_recorded`.

### AC7 - Dogfood Readiness Gate

Status: completed

Acceptance:

- Add a deterministic gate that decides whether Capo may start first real-agent dogfood from recorded connector evidence.
- Keep the gate read-only and do not run provider CLIs or inspect subscription credentials.
- Require a successful Codex real-smoke report with clean credential scan and expected marker before clearing the first dogfood blocker.
- Expose the gate through the shared query contract so CLI/dashboard/voice/web surfaces use the same readiness rule.

Evidence:

- `AdapterDogfoodGate` and shared gate computation in `../../crates/capo-query/src/lib.rs`.
- `capo adapter dogfood-gate [--state PATH]` in `../../crates/capo-cli/src/main.rs`.
- `capo dashboard` renders the same shared dogfood gate.
- `cargo test -p capo-query adapter_dogfood -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

Decision:

- The first real-agent dogfood gate is cleared only by recorded Codex evidence with `smoke_status=passed`, `credential_scan_status=clean`, marker present, and `dogfood_readiness_effect=real_agent_connector_proven`.
- The gate is read-only and evidence-derived. It does not launch Codex or Claude, inspect subscription state, or read credentials.
- Claude remains a target connector, but first dogfood can start after Codex is proven because AC1 explicitly defines Codex as the first local connector proof.

### AC8 - Smoke Artifact Scan Enforcement

Status: completed

Acceptance:

- Expose a command that scans smoke artifact files for credential/session markers without launching provider CLIs.
- Require the scan to pass before accepting any `passed` smoke report.
- Keep failed/skipped smoke reports recordable without an artifact directory so operators can document blockers.
- Cover raw secret marker rejection and clean artifact acceptance with CLI tests.

Evidence:

- `capo adapter smoke-report scan --artifact-root PATH` in `../../crates/capo-cli/src/main.rs`.
- `capo adapter smoke-report record --status passed ... --artifact-root PATH` now scans the artifact directory before accepting a passed report.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

Decision:

- A passed smoke report must include an artifact root and the artifact scan must pass. Operators can still record skipped/failed reports without artifacts to document blockers.
- The scan reuses the adapter-layer sensitive marker scanner and does not launch provider CLIs, inspect subscriptions, or read credential stores.
- This makes the dogfood readiness gate depend on a Capo-verified artifact scan rather than only an operator-provided `--credential-scan clean` flag.

### AC9 - Local Adapter Launch Contract

Status: completed

Acceptance:

- Define a reusable local launch plan for Codex and Claude Code that can build runtime configs and runtime requests without launching provider CLIs.
- Keep subscription credential handling explicit as `user_local_subscription` with vendor CLI login, not API-key or token material.
- Preserve restrictive defaults from the smoke harness for normal local launch planning.
- Reject secret-like env allowlist entries or argv markers before a subscription-backed launch plan can be treated as safe.

Evidence:

- `LocalAdapterLaunchPlan` in `../../crates/capo-adapters/src/lib.rs`.
- `CodexExecAdapter::local_launch_plan(...)` and `ClaudeCodeAdapter::local_launch_plan(...)` in `../../crates/capo-adapters/src/lib.rs`.
- `cargo test -p capo-adapters launch_plan -- --nocapture`: passed.
- `cargo test -p capo-adapters local_smoke_plan -- --nocapture`: passed.

Decision:

- Keep launch planning in `capo-adapters`, while actual process execution remains owned by `capo-runtime`.
- Reuse the launch-plan shape for both smoke tests and future controller dispatch so Codex/Claude do not grow separate command-construction paths.
- AC9 does not clear AC1 or AC3. Real subscription-backed execution remains gated on explicit user opt-in and artifact/state scanning.

### AC10 - Controller Dispatch Planning

Status: completed

Acceptance:

- Add a controller-owned path that resolves an agent plus Codex/Claude adapter selection into a local runtime launch plan without executing provider CLIs.
- Expose an operator command to inspect the planned dispatch contract without rendering the raw prompt.
- Preserve runtime ownership: the controller may plan a runtime request, but `capo-runtime` still owns process execution.
- Do not create smoke workspaces/artifact directories or claim real-agent readiness.

Evidence:

- `FakeBoundaryController::plan_local_adapter_dispatch(...)` in `../../crates/capo-controller/src/lib.rs`.
- CLI `capo adapter plan-launch --adapter codex|claude --agent NAME --goal GOAL` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-controller local_adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.

Decision:

- Use `adapter plan-launch` as the first operator-visible bridge from Capo task intent to real-adapter runtime metadata.
- The command auto-registers the named agent if needed so operators can inspect a launch contract before real connector proof.
- The prompt is intentionally not rendered in command output. The output reports counts, policy, paths, and provider/runtime metadata only.
- AC10 still does not clear AC1 or AC3. Real local adapter execution remains blocked on explicit opt-in smoke evidence.

### AC11 - Durable Dispatch Plan Read Model

Status: completed

Acceptance:

- Persist adapter dispatch plans as Capo-owned events/projections when explicitly requested.
- Rebuild dispatch-plan read models from projection records.
- Expose recorded dispatch plans through the shared dashboard/query surface.
- Keep recorded plans prompt-redacted and honest that no provider CLI executed.

Evidence:

- `AdapterDispatchPlanProjection`, `EventKind::AdapterDispatchPlanned`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- `ProjectDashboard.adapter_dispatch_plans` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter plan-launch ... --record` and dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state adapter_dispatch_plan -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.

Decision:

- Make `plan-launch` non-mutating by default except for the existing safe agent registration behavior; require `--record` to persist the dispatch-plan projection.
- Store runtime metadata, provider kind, credential scope, redaction/env counts, and `provider_cli_executed=false`, but do not store or render the raw prompt.
- AC11 improves inspectability and restart/rebuild behavior for planned real-agent dispatch. It still does not satisfy AC1 or AC3 because no subscription-backed provider process is run.

### AC12 - Workpad Next Adapter Plan

Status: completed

Acceptance:

- Compose indexed workpad next-task selection with Codex/Claude adapter dispatch planning.
- Allow operators to record a prompt-redacted dispatch plan for the next actionable observed-only workpad task.
- Preserve markdown/workpad source truth: planning must not import, start, or mark the selected workpad task active.
- Keep provider execution blocked and explicit: no provider CLI launch, no runtime artifact directory creation, and no real-agent readiness claim.

Evidence:

- CLI `capo workpad plan-next --agent NAME --adapter codex|claude [--path PATH] [--record]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Add `workpad plan-next` as the dogfood bridge from Capo's markdown task queue into real-adapter dispatch planning.
- Reuse the same dispatch-plan projection and prompt-redaction rules as `adapter plan-launch --record`.
- Planning the next workpad task records adapter intent only. `workpad import`, `workpad start-next`, and future real adapter execution remain separate explicit mutation surfaces.

### AC13 - Dispatch Execution Gate

Status: completed

Acceptance:

- Add a read-only execution gate for recorded adapter dispatch plans.
- Block provider CLI execution until the shared real-agent dogfood gate is cleared by recorded Codex smoke evidence.
- Require the selected dispatch plan to remain planned, prompt-redacted, and not already executed.
- Do not execute provider CLIs, create runtime artifact directories, or claim that the real smoke has run.

Evidence:

- CLI `capo adapter dispatch-gate --dispatch-plan DISPATCH_PLAN_ID` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Keep real execution behind an explicit read-only gate before adding any command that can invoke a subscription-backed provider CLI.
- Reuse the shared `AdapterDogfoodGate` from `capo-query` so dashboard, dogfood checks, and dispatch gating agree on the same recorded-evidence rule.
- A recorded dispatch plan becomes execution-eligible only when Codex real-smoke evidence is recorded as passed, clean, marker-confirmed, and `real_agent_connector_proven`; otherwise the command reports blocked reasons without mutating state.

### AC14 - Dispatch Gate Audit Trail

Status: completed

Acceptance:

- Allow operators to persist dispatch-gate decisions as Capo-owned audit records.
- Rebuild dispatch-gate read models from projection records after restart.
- Expose recorded gate decisions through the shared dashboard/query surface.
- Keep records prompt-redacted and explicit that provider CLIs were not executed.

Evidence:

- `AdapterDispatchGateProjection`, `EventKind::AdapterDispatchGateChecked`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- `ProjectDashboard.adapter_dispatch_gates` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter dispatch-gate ... --record` and dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat gate checks as separate audit facts from dispatch plans. A dispatch plan records intent; a dispatch gate records whether execution would be allowed at a point in time.
- Store only plan ID, adapter kind, gate status, reason codes, prompt policy, and `provider_cli_executed=false`. Do not store or render the raw prompt.
- Recording a gate does not launch provider CLIs, create runtime artifact directories, or transition a dispatch plan into execution.

### AC15 - Dispatch Fixture Replay

Status: completed

Acceptance:

- Compose a recorded dispatch plan, a recorded ready dispatch gate, and deterministic adapter fixture replay.
- Refuse replay when the selected dispatch plan has no recorded ready gate.
- Route fixture events through controller/state/evidence without launching provider CLIs.
- Keep raw prompt text and raw provider fixture text out of CLI output, state, and evidence exports.

Evidence:

- CLI `capo adapter replay-dispatch --dispatch-plan DISPATCH_PLAN_ID --fixture PATH [--out DIR]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Add fixture-only dispatch replay as a deterministic pre-real-execution scaffold. It proves the shape from planned dispatch -> gate -> adapter events -> controller/state/evidence while preserving the opt-in boundary for real provider CLIs.
- Require a recorded ready dispatch gate before replay so this path cannot bypass the same readiness contract that future provider execution will use.
- Replay uses fixture parsing and fake controller session plumbing only. It records `provider_cli_executed=false` and does not create the planned runtime workspace or artifact root.

### AC16 - Dispatch Replay Read Model

Status: completed

Acceptance:

- Persist dispatch fixture replay outcomes as Capo-owned read models.
- Rebuild replay rows from projection records after restart.
- Expose replay rows through the shared dashboard/query surface.
- Track counts, fixture hash, session/run refs, and raw-content policy without storing raw prompt or raw provider fixture text.

Evidence:

- `AdapterDispatchReplayProjection`, `EventKind::AdapterDispatchReplayed`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- `ProjectDashboard.adapter_dispatch_replays` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter replay-dispatch ...` records replay rows and dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state adapter_dispatch_replay -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat dispatch replay as a separate audit/result fact from dispatch plans and gate checks. Plans record intent, gates record execution permission, and replays record deterministic adapter-event ingestion results.
- Store fixture hash, fixture path, event counts, session/run refs, and `raw_content_policy=content_hashed_not_rendered`; keep raw provider text and raw dispatch prompts out of read models and dashboard output.
- Replays remain non-provider execution evidence: `provider_cli_executed=false`, no vendor CLI launch, and no planned runtime workspace/artifact root creation.

### AC17 - Dispatch Chain Status

Status: completed

Acceptance:

- Add a read-only operator command that summarizes a recorded dispatch plan, latest recorded gate, latest dispatch replay, and next action.
- Reuse shared query/dashboard read models rather than adding a second persistence path.
- Keep output prompt-redacted and fixture-content-redacted.
- Do not execute provider CLIs, create runtime artifact directories, or mutate dispatch state.

Evidence:

- CLI `capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Use `dispatch-status` as the operator introspection surface for the plan -> gate -> replay chain. It reports plan metadata, dogfood gate status, latest gate status/reasons, latest replay counts/raw-content policy, and the next safe action.
- Keep the command read-only over `ProjectDashboard` so CLI, future dashboard, voice, and mobile surfaces can share the same state contract.
- The command deliberately does not render raw dispatch prompts or raw fixture/provider text, and it preserves `provider_cli_executed=false` until a future explicit provider-running command exists.

### AC18 - Dispatch Execution Request Audit

Status: completed

Acceptance:

- Add an operator command that records a request to cross from a recorded dispatch plan/gate into real provider execution.
- Keep the request fail-closed unless a latest recorded ready dispatch gate exists.
- Persist execution-request rows as separate audit facts instead of overloading plans, gates, or fixture replays.
- Keep provider CLIs unexecuted in this slice and require a future explicit opt-in env for actual execution.

Evidence:

- CLI `capo adapter execution-request --dispatch-plan DISPATCH_PLAN_ID [--record]` in `../../crates/capo-cli/src/main.rs`.
- `AdapterDispatchExecutionRequestProjection`, `EventKind::AdapterDispatchExecutionRequested`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- Shared dashboard query exposure in `../../crates/capo-query/src/lib.rs` and CLI dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state adapter_dispatch_execution_request -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat execution requests as their own lifecycle fact: plans record intent, gates record whether execution would be allowed, replays record deterministic fixture ingestion, and execution requests record an operator request to cross the provider boundary.
- The command records `provider_cli_executed=false` and `status=waiting_on_explicit_provider_opt_in` when the plan has a ready gate. Without a ready gate, it records a blocked request with the gate reason or `recorded_ready_dispatch_gate_missing`.
- Actual provider execution remains deferred until the user explicitly authorizes the relevant opt-in environment variable, currently `CAPO_RUN_CODEX_LOCAL_DISPATCH` or `CAPO_RUN_CLAUDE_LOCAL_DISPATCH`.
