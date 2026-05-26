# Agent Connectors Feature

## Objective

Prove that Capo can safely dispatch work to real local subscription-backed coding agents, starting with Codex and then Claude Code, without reading or persisting credential material.

Priority: highest feature-phase blocker. Real local Codex/Claude connector proof is required before Capo can honestly claim first real-agent dogfood readiness or move remote-runtime semantics beyond contract/stub behavior.

Authorization: explicitly approved by the user on 2026-05-26 for Capo to work with the user's local Codex / ChatGPT Pro subscription and Claude Code subscription for the gated local smoke and real-dispatch proof paths. This approval does not relax the credential boundary: Capo must still avoid reading or persisting credential/session material, keep restrictive launch defaults, scan artifacts/state, and record evidence before clearing dogfood gates.

## Prototype Inputs

- P6 parsed Codex, Claude Code, and ACP fixtures into normalized adapter events.
- P7 built restrictive local smoke plans and artifact scanning, but real provider smoke tests were not run.
- P12 proved the controller/evidence path with fake agents.

## Dependencies

- Use `LocalProcessRunner`; do not spawn provider CLIs directly from adapter code.
- Keep subscription connectors local-only and user-owned.
- Preserve read-model ownership: provider streams are adapter inputs, not controller truth.
- Add a deterministic mock-agent path before broadening real-provider coverage. The mock should exercise the same adapter/controller/runtime surfaces as real agents where possible, so agent interaction tests can be scripted without subscription CLIs.

## Tasks

### AC1 - Codex Opt-In Smoke

Status: approved_pending_execution

Acceptance:

- Run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`; explicit user opt-in was granted on 2026-05-26.
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

- Prior skipped verification was due to missing opt-in. That blocker is removed as of 2026-05-26; the remaining work is to run the smoke safely, diagnose any environment/CLI blockers, scan artifacts, and record passed/failed/skipped evidence.

Decision:

- The Codex connector is high priority and approved for real local proof, but is not yet safe enough for first dogfood until a clean subscription-backed smoke report is recorded. The harness shape remains appropriate: local process runtime, isolated temporary workspace, read-only sandbox, ephemeral mode, ignored user config/rules, bounded/redacted artifacts, and marker scanning.

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

- Claude Code restricted-argument compatibility is ready for an opt-in smoke. User authorization was granted on 2026-05-26 for the local Claude Code subscription-backed proof path; the smoke remains gated behind `CAPO_RUN_CLAUDE_LOCAL_SMOKE=1` and must still preserve the credential/session boundary and artifact scanning rules.

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

- A real local Codex or Claude adapter process has still not produced accepted proof. User opt-in was granted on 2026-05-26, so this is now an execution/evidence task rather than an authorization blocker.

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

### AC19 - Dispatch Prompt Source Contract

Status: completed

Acceptance:

- Record prompt-source metadata for dispatch plans without storing raw prompt text.
- Distinguish non-replayable inline CLI prompts from workpad-derived prompts that can be materialized only if the source hash still matches.
- Expose prompt-source rows through shared query/dashboard surfaces for future runner and operator inspection.
- Keep prompt source recording separate from provider CLI execution and runtime artifact creation.

Evidence:

- `AdapterDispatchPromptSourceProjection`, `EventKind::AdapterDispatchPromptSourceRecorded`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- Shared dashboard query exposure in `../../crates/capo-query/src/lib.rs` and CLI dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `capo adapter plan-launch --record` records `source_kind=inline_cli_prompt` with `materialization_status=manual_prompt_not_replayable`.
- `capo workpad plan-next --record` records `source_kind=workpad_task` with `materialization_status=replayable_if_source_hash_matches`.
- `cargo test -p capo-state adapter_dispatch_prompt_source -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Preserve the raw prompt non-retention rule while still giving future real execution a materialization contract.
- Inline CLI prompt dispatch plans are intentionally not replayable after recording because Capo does not keep the raw prompt.
- Workpad-derived dispatch plans keep source path/anchor and source hash so a future runner can rederive the prompt from markdown only when the indexed source has not drifted.

### AC20 - Dispatch Prompt Materialization Dry Run

Status: completed

Acceptance:

- Add a provider-free command that checks whether a recorded dispatch prompt can be materialized without rendering the raw prompt.
- Refuse inline CLI prompt history because Capo does not retain the raw prompt.
- For workpad-derived prompt sources, require matching source hash and matching derived prompt hash before reporting readiness.
- Persist materialization results as separate audit/read-model rows with hashes, status, raw prompt policy, and reasons only.

Evidence:

- CLI `capo adapter materialize-prompt --dispatch-plan DISPATCH_PLAN_ID [--record]` in `../../crates/capo-cli/src/main.rs`.
- `AdapterDispatchPromptMaterializationProjection`, `EventKind::AdapterDispatchPromptMaterialized`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- Shared dashboard query exposure in `../../crates/capo-query/src/lib.rs` and CLI dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-state adapter_dispatch_prompt_materialization -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Treat prompt materialization as a dry-run audit fact before any provider execution. This lets Capo prove whether the future runner can build the prompt from source without exposing the prompt in CLI output, state, or dashboard.
- Inline CLI plans materialize to `blocked_non_replayable_prompt`.
- Workpad plans materialize to `ready_without_rendering_prompt` only when the current indexed source hash and derived prompt hash match the recorded prompt source.

### AC21 - Real Dispatch Runner Preflight

Status: completed

Acceptance:

- Add a provider-free command that composes dispatch plan, execution request, prompt materialization, and explicit provider opt-in into one runner preflight.
- Keep provider CLIs unexecuted; the command must not create runtime workspaces or artifact roots.
- Report why real execution is blocked when execution request, prompt materialization, or opt-in evidence is missing.
- For workpad-derived ready prompts, report the remaining explicit opt-in blocker without rendering the prompt.

Evidence:

- CLI `capo adapter run-preflight --dispatch-plan DISPATCH_PLAN_ID` in `../../crates/capo-cli/src/main.rs`.
- Inline CLI prompt dispatch preflight blocks on `blocked_prompt_materialization_not_ready`.
- Workpad-derived dispatch preflight with ready gate, execution request, and prompt materialization blocks on `blocked_missing_explicit_provider_opt_in` until `CAPO_RUN_CODEX_LOCAL_DISPATCH=1`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Treat `run-preflight` as the final provider-free seam before a future runtime-running command. It proves all recorded Capo facts are aligned and names the exact opt-in env required for a real provider boundary crossing.
- Do not execute provider CLIs or call `LocalProcessRunner` in AC21. Actual execution remains gated behind explicit user opt-in and a future command that consumes this preflight.

### AC22 - Guarded Local Dispatch Runner Surface

Status: completed

Acceptance:

- Add a runner command that consumes the recorded run preflight and fails closed unless every preflight fact allows provider execution.
- Keep provider CLIs unexecuted in normal tests and unless the adapter-specific opt-in env is explicitly set.
- When execution is allowed, reconstruct workpad-derived prompts from source, call `LocalProcessRunner`, capture bounded/redacted artifacts, scan artifacts for credential/session markers, and delete captured artifacts on failed marker scans.
- Do not support inline CLI prompt execution because Capo intentionally does not retain the raw prompt.

Evidence:

- CLI `capo adapter run-local --dispatch-plan DISPATCH_PLAN_ID` in `../../crates/capo-cli/src/main.rs`.
- Missing prompt materialization returns `status=blocked_missing_prompt_materialization` and `provider_cli_executed=false`.
- The shared preflight for ready workpad prompt materialization still returns `status=blocked_missing_explicit_provider_opt_in` and `provider_cli_executed=false` until `CAPO_RUN_CODEX_LOCAL_DISPATCH=1`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- Regression coverage deletes stdout/stderr artifacts when the post-run marker scan fails.

Decision:

- `run-local` is the first real provider-boundary command surface, but it is fail-closed by default. It only reaches `LocalProcessRunner` after dispatch plan, execution request, prompt materialization, dogfood gate, and explicit opt-in agree.
- The runner reconstructs only workpad-derived prompts whose source and prompt hashes already matched in prompt materialization. Inline CLI dispatch plans remain non-replayable.
- Execution outcomes are currently returned by the command with runtime artifact refs. A follow-up should persist provider execution outcomes as their own read model before claiming AC3 real-agent controller completion.

Review:

- Focused provider-safety review found one blocker: a failed credential marker scan could leave captured runtime artifacts on disk. The runner path now deletes captured stdout/stderr artifacts before returning the scan error.
- The review also noted that `run-local` recomputes preflight from recorded facts rather than requiring a separately persisted preflight row. This is accepted for AC22 because there is no preflight projection yet and the command still requires the same recorded plan/request/materialization facts plus explicit opt-in.

### AC23 - Dispatch Execution Outcome Read Model

Status: completed

Acceptance:

- Persist local dispatch execution outcomes as separate Capo-owned audit/read-model rows.
- Expose execution outcomes through the shared query/dashboard surface.
- Let `run-local --record` record blocked preflight outcomes without launching provider CLIs.
- When future opt-in execution succeeds, record runtime process ref, stdout/stderr artifact refs, exit code, credential scan status, raw prompt policy, and raw output policy without raw prompt/provider text.

Evidence:

- `AdapterDispatchExecutionProjection`, `EventKind::AdapterDispatchExecuted`, SQLite table, rebuild codec, and read query in `../../crates/capo-state/src/lib.rs`.
- Shared dashboard query exposure in `../../crates/capo-query/src/lib.rs` and CLI dashboard rendering in `../../crates/capo-cli/src/main.rs`.
- `capo adapter run-local --dispatch-plan DISPATCH_PLAN_ID --record` records blocked preflight outcomes with `provider_cli_executed=false`.
- `cargo test -p capo-state adapter_dispatch_execution -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat local dispatch executions as a fourth durable fact after plans, gates, and execution requests. This keeps real provider side effects auditable instead of only returning transient CLI output.
- Blocked execution outcomes are useful state: they let dashboard/voice/mobile surfaces explain why execution did not happen without re-running the command.
- Successful execution outcome recording is wired into `run-local`, but real Codex/Claude execution remains skipped until explicit opt-in.

### AC24 - Dispatch Status Execution Introspection

Status: completed

Acceptance:

- Add latest dispatch execution outcome visibility to `capo adapter dispatch-status`.
- Keep `dispatch-status` read-only over shared query/read-model state.
- Report blocked execution reasons and successful execution artifact refs without rendering raw prompts or provider output.
- Preserve provider execution gating; status inspection must not launch provider CLIs or create runtime artifacts.

Evidence:

- `capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID` now renders latest execution ID, status, provider execution flags, credential scan status, stdout/stderr artifact refs, and reason codes from `ProjectDashboard.adapter_dispatch_executions`.
- Blocked `run-local --record` outcomes change the dispatch next action to `resolve_latest_execution_blocker` until fixture replay or future successful execution supersedes the operator action.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat `dispatch-status` as the compact operator surface for the full dispatch chain: plan, gate, replay, and execution outcome.
- Execution outcome introspection is read-only and query-derived. It does not recompute preflight, rematerialize prompts, or cross the provider boundary.

### AC25 - Dispatch Chain Evidence Export

Status: completed

Acceptance:

- Add a provider-free command that exports a prompt-redacted dispatch-chain evidence artifact.
- Include plan, latest gate, latest fixture replay, latest local execution outcome, and dogfood gate status.
- Record the exported markdown as a Capo artifact plus evidence projection linked to the dispatch session/run.
- Refuse to render raw prompts, raw provider fixture text, or raw provider output.

Evidence:

- CLI `capo adapter dispatch-evidence --dispatch-plan DISPATCH_PLAN_ID --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Exported artifacts use the `<!-- capo:adapter-dispatch-evidence -->` marker and guarded overwrite behavior.
- Artifact records use `kind=adapter_dispatch_evidence`; evidence projections use `kind=adapter_dispatch_evidence`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat dispatch-chain evidence as a review artifact, not a new execution step. The command reads existing projections and writes only Capo-owned evidence metadata/artifacts.
- Use artifact IDs and policy fields instead of raw output in the report. Runtime stdout/stderr remain referenced by artifact ID only.
- This prepares the real opt-in execution path for review without weakening the provider boundary.

### AC26 - Dispatch Status Query Contract

Status: completed

Acceptance:

- Move dispatch-status plan/gate/replay/execution summary assembly into the shared query surface.
- Keep CLI output stable while making the same dispatch-chain status available to future voice, web, mobile, and TUI consumers.
- Preserve prompt/output redaction: status fields expose metadata, artifact IDs, booleans, counts, and next action only.
- Do not run provider CLIs, materialize prompts, open tunnels, or inspect credentials.

Evidence:

- `ProjectDashboard::adapter_dispatch_status(...)` and `AdapterDispatchStatus` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter dispatch-status` now renders the shared query summary in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query adapter_dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat dispatch-chain status as a read-model query contract, not CLI glue. The CLI still owns text formatting, but the cross-row selection and next-action derivation live in `capo-query`.
- Keep dispatch evidence export on its existing projection inputs for now; it produces a markdown artifact and has broader rendering needs than the one-line status contract.

### AC27 - Latest Dispatch Status Selection

Status: completed

Acceptance:

- Add a shared query helper that selects the latest recorded dispatch-chain status without requiring the operator to know a dispatch-plan ID.
- Support an optional agent-name filter so operators can ask for the latest dispatch status for a specific agent.
- Expose the selector through the existing `dispatch-status` operator surface without mutating state.
- Preserve prompt/output redaction and avoid provider CLI execution, prompt materialization, tunnel changes, or credential inspection.

Evidence:

- `ProjectDashboard::latest_adapter_dispatch_status(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter dispatch-status --latest [--agent NAME]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query latest_adapter_dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat latest dispatch selection as query/read-model behavior. It chooses the dispatch plan with the latest plan/gate/replay/execution activity and then renders the same `AdapterDispatchStatus` contract as ID-based lookup.
- Keep `--agent` scoped to `--latest` so ID-based lookup remains unambiguous.

### AC28 - Latest Dispatch Evidence Export

Status: completed

Acceptance:

- Add a provider-free latest-selector path for dispatch evidence export.
- Support optional agent-name filtering so operators can export latest dispatch-chain evidence for a specific agent.
- Reuse the prompt-redacted dispatch evidence artifact format and guarded writer.
- Do not run provider CLIs, materialize prompts, open tunnels, or inspect credentials.

Evidence:

- CLI `capo adapter dispatch-evidence --latest [--agent NAME] --out DIR` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat latest evidence export as operator ergonomics over the shared dispatch status selector. It selects the latest plan through `ProjectDashboard::latest_adapter_dispatch_status(...)`, then writes the same Capo-owned prompt-redacted evidence artifact as exact plan export.
- Keep exact `--dispatch-plan` and latest `--latest` mutually exclusive; `--agent` is only valid with `--latest`.

### AC29 - Dispatch Tool Observation Evidence

Status: completed

Acceptance:

- Include observed-only native tool observations in dispatch-chain evidence exports after fixture replay records them.
- Preserve the distinction between governed Capo tool calls and adapter/provider-native observed-only tool activity.
- Do not render raw dispatch prompts, raw provider fixture text, or raw provider tool input/output.
- Cover dispatch evidence export with regression assertions for observed tool metadata.

Evidence:

- CLI `capo adapter dispatch-evidence --dispatch-plan DISPATCH_PLAN_ID --out DIR` now includes an `Observed Tool Activity` section sourced from `tool_observations_for_session`.
- CLI `capo adapter dispatch-evidence --latest [--agent NAME] --out DIR` reuses the same observed-tool evidence rendering after latest dispatch selection.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat observed native tool activity as dispatch-chain review evidence when fixture replay records structured tool observations.
- Keep observed-only native tools separate from governed Capo tool calls. The evidence artifact renders observation ID, tool name, adapter-event source, observed status, instrumentation level, confidence, external ref, artifact ref, and raw-event hash only.
- Continue excluding raw dispatch prompts, raw provider fixture text, and raw provider tool input/output from dispatch evidence artifacts.

### AC30 - Adapter Smoke Report Evidence Export

Status: completed

Acceptance:

- Add a provider-free command that exports a Capo-owned evidence artifact for a recorded adapter smoke report.
- Record the exported markdown as project-level evidence so readiness and dogfood reviews can cite connector proof or blocker records.
- Use guarded overwrite behavior so Capo does not overwrite user-authored files.
- Do not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, request approvals, activate grants, or render smoke stdout/stderr content.

Evidence:

- `ProjectDashboard::adapter_smoke_report_status(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter smoke-report evidence --smoke-report SMOKE_REPORT_ID --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Exported artifacts use the `<!-- capo:adapter-smoke-evidence -->` marker and guarded overwrite behavior.
- Artifact records use `kind=adapter_smoke_evidence`; evidence projections use `kind=adapter_smoke_evidence`.
- `cargo test -p capo-query adapter_smoke_report -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat smoke-report evidence as a connector-readiness review artifact, not a provider execution step.
- Render smoke report metadata and artifact-root references only. Do not render smoke stdout/stderr content, raw prompts, provider output, tokens, cookies, or subscription session material.
- Keep skipped and failed reports exportable because they explain why real-agent dogfood remains blocked.

### AC31 - Adapter Smoke Report Status Query

Status: completed

Acceptance:

- Add shared query helpers for exact and latest adapter smoke report status.
- Expose exact and latest smoke report status through a read-only operator command.
- Support adapter filtering for latest smoke report selection.
- Do not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, request approvals, activate grants, render smoke stdout/stderr content, or mutate state.

Evidence:

- `ProjectDashboard::adapter_smoke_report_status(...)` and `ProjectDashboard::latest_adapter_smoke_report(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo adapter smoke-report status --smoke-report SMOKE_REPORT_ID` in `../../crates/capo-cli/src/main.rs`.
- CLI `capo adapter smoke-report status --latest [--adapter codex|claude]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query adapter_smoke_report -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Decision:

- Treat adapter smoke report status as shared query/read-model behavior so CLI, dashboard, voice, web, and mobile surfaces can answer connector readiness questions from the same selector.
- Keep exact ID lookup and latest lookup separate. Adapter filtering is only valid for latest lookup.
- Render metadata only: status, credential scan status, marker flag, artifact-root reference, dogfood readiness effect, reason, and no-side-effect markers.

### AC32 - Latest Adapter Smoke Evidence Export

Status: completed

Acceptance:

- Add a provider-free latest-selector path for adapter smoke-report evidence export.
- Support the same optional adapter filter as latest smoke-report status.
- Reuse the Capo-marked adapter smoke evidence artifact format and guarded writer.
- Do not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, request approvals, activate grants, render smoke stdout/stderr content, or mutate connector state beyond recording the evidence artifact.

Evidence:

- CLI `capo adapter smoke-report evidence --latest [--adapter codex|claude] --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Latest export reuses `ProjectDashboard::latest_adapter_smoke_report(...)` and the existing Capo-marked smoke evidence renderer/writer.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.

Decision:

- Treat latest smoke evidence export as operator ergonomics over the shared adapter smoke selector. It selects the latest matching smoke report, then writes the same Capo-owned connector-readiness evidence artifact as exact export.
- Keep exact `--smoke-report` and latest `--latest` mutually exclusive; `--adapter` is only valid with `--latest`.
- Keep the export read-model-derived and provider-free. It records a project evidence row, but does not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, approvals, grants, raw prompt/output rendering, or connector-state mutation.

### AC33 - Adapter Dogfood Gate Evidence Export

Status: completed

Acceptance:

- Add a provider-free command that exports a Capo-owned evidence artifact for the adapter dogfood gate.
- Include required, proven, and blocked adapters plus connector smoke-report refs.
- Record the exported markdown as project-level evidence so dashboard/dogfood reviews can cite the first real-agent readiness gate.
- Use guarded overwrite behavior so Capo does not overwrite user-authored files.
- Do not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, request approvals, activate grants, render smoke stdout/stderr content, or mutate connector state beyond recording the evidence artifact.

Evidence:

- CLI `capo adapter dogfood-gate evidence --out DIR` in `../../crates/capo-cli/src/main.rs`.
- Exported artifacts use the `<!-- capo:adapter-dogfood-gate-evidence -->` marker and guarded overwrite behavior.
- Artifact records use `kind=adapter_dogfood_gate_evidence`; evidence projections use `kind=adapter_dogfood_gate_evidence`.
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

Decision:

- Treat adapter dogfood gate evidence as the connector-level checkpoint before broader `dogfood readiness`.
- Keep this report narrower than dogfood readiness: it answers only whether recorded connector smoke evidence clears the first real-agent dogfood gate.
- Keep smoke report refs metadata-only. The report does not render smoke stdout/stderr, raw prompts, provider output, tokens, cookies, subscription sessions, or credential material.

### AC34 - Scriptable Mock Agent Harness

Status: completed

Acceptance:

- Add a reusable mock agent for deterministic tests of agent interaction flows: start/session, prompt/response, streamed updates, tool requests, permission requests, interruptions, failures, and completion.
- Drive the mock through Capo's existing adapter/controller/runtime boundaries instead of adding test-only controller shortcuts. Prefer the static-dispatch `FakeAdapter`/fake provider path first, and include an ACP-shaped mock path if that is the cleaner way to exercise protocol behavior.
- Let tests script responses and state transitions explicitly, similar in spirit to `../aget` mock site/tool tests, so each test can say exactly how the agent responds.
- Keep the mock provider-free and credential-free. It must not launch Codex, Claude, local model servers, tunnels, or external services.
- Use the mock in at least one end-to-end controller/CLI test that proves deterministic multi-turn interaction and evidence/read-model updates without relying on fixed Codex/Claude fixture files.

Evidence:

- Reference pattern: `../aget/tests/support/mock_site.rs`, `../aget/tests/support/mock_tools.rs`, and `../aget/tests/fixtures/mock-tools/`.
- Existing Capo boundaries to reuse: `AgentAdapter::Fake`, `FakeBoundaryController`, normalized adapter events, ACP fixture replay, and static dispatch boundary enums.
- Scripted mock agent implementation: `../../crates/capo-adapters/src/scripted_mock_agent.rs`.
- Controller application path: `FakeBoundaryController::apply_scripted_mock_turn` in `../../crates/capo-controller/src/lib.rs`.
- `cargo test -p capo-adapters scripted_mock_agent -- --nocapture`: passed.
- `cargo test -p capo-controller scripted_mock_agent_drives_multi_turn_controller_state -- --nocapture`: passed.
- New tests run without opt-in env vars or provider subscriptions.
- Focused AC34 review found three medium gaps: unprojected permission/failure/interruption events, bypassed adapter enum dispatch, and duplicate streaming delta idempotency collisions. All three were fixed before completion.

Decision:

- Treat this as a testing architecture task, not a substitute for real local connector proof. The mock proves Capo interaction semantics deterministically; Codex/Claude smoke still proves real subscription-backed connector safety.
- Prefer scripted mock interactions over more golden JSONL files when testing controller behavior that depends on live turn sequencing, permission/tool branches, interruption, or failure handling.
- Scripted mock turns emit normalized adapter events with `adapter_kind=mock` and stable idempotency keys. The controller applies them through the static `AgentAdapter::ScriptedMock` variant and existing adapter replay pipeline, so tests exercise read-model, tool-observation, evidence, redirect, permission, failure, interruption, and interrupt behavior without launching a provider.
- The first AC34 test exposed and fixed a replay event-ID collision: controller adapter replay event IDs now include stable adapter event identity instead of only session plus local index. This matters for multi-turn streams and future ACP `session/load` replay.
- Repeated streaming deltas for the same item now include a stable event index in the mock timeline key, so deterministic reruns dedupe correctly while distinct deltas in the same turn remain visible.
