# Feature Knowledge

## Objective

Capture cross-feature decisions and sequencing until individual feature workpads exist.

Feature phase is ready after prototype gate P15.

## F0 - Split Feature Workpads

Status: completed on 2026-05-25.

Decisions:

- Split the post-prototype backlog by boundary rather than by UI milestone. This keeps real-agent connectors, workpad dogfood, dashboard/query, permissions/tools, memory/eval, voice, and remote runtime independently reviewable.
- The first feature priority should be either `agent-connectors.md` if the goal is real Codex/Claude execution, or `dogfood-bridge.md` if the goal is importing Capo's own workpads before real-agent execution.
- Real local agent execution remains the main product constraint from the prototype gate. Fake agents prove controller/state/evidence semantics, not useful coding output.
- Workpad import/update safety is the main dogfood bridge constraint. Evidence export is safe, but Capo cannot yet manage source workpad files directly.
- Dashboard and voice should share a reusable query surface before adding richer UI or conversational clients.

Follow-up:

- `agent-connectors.md` should start with Codex opt-in smoke because Codex is already wired through restrictive smoke-plan code.
- `dogfood-bridge.md` should preserve the source-of-truth distinction between markdown task status and Capo execution status.

## F2/DB1 - Workpad Index

Status: completed on 2026-05-25.

Decisions:

- Start the dogfood bridge with read-only indexing rather than task execution. Capo can now observe the project workpad tree without mutating source markdown.
- Add `capo-workpads` for markdown scanning and task-status extraction. The crate has no third-party dependencies and deliberately writes nothing.
- Persist workpad observations through SQLite projections:
  - `workpad_files`: path, project, content hash, headings, objective text, observed timestamp, update sequence.
  - `workpad_tasks`: source task ID, project, path, source anchor, title, observed markdown status, Capo execution status, observed timestamp, update sequence.
- Use `observed_only` as the initial Capo execution status for imported markdown tasks. DB2 will decide how selected observed tasks become executable Capo tasks.
- Add `capo workpad index --root PATH --state PATH` as the first non-destructive CLI path for dogfood import.
- After review, constrain the scanner to selected Capo workpad docs rather than recursive `workpads/**` indexing. This prevents prior-art clones and reference repos from becoming Capo task refs.
- Add mixed-case task ID support for headings like `A2a`, `A5a`, and `R2a`.
- Add a reset projection at the start of each workpad index batch so rebuild and re-index remove stale file/task refs.

Verification:

- `cargo test -p capo-workpads`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.
- Manual repo smoke `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` reported `files=43`, `tasks=98` after scoping fixes.

Review:

- Focused review initially blocked DB1 on over-indexing, missed mixed-case task IDs, and stale projections. The fixes above were applied before completion.

Follow-up:

- DB2 should map selected `workpad_tasks` into Capo task records while preserving markdown status as observed source truth.
- DB3 should add reviewed update/evidence proposal artifacts before Capo can apply any changes to source workpads.

## F2/DB2 - Capo Task Import

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad import --workpad-task WORKPAD_TASK_ID` to convert a selected observed workpad task into a normal Capo task read model.
- Default imported task IDs are deterministic from the workpad task ID, with an optional `--task TASK_ID` override for operators.
- Preserve the data boundary:
  - `observed_status` remains the markdown status observed from the source file.
  - `workpad_tasks.capo_execution_status=imported` means Capo has imported that source task.
  - `tasks.capo_execution_status=ready` means the Capo task record is ready for later orchestration.
- Store source path, heading anchor, source hash, observed status, and workpad task ID in the import event payload and task summary until DB3 adds Capo-owned reviewed artifacts.
- Use project-scoped idempotency keys for imports based on task ID, workpad task ID, and source hash so repeated imports of the same observed source do not duplicate events.
- Preserve imported workpad execution status across no-change re-indexes. Re-index still removes stale workpad task refs when the markdown source task disappears, and later restores them if the source content recurs.
- Do not use project-scoped idempotency keys for workpad index events. The projection reset must reapply every observation so A-B-A source changes cannot leave current read models stale.
- Refuse `--task` imports that would overwrite an existing unrelated, active, or session-linked Capo task read model.
- Use `--expected-hash` for optimistic source drift checks. Imports with a stale expected hash fail before writing Capo task state.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.

Review:

- Focused review found two blockers in the first draft: source-fingerprint recurrence could leave stale read models, and imports could clobber existing Capo task state. Both were fixed with regression coverage.

Follow-up:

- DB3 should replace ad hoc source metadata in task summaries with Capo-owned proposal/evidence artifacts.
- Dashboard/query work should expose imported workpad task refs without forcing consumers to parse task summaries.

## F2/DB3 - Reviewed Workpad Artifacts

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR` as the first safe write-adjacent dogfood command.
- Proposal artifacts are Capo-owned markdown files marked with `<!-- capo:workpad-proposal -->`. They include source path, source anchor, source hash, observed markdown status, Capo execution status, proposed update text, apply policy, and rollback/fallback instructions.
- Proposal generation records a safe `workpad_update_proposal` artifact row and evidence projection, but does not edit source workpad markdown.
- `capo workpad apply --proposal PATH --confirm` exists as the explicit confirmation surface, but DB3 intentionally leaves source writeback disabled. Confirmed apply reports `workpad_apply_supported=false` and `source_modified=false`.
- Proposal artifact identity includes task ID, workpad task ID, source hash, and proposal text. Different proposal bodies get different artifact files.
- Existing non-Capo files are never overwritten. Existing changed Capo proposal files are also not overwritten, so human review notes cannot be silently erased.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Manual smoke against this repo with temporary state/output dirs: `workpad index`, `workpad import`, `workpad propose`: passed.

Review:

- Focused review found one blocker in the first draft: repeated proposal writes with different bodies could overwrite the artifact while event idempotency no-opped. The fix was to include proposal text in artifact identity and refuse changed Capo proposal overwrites.

Follow-up:

- Future source writeback should validate source hash at apply time, generate a patch/diff artifact first, and keep a rollback artifact before modifying markdown.
- Dashboard/query work should expose proposal artifact/evidence refs directly instead of making users parse CLI output.

## F2/DB4 - Next Workpad Selection

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad next [--path PATH]` as a read-only dogfood operator command over indexed workpad task read models.
- Prefer observed markdown `in_progress` before `pending`, `ready`, and `waiting_on_opt_in`, preserving markdown as the source of task state.
- Select only `capo_execution_status=observed_only` workpad refs. Imported workpad tasks are already represented as Capo task read models and should not be re-selected as import candidates.
- Return the deterministic default Capo task ID for the selected workpad task, but do not import it automatically. Import remains an explicit operator action.
- Allow path scoping so Capo can answer "what is next in this workpad?" without changing global phase routing.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- A future dogfood command can compose `workpad next`, `workpad import`, and agent dispatch, but it should stay explicit until real connector proof is recorded.

## F2/DB5 - Start Next Workpad Task

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad start-next --agent NAME [--path PATH]` as the first explicit dogfood bridge from indexed markdown work into controller execution.
- The command composes DB4 next selection, DB2 task import, and the existing controller send-task path. It does not edit source markdown.
- Preserve the imported workpad task ID as the controller task ID by allowing `CommandIntent::SendTask` envelopes to carry an optional structured `task_id`. Existing `task send` calls still use goal-derived task IDs.
- Keep dispatch fake/local for now. Real Codex/Claude dispatch remains blocked on explicit opt-in smoke evidence.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- After real connector proof, `start-next` can grow an adapter/runtime selector instead of assuming the registered fake agent path.

## F1/AC1-AC2 - Local Connector Preflight

Status: in progress on 2026-05-25.

Decisions:

- Do not run the real Codex subscription-backed smoke without explicit user opt-in. The ignored test was executed without `CAPO_RUN_CODEX_LOCAL_SMOKE=1` and stayed inside the opt-in gate.
- Installed Codex is `codex-cli 0.133.0`; `codex exec --help` currently supports the planned safe smoke flags: JSONL output, read-only sandbox, ephemeral mode, ignored user config/rules, and isolated `--cd`.
- Installed Claude Code is `2.1.150`; its help currently supports the restricted noninteractive stream path.
- Tighten the Claude smoke plan with `--no-session-persistence`, `--disable-slash-commands`, and `--tools ""` in addition to plan permission mode, disallowed tools, empty MCP config, and strict MCP config.
- Treat Codex as still unproven for dogfood until the real opt-in smoke runs and artifact/state scans pass.
- Treat Claude restricted args as verified enough for a future opt-in smoke, but do not run Claude without explicit authorization.

Verification:

- `cargo test -p capo-adapters local_smoke_plan`: passed.
- `cargo test -p capo-adapters local_adapter_smoke_runner`: passed.
- `cargo test -p capo-adapters artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets`: passed.
- `cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` without opt-in: passed by skipping the provider process.

Review:

- Focused connector safety review found no blocking issues. It confirmed Codex opt-in gating is preserved, Claude restricted flags match current help, and F1 remains honestly in progress because real subscription-backed smoke has not run.

Skipped verification:

- Real Codex local smoke with `CAPO_RUN_CODEX_LOCAL_SMOKE=1` was not run because explicit opt-in is required.
- Real Claude local smoke with `CAPO_RUN_CLAUDE_LOCAL_SMOKE=1` was not run because this pass only verified restricted arguments.

Follow-up:

- After explicit user opt-in, run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`, inspect artifacts/state for credential markers, and decide whether Codex is safe enough for first dogfood.
- If Codex passes, AC3 should route a real adapter event stream through controller/state/evidence instead of stopping at adapter-level smoke.

## F1/AC7 - Dogfood Readiness Gate

Status: completed on 2026-05-25.

Decisions:

- Add a shared `AdapterDogfoodGate` in `capo-query` so operator surfaces consume one readiness rule instead of duplicating connector checks.
- Keep the first real-agent dogfood gate evidence-derived and read-only. It checks persisted smoke reports; it does not run provider CLIs, inspect subscription state, or read credentials.
- Require Codex proof for first dogfood: passed smoke report, clean credential scan, marker found, and `dogfood_readiness_effect=real_agent_connector_proven`.
- Keep Claude as a first-class target connector, but do not block first dogfood on Claude because AC1 defines Codex as the first local connector proof.

Verification:

- `cargo test -p capo-query adapter_dogfood -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

Follow-up:

- After explicit Codex opt-in smoke, record the resulting smoke report and re-run `capo adapter dogfood-gate` before moving any Capo work into real-agent dogfood.

## F1/AC8 - Smoke Artifact Scan Enforcement

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter smoke-report scan --artifact-root PATH` as a provider-free artifact scan command for local smoke outputs.
- Enforce the same scan before accepting any `passed` smoke report. A passed report now requires `--artifact-root`; skipped and failed reports remain recordable without artifacts so blockers can still be documented.
- Reuse the adapter-layer sensitive marker scanner so the scan policy remains shared between the actual smoke runner and the operator evidence contract.
- Keep `--credential-scan clean` in the command as an explicit operator-facing assertion, but make it insufficient on its own for passed reports.

Verification:

- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

Follow-up:

- After the opt-in Codex smoke runs, use the generated artifact root directly with the smoke report command rather than relying on manual inspection notes.

## F1/AC9 - Local Adapter Launch Contract

Status: completed on 2026-05-25.

Decisions:

- Add `LocalAdapterLaunchPlan` as the adapter-owned launch contract for local subscription-backed CLIs. It builds `LocalProcessConfig` and `LocalProcessRequest`, but does not execute provider processes.
- Keep process ownership in `capo-runtime`; adapters only construct launch metadata and normalized event parsers.
- Encode local subscription use as `credential_scope=user_local_subscription`. The launch plan does not store credential paths, keychain refs, tokens, cookies, or API keys.
- Preserve restrictive Codex defaults: JSONL output, read-only sandbox, ephemeral mode, ignored user config/rules, explicit workspace.
- Preserve restrictive Claude Code defaults: stream-json output, plan permission mode, no session persistence, disabled slash commands, no tools, disallowed tools, empty strict MCP config.
- Add `assert_subscription_safe` to fail closed on secret-like env allowlist entries or argv markers before a launch plan can be treated as safe.

Verification:

- `cargo test -p capo-adapters launch_plan -- --nocapture`: passed.
- `cargo test -p capo-adapters local_smoke_plan -- --nocapture`: passed.

Follow-up:

- Wire the launch plan into the controller dispatch path only after the opt-in real Codex smoke produces clean evidence.

## F1/AC10 - Controller Dispatch Planning

Status: completed on 2026-05-25.

Decisions:

- Add `FakeBoundaryController::plan_local_adapter_dispatch(...)` as the controller-owned bridge from agent intent to local Codex/Claude runtime launch metadata.
- Keep this path read-only with respect to provider execution. It resolves the registered agent, builds session/run IDs, validates the adapter launch plan, and returns runtime metadata, but it never calls `LocalProcessRunner`.
- Add `capo adapter plan-launch --adapter codex|claude --agent NAME --goal GOAL` so operators can inspect the planned dispatch contract before running any subscription-backed smoke.
- Do not render the raw prompt in CLI output. The output reports provider kind, credential scope, runtime program, arg counts, cwd/artifact paths, env/redaction counts, and `provider_cli_executed=false`.
- Let the command auto-register the named agent if missing. This keeps launch planning usable during dogfood setup while real dispatch remains gated.

Verification:

- `cargo test -p capo-controller local_adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.

Follow-up:

- After AC1 has clean opt-in Codex smoke evidence, the next controller slice can convert the launch plan into an audited runtime start and adapter stream consumption path.

## F1/AC11 - Durable Dispatch Plan Read Model

Status: completed on 2026-05-25.

Decisions:

- Add `AdapterDispatchPlanProjection` as the durable, prompt-redacted record of planned real-adapter dispatch.
- Add `capo adapter plan-launch --record` so operators can explicitly persist planned Codex/Claude launch metadata before any provider process runs.
- Expose dispatch plans through `ProjectDashboard.adapter_dispatch_plans` and the text dashboard. This makes planned real-agent work visible beside readiness, smoke reports, agents, sessions, and workpad tasks.
- Keep prompt text out of projection records and dashboard output. The projection stores `runtime_prompt_policy=not_rendered` and counts/policies instead.
- Keep `provider_cli_executed=false` in the projection so recorded plans cannot be mistaken for real adapter execution evidence.

Verification:

- `cargo test -p capo-state adapter_dispatch_plan -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.

Follow-up:

- A future real-dispatch slice should transition a recorded plan from `planned` into runtime start events only after AC1 records clean opt-in Codex smoke evidence.

## F1/AC12 - Workpad Next Adapter Plan

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad plan-next --agent NAME --adapter codex|claude` as the non-executing bridge from indexed markdown workpads into real-adapter dispatch planning.
- Use the same prompt-redacted dispatch-plan projection as `adapter plan-launch --record`; the raw generated goal is hashed for identity but not rendered or stored.
- Keep workpad source state unchanged. `plan-next` selects the next actionable `observed_only` workpad task but does not import it, start it, or update its Capo execution status.
- Keep provider execution unclaimed. The recorded dispatch plan says `provider_cli_executed=false` and does not create runtime artifact directories.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- After real Codex smoke evidence clears AC1, `workpad plan-next` can become the preview step before an explicit real-adapter `workpad start-next` variant.

## F1/AC13 - Dispatch Execution Gate

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter dispatch-gate --dispatch-plan DISPATCH_PLAN_ID` as the read-only gate between recorded dispatch intent and any future provider CLI execution command.
- Reuse the shared `AdapterDogfoodGate` from `capo-query`. A Codex dispatch plan is execution-eligible only after recorded smoke evidence proves `smoke_status=passed`, `credential_scan_status=clean`, marker present, and `dogfood_readiness_effect=real_agent_connector_proven`.
- Keep the gate fail-closed on plan-level invariants: dispatch plan status must still be `planned`, prompt policy must remain `not_rendered`, and `provider_cli_executed` must be false.
- Do not launch provider CLIs, create runtime artifact directories, or mutate dispatch-plan state in this slice. AC13 only reports whether execution would be allowed.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- A future execution command should call this gate before invoking `LocalProcessRunner`, then record a separate runtime-start/adapter-stream lifecycle rather than reusing the planned projection as execution evidence.

## F1/AC14 - Dispatch Gate Audit Trail

Status: completed on 2026-05-25.

Decisions:

- Add `AdapterDispatchGateProjection` as a durable audit record for dispatch-gate checks. Dispatch plans record intended runtime metadata; dispatch gates record whether that plan would be allowed to execute under current evidence.
- Add `capo adapter dispatch-gate --dispatch-plan DISPATCH_PLAN_ID --record` to persist the gate result without invoking a provider CLI.
- Expose recorded gate checks through `ProjectDashboard.adapter_dispatch_gates` and CLI dashboard rendering so future voice/web/mobile surfaces can inspect the same audit trail.
- Keep gate records prompt-redacted: store dispatch plan ID, adapter kind, readiness status, reason codes, prompt policy, and `provider_cli_executed=false`, not the raw goal.

Verification:

- `cargo test -p capo-state adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- The future provider-running command should append a separate execution-start projection only after a recorded ready gate and should preserve blocked gate records as denial/audit evidence.

## F1/AC15 - Dispatch Fixture Replay

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter replay-dispatch --dispatch-plan DISPATCH_PLAN_ID --fixture PATH` as a fixture-only path from recorded dispatch intent to controller/state/evidence replay.
- Require a recorded ready dispatch gate before replay. A dispatch plan with no ready gate fails before parsing the fixture or mutating controller state.
- Reuse the selected dispatch plan's adapter kind and agent binding rather than accepting separate adapter/agent/goal arguments. The command never stores or renders the original raw dispatch prompt.
- Keep this explicitly non-provider execution: `provider_cli_executed=false`, no planned runtime workspace/artifact directory creation, and raw provider fixture text is filtered into content hashes/evidence summaries only.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- The future real-provider execution command can reuse the same dispatch-plan and ready-gate lookup, but must append runtime start/process lifecycle records instead of fixture replay events.

## F1/AC16 - Dispatch Replay Read Model

Status: completed on 2026-05-25.

Decisions:

- Add `AdapterDispatchReplayProjection` as a durable read model for deterministic dispatch fixture replay results.
- Keep the lifecycle facts separate:
  - dispatch plans record intended runtime metadata;
  - dispatch gates record whether execution would be allowed;
  - dispatch replays record fixture-ingestion outcomes and counts.
- Persist fixture path and fixture hash plus event counts, session/run refs, and `raw_content_policy=content_hashed_not_rendered`. Do not persist raw fixture message/tool text or raw dispatch prompt text.
- Expose replay rows through `ProjectDashboard.adapter_dispatch_replays` and CLI dashboard rendering so future operator surfaces can inspect fixture replay history without parsing session events.

Verification:

- `cargo test -p capo-state adapter_dispatch_replay -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- A future real-provider execution result should use a sibling projection with runtime process refs and adapter stream cursor refs rather than overloading fixture replay rows.

## F1/AC17 - Dispatch Chain Status

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID` as a read-only operator summary for the recorded dispatch chain.
- Reuse `ProjectDashboard` read models for dispatch plans, gate audits, replays, and dogfood readiness. The command does not add a new table or persistence path.
- Render the latest gate and latest replay for the selected plan, including gate reasons, replay counts, raw-content policy, and a conservative next action.
- Keep raw dispatch prompt text and raw provider/fixture output out of the command. The command reports only IDs, counts, policies, statuses, and already-redacted metadata.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Future voice/web/mobile surfaces should use the same query/read-model facts for dispatch-chain summaries rather than reconstructing chain state from events.

## F1/AC18 - Dispatch Execution Request Audit

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter execution-request --dispatch-plan DISPATCH_PLAN_ID [--record]` as the first durable audit surface for an operator request to cross from planned/gated dispatch into real provider execution.
- Keep execution requests separate from plans, gates, and fixture replays. This preserves the lifecycle vocabulary: plan intent, gate permission, replay fixture ingestion, execution request boundary-crossing intent.
- Fail closed without a latest recorded ready gate. Blocked requests can still be recorded to explain why real execution did not start.
- Even with a ready gate, this slice records `status=waiting_on_explicit_provider_opt_in` and `provider_cli_executed=false`. Actual provider CLI launch remains deferred behind explicit opt-in env vars.
- Use adapter-specific future opt-in env names: `CAPO_RUN_CODEX_LOCAL_DISPATCH` and `CAPO_RUN_CLAUDE_LOCAL_DISPATCH`.

Verification:

- `cargo test -p capo-state adapter_dispatch_execution_request -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- The future provider-running command should require a recorded ready execution request plus explicit opt-in env before invoking `LocalProcessRunner`, then append a sibling execution-result projection with runtime process refs and adapter stream cursor refs.

## F1/AC19 - Dispatch Prompt Source Contract

Status: completed on 2026-05-25.

Decisions:

- Add prompt-source records as separate facts from dispatch plans. This avoids expanding the plan table into raw prompt storage while still telling future runners whether a prompt can be safely materialized.
- Keep `raw_prompt_policy=not_rendered` for all recorded prompt sources.
- Mark plain `adapter plan-launch --record` prompts as `source_kind=inline_cli_prompt` and `materialization_status=manual_prompt_not_replayable`.
- Mark `workpad plan-next --record` prompts as `source_kind=workpad_task` and `materialization_status=replayable_if_source_hash_matches`, with source path/anchor and indexed source hash.
- Future real execution should refuse to materialize a workpad prompt if the current workpad hash differs from the prompt-source `source_hash`.

Verification:

- `cargo test -p capo-state adapter_dispatch_prompt_source -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- A future provider-running command should materialize only hash-guarded workpad prompt sources or explicitly supplied one-shot prompt input; it should not reconstruct inline CLI prompts from historical state.

## F1/AC20 - Dispatch Prompt Materialization Dry Run

Status: completed on 2026-05-26.

Decisions:

- Add `capo adapter materialize-prompt --dispatch-plan DISPATCH_PLAN_ID [--record]` as a provider-free dry run over recorded prompt-source rows.
- Keep output and state prompt-redacted. The command reports prompt/source hashes, status, raw prompt policy, and reason codes, but never renders the materialized prompt.
- Inline CLI prompt sources fail closed with `blocked_non_replayable_prompt` because the raw prompt was intentionally not retained.
- Workpad prompt sources become `ready_without_rendering_prompt` only when the indexed workpad file hash matches the recorded source hash and the derived `workpad_task_goal` hash matches the recorded prompt hash.
- Prompt materialization is a separate read model from prompt source and execution requests, so future provider execution can require a recent ready materialization fact without overloading source metadata.

Verification:

- `cargo test -p capo-state adapter_dispatch_prompt_materialization -- --nocapture`: passed.
- `cargo test -p capo-query adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- Future real dispatch execution should require `ready_without_rendering_prompt` before constructing provider argv, then perform the final provider launch only under the explicit opt-in env.

## F3/DS1 - Query Surface Extraction

Status: completed on 2026-05-25.

Decisions:

- Add `capo-query` as the shared read-model aggregation crate for operator surfaces.
- Keep `capo-query` small and side-effect free. It depends on `capo-core` and `capo-state` only, avoiding controller/runtime/adapter dependencies.
- Move dashboard aggregation out of `capo-cli` into `ProjectDashboard`, `AgentDashboardRow`, and `SessionDashboardRow`.
- Keep terminal rendering in `capo-cli`; query structs are renderer-neutral so voice, web, mobile, and future TUI surfaces can consume the same contract.
- Preserve the existing CLI dashboard output shape for P12/P13 compatibility.

Verification:

- `cargo test -p capo-query`: passed.
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`: passed.
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused review found no architecture blocker. It confirmed aggregation moved out of CLI, `capo-query` dependencies are clean, and renderer neutrality is preserved.
- Review requested stronger query-contract coverage for project filtering, idle agents, custom recent-event limits, and missing current-session read models. Those tests were added before completion.

Follow-up:

- DS2 should extend the query contract before rendering new operator fields, especially tool-call and memory-packet details.
- Voice integration should prefer `capo-query` dashboard/session structs over reading state projections directly.

## F3/DS2 - Operator Dashboard View

Status: completed on 2026-05-25.

Decisions:

- Extend `ProjectDashboardQuery` and `SessionDashboardRow` rather than adding a parallel CLI-only dashboard path.
- Include tool-call projections and memory-packet refs in the shared dashboard query contract so CLI, voice, web, mobile, and future TUI views can render the same operator facts.
- Keep the DS2 rendering surface as the existing text CLI dashboard. This preserves a useful dogfood operator view without adding a premature UI dependency.
- Add `capo dashboard --project PROJECT_ID`, `--session SESSION_ID`, and `--status STATUS`.
- Treat `--status` as an any-status filter over agent, session, and run status for v0. This is broad but useful for the current text dashboard; split into status domains later if dogfood usage shows ambiguity.
- Reject unknown dashboard flags and missing filter values. Operator filters should fail closed instead of showing a broader state view than requested.

Verification:

- `cargo test -p capo-query`: passed.
- `cargo test -p capo-cli dashboard_rejects_malformed_filters`: passed.
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused dashboard review found two medium blockers: the CLI had no user-facing project filter, and malformed filters could silently widen dashboard output. Both were fixed before completion.
- The review also identified broad status matching as a low residual risk. It is documented as intentional v0 behavior.

Follow-up:

- If `--status` becomes confusing during dogfood, split it into explicit `--agent-status`, `--session-status`, and `--run-status` filters.
- Future web/TUI work should consume `capo-query` structs directly rather than adding state reads in a UI crate.

## F3/DS3 - Workpad Queue Visibility

Status: completed on 2026-05-25.

Decisions:

- Extend `ProjectDashboard` with `workpad_tasks` so all operator surfaces can see the indexed dogfood queue through the shared query contract.
- Render workpad task rows in the CLI dashboard with source path, source anchor, observed markdown status, Capo execution status, and deterministic default Capo task ID.
- Keep dashboard rendering read-only. Workpad import/start/propose commands remain explicit mutation surfaces.

Verification:

- `cargo test -p capo-query workpad_tasks -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Follow-up:

- Future TUI/web/mobile dashboards should consume `ProjectDashboard.workpad_tasks` instead of querying state directly.

## F3/DS4 - Workpad Queue Filters

Status: completed on 2026-05-25.

Decisions:

- Add explicit `--workpad-path` and `--workpad-status` dashboard filters backed by `ProjectDashboardQuery`, not CLI-only filtering.
- Keep `--status` scoped to agent/session/run rows. Workpad filters use separate names so operators do not accidentally hide or widen the wrong state surface.
- Let `--workpad-status` match either observed markdown status or Capo execution status because dashboard rows expose both as separate fields.
- Fail closed on missing workpad filter values, matching the existing dashboard filter parser behavior.

Verification:

- `cargo test -p capo-query workpad -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_rejects_malformed_filters -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

## F5/ME1 - Memory Record Read Models

Status: completed on 2026-05-25.

Decisions:

- Promote memory beyond packet-only evidence with SQLite projections for `MemoryRecordProjection` and `MemorySourceProjection`.
- Keep operational truth in the event/projection log. Memory records are derived read models with provenance, review state, sensitivity, redaction, validity, supersession, and invalidation fields.
- Store replayable source provenance separately from the record body. Sources track source kind, event/artifact/path refs, anchor, content hash, source sequence, quote artifact, and observed timestamp.
- Add `packet_eligible_memory_records` as the first selected-record read path for packet building. It only returns reviewed, non-invalidated, non-expired, non-sensitive records with a packet item ref and at least one replayable source hash plus anchor/event/artifact locator.
- Fail closed when rebuilding incomplete memory record projection payloads. Missing subject, predicate, object, body, confidence, or redaction state now stops projection rebuild instead of defaulting to safe-looking values.

Verification:

- `cargo test -p capo-state memory_record`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused memory read-model review found two blockers: packet eligibility could select records without replayable sources, and projection replay defaulted required payload fields. Both were fixed with regression tests before completion.

Follow-up:

- ME2 should derive task outcome reports from events, tool calls, evidence, and memory refs rather than free-form summaries.
- Future packet builder integration should consume `packet_eligible_memory_records` with `memory_sources_for_record` rather than reading packet artifacts as the memory authority.

## F5/ME2 - Task Outcome Report

Status: completed on 2026-05-25.

Decisions:

- Add `TaskOutcomeReport` generation in `capo-eval` as a local evidence-backed report builder, not an LLM summary.
- Derive report content from state read models: session, latest run, event trace, tool calls, evidence refs, and memory packet refs.
- Add `task_outcome_reports` as the durable state projection for report refs, counts, duration sequence span, confidence, blocker, review outcome, and report artifact ID.
- Add `capo eval task-outcome --session SESSION_ID --out DIR` as the first CLI export path for task outcome reports.
- Treat outcome report markdown as evidence artifacts with `<!-- capo:task-outcome-report -->` markers. Changed Capo-owned report files and non-Capo files are not overwritten.
- Filter prior `task_outcome_report` evidence and `task.outcome_report_generated` events out of report generation so reruns do not become self-referential.
- Derive `review_outcome` from recorded evidence kinds for ME2. `review_blockers` / `review_findings` and `review_no_blockers` / `reviewed_no_blockers` drive the initial outcome value until ME3 adds first-class review finding records.
- Key report, artifact, event, and idempotency identity from the same stable snapshot inputs, including completed sequence and review outcome.
- Refuse reports for non-terminal runs. The current terminal set covers completed/failed/canceled/interrupted/exited/exited_unknown, with interrupted fake-controller flows becoming reportable after recovery marks the run `exited_unknown`.

Verification:

- `cargo test -p capo-eval`: passed.
- `cargo test -p capo-state task_outcome`: passed.
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused reviews found blockers in report idempotency, self-referential reruns, overwrite safety, free-form review outcome input, missing terminal-status guard, conflicting review evidence precedence, and report/artifact/event identity alignment. Fixes and regression coverage were added before completion.
- Final focused re-review found no blockers in no-review reruns, later review evidence behavior, overwrite guards, idempotency/state consistency, or terminal status gating.

Follow-up:

- ME3 should replace evidence-kind review derivation with first-class review finding records linked to tasks, sessions, tools, and follow-up workpad items.
- Later UI/query work should expose task outcome reports without requiring users to inspect CLI markdown artifacts manually.

## F5/ME3 - Review Feedback Loop

Status: completed on 2026-05-25.

Decisions:

- Add `review_findings` as the first durable review feedback read model. Findings link project, task, session, optional run, optional tool call, optional follow-up workpad task, reviewer, kind, severity, status, summary, evidence artifact, and follow-up text.
- Add `capo review record --session SESSION_ID --reviewer NAME --kind blocker|finding|no_blockers --summary TEXT --out DIR` as the CLI capture path for human/subagent review outcomes.
- Review findings write guarded `<!-- capo:review-finding -->` markdown artifacts and `review.finding_recorded` events, plus evidence projections using review evidence kinds.
- Task outcome report review derivation now prefers first-class `review_findings` over legacy review evidence kinds.
- Validate tool-call links against the target session before persistence.
- Validate follow-up workpad task links against the session project before persistence.
- Include follow-up workpad task ID in review finding identity so changing only the follow-up link produces a distinct review finding instead of a stale overwrite/idempotency collision.

Verification:

- `cargo test -p capo-state review_findings`: passed.
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused ME3 review found blockers in follow-up identity and unchecked tool/workpad links. Both were fixed.
- Focused ME3 re-review found no blockers in identity/idempotency, link validation, or review artifact overwrite safety.

Follow-up:

- Future dashboard/query work should expose review findings directly so operators do not need to inspect markdown artifacts for blockers.
- Future workpad writeback should use review findings as the durable source when creating follow-up items or marking findings resolved.

## F4/PT1 - Static Policy Variant

Status: completed on 2026-05-25.

Decisions:

- Add `PermissionPolicy::Static` as a real static-dispatch policy variant beside `TrustedLocal` and `Fake`.
- Start with `read-only-local` and `reviewer` profiles. `read-only-local` can invoke read/status/workpad tools and read git status/diff scopes; it denies state writes, shell execution, memory packet build, and other absent scopes.
- Keep `trusted-local-dev` as explicit opt-in broad local prototype behavior. It still produces durable grant metadata.
- Use `serde_json` to parse requested scope JSON as an array of strings. Malformed, object-shaped, or mixed-type payloads are denied.
- Scope grant IDs by session, effect, profile, and scope payload to avoid same-session grant collisions.
- Persist decision source, persistence, and explanation in `capability_grants`, and migrate existing local stores by adding missing columns.
- Controller-level denied permissions now stop tool invocation, grant-use events, memory packet creation, and evidence recording.

Verification:

- `cargo info serde_json`: version `1.0.150`, license `MIT OR Apache-2.0`, rust-version `1.71`.
- `cargo test -p capo-tools`: passed.
- `cargo test -p capo-state artifacts_tool_grants_memory_and_evidence_are_persisted_and_rebuilt`: passed.
- `cargo test -p capo-controller denied_static_permission_stops_tool_invocation_in_controller_path`: passed.
- `cargo test -p capo-controller`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- First focused permission review found blockers in scope parsing, grant identity, and decision metadata durability. Structured JSON parsing, scoped grant IDs, and persisted decision metadata resolved them.
- Second focused permission review found blockers in denied controller execution and session-scoped permission event IDs. Denied permissions now stop execution in the controller path, and permission lifecycle event IDs include grant/tool identity.

Follow-up:

- PT2 should build the pending approval queue on the same `PermissionDecision` and grant metadata instead of introducing a separate approval vocabulary.
- PT3 should route wrapper tools through the same deny-before-invoke controller behavior.

## F4/PT2 - User Approval Queue

Status: completed on 2026-05-25.

Decisions:

- Add `permission_approvals` as the first durable approval queue read model. It tracks pending/decided status, requested scope, subject, requested-by actor, reason, session/tool-call refs, selected decision, and linked grant ID.
- Add CLI-first approval operations:
  - `capo permission request --approval APPROVAL_ID --scope-json JSON --reason REASON`
  - `capo permission list`
  - `capo permission decide --approval APPROVAL_ID --decision allow_once|allow_always|reject_once|reject_always`
- Keep decisions on the same capability-grant projection used by PT1 instead of adding a parallel permission vocabulary.
- Map `allow_once` to `effect=allow`, `persistence=once`, and a grant subject scoped by approval ID plus session/tool-call refs when present.
- Map `allow_always` to `effect=allow`, `persistence=until_revoked`, but restrict it in the PT2 CLI path to Capo-owned read/status scopes.
- Map `reject_once` to a decided approval without a reusable deny grant.
- Map `reject_always` to `effect=deny`, `persistence=until_revoked`, and a scoped durable denial grant.
- Move approval decisions into a state-store transaction with a pending-status guard. This prevents two concurrent deciders from both committing conflicting decisions.
- Emit `capability.grant_created` for decisions that create a grant, keeping the audit stream aligned with the controller permission lifecycle.
- Validate permission approval/grant JSON in the state layer before commit so non-CLI projection producers cannot create rows that later break replay.

Verification:

- `cargo test -p capo-state permission_approval`: passed.
- `cargo test -p capo-cli permission_approval_queue_maps_decisions_to_scoped_grants`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- First focused review found two blockers: duplicate/conflicting decisions could race, and `allow_always` could mint broad durable grants. Both were fixed.
- A second focused review found two additional blockers and two medium audit/replay issues: one-shot decisions could become reusable grants, decision races were only sequentially tested, grant creation lacked a `capability.grant_created` event, and state-layer JSON safety depended on CLI validation. All were fixed.
- Final focused re-review found no blockers.

Follow-up:

- PT3 should reuse the approval queue when wrapper tools encounter a policy decision that needs user input instead of assuming all decisions are available synchronously.

## F4/PT3 - Tool Wrapper Expansion

Status: completed on 2026-05-25.

Decisions:

- Add `ToolExposure::Runtime(RuntimeToolWrappers)` as the first wrapper boundary for tools Capo executes directly.
- Register wrapper tools for shell, git status, git diff, file read, file write, and workpad read.
- Route shell/git wrappers through `LocalProcessRunner` so workspace checks, redaction rules, output limits, and runtime output artifacts stay behind the runtime boundary.
- Keep `capo.workpad_read` narrower than general file reads: it accepts `TASKS.md`, `project.md`, and `workpads/*.md` paths only.
- Record wrapper input/output artifacts with content hashes, sizes, URI, summaries, and redaction state.
- Apply configured redaction rules to wrapper-owned input artifacts as well as runtime stdout/stderr.
- Sanitize tool call IDs and run IDs before using them as artifact path components.
- Bind split authorization to tool, session, run, tool call, profile, scope, and input/context hash before invocation. Capo-owned registry context hashing is length-prefixed to avoid newline-collision replay.

Verification:

- `cargo test -p capo-tools`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Review:

- Focused review found blockers in authorization replay, `workpad_read` arbitrary file reads, artifact path escaping, unredacted input artifacts, and misleading permission event status. All were fixed.
- Re-review found same-tool replay and runtime run ID path blockers. Both were fixed.
- Final re-review found ambiguous Capo registry context hashing. Length-prefix context hashing and regression coverage fixed it.

Follow-up:

- Controller integration should persist wrapper artifacts and lifecycle events through the existing state projections instead of leaving PT3 as a crate-level execution boundary.
- PT3 does not make provider-native tools governed; native Codex/Claude tool observations remain observed-only unless routed through these wrappers or reported with structured lifecycle evidence.

## F6/V1 - Voice Controller Integration

Status: completed on 2026-05-25.

Decisions:

- Add `capo voice submit --transcript TEXT` as the first dummy-transcript integration surface. It is intentionally text-only; real audio capture, ASR, streaming, and mobile voice remain deferred.
- Keep `capo-voice` responsible for lowering transcripts into `VoiceCommandPlan`; the CLI only submits the resulting command envelope or renders the read contract.
- Route non-mutating voice status/dashboard plans through `capo-query::project_dashboard` so voice, CLI, dashboard, web, and mobile continue sharing the same read-model contract.
- Route steering plans through `FakeBoundaryController::redirect_command` instead of writing state directly.
- Preserve raw transcript non-retention by not storing the transcript in events, artifacts, or output. The durable command text for steering is the normalized target goal, not the raw transcript.
- Unknown transcripts and stop/interrupt plans without `--confirm` return a response without appending state events.

Verification:

- `cargo test -p capo-voice`: passed.
- `cargo test -p capo-cli voice -- --nocapture`: passed.

Follow-up:

- V2 should add first-class voice-origin approval/audit records for visible confirmations instead of treating `--confirm` as a bare CLI flag.
- V3 should prove retained summaries pass review/redaction before memory ingestion and that raw transcripts remain absent from state and evidence artifacts.

## F6/V2 - Voice Permission Confirmation

Status: completed on 2026-05-25.

Decisions:

- Privileged voice plans now use the existing permission approval projection instead of a parallel confirmation vocabulary.
- Unconfirmed voice stop/interrupt requests queue a `voice-control` approval with `scope_json=[\"voice:approve:privileged\"]`, `requested_by=voice:<actor>`, and the active session ID when one is known. The queued approval uses only intent/session metadata, not the raw transcript.
- Confirmed voice stop/interrupt requests create the approval if needed, record an `allow_once` decision, create a once-scoped capability grant with `decision_source=user_visible_voice_confirmation`, and only then call the controller stop/interrupt handler.
- Controller stop/interrupt commands receive generic durable reasons such as `voice stop confirmed` or `voice interrupt confirmed`. This avoids persisting the raw transcript or the voice-derived reason from the dummy parser.
- The first visible confirmation surface is still CLI `--confirm`; future voice/mobile UI can drive the same approval records directly.

Verification:

- `cargo test -p capo-voice`: passed.
- `cargo test -p capo-cli voice -- --nocapture`: passed.

Follow-up:

- V3 should add retention/redaction smoke coverage for reviewed voice summaries before memory ingestion.
- A future UI/API slice should make voice approvals visible outside CLI output, reusing the `permission_approvals` read model.

## F6/V3 - Voice Retention And Redaction Smoke

Status: completed on 2026-05-25.

Decisions:

- Add an explicit dummy retention path to `capo voice submit`: `--redacted-summary TEXT --reviewed-summary`.
- Refuse redacted-summary retention unless the caller also marks the summary reviewed. This keeps generated or unreviewed voice summaries out of memory records.
- Store retained voice summaries as existing `MemoryRecordProjection` / `MemorySourceProjection` rows with `review_state=reviewed`, `redaction_state=redacted`, `record_kind=summary`, and source anchor `voice:redacted-summary`.
- Do not create raw transcript artifacts. The memory ingest event stores record ID, intent, voice session ID, and summary hash, not transcript text.
- The smoke test scans the state tree for a raw phrase that appeared only in the submitted dummy transcript, proving the current CLI path did not persist it in SQLite or artifacts.

Verification:

- `cargo test -p capo-cli voice -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

Follow-up:

- Future real ASR/audio work must repeat this proof with audio/transcript artifacts and a stronger redaction pipeline before enabling durable transcript retention.

## F1/AC3a - Deterministic Adapter Replay

Status: completed on 2026-05-25.

Decisions:

- Add `FakeBoundaryController::apply_normalized_adapter_events` as the controller-owned replay seam for parsed provider/ACP streams.
- Keep provider streams as adapter inputs, not controller truth. Replay writes Capo-owned event/projection rows for session summaries, adapter-native tool calls, and adapter replay evidence.
- Store raw provider event hashes and content hashes in event payloads instead of raw provider message/tool text. This keeps the deterministic replay path useful for introspection without training the state model to persist provider output verbatim.
- Preserve existing fake controller flows and adapter parser fixtures. Codex and Claude replay tests now prove normalized fixture events can update Capo read models without launching subscription-backed CLIs.
- Do not mark real-agent readiness complete. The real Codex/Claude smoke remains opt-in gated and unrun.

Verification:

- `cargo test -p capo-controller replay -- --nocapture`: passed.

Follow-up:

- After explicit user opt-in, run a real subscription-backed adapter smoke and pass the resulting normalized stream through the same controller replay seam, then export evidence and scan it for credential/session markers.

## F1/AC3b - Adapter Fixture Replay CLI

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter replay-fixture --adapter codex|claude|acp --fixture PATH --agent NAME --goal GOAL [--out DIR]` as a deterministic operator surface for the adapter replay seam.
- The command registers the target agent if needed, starts a normal Capo session through the fake controller/runtime scaffold, replays normalized adapter fixture events through controller-owned state, and optionally exports markdown evidence.
- Evidence export remains Capo-owned markdown. Tests assert raw provider message/tool text from the fixture is absent from CLI output, state files, and exported evidence.
- This is not a substitute for real subscription-backed smoke. It is the regression harness that the real smoke should converge on after opt-in.

Verification:

- `cargo test -p capo-cli adapter_fixture -- --nocapture`: passed.

Follow-up:

- Real opt-in Codex/Claude smoke should feed captured normalized streams into this same replay/evidence path and then run credential/session marker scans.

## F1/AC4 - Connector Readiness Surface

Status: completed on 2026-05-25.

Decisions:

- Add `capo adapter readiness` as a deterministic operator check for the subscription-backed connector gate.
- The command renders Codex and Claude smoke-plan metadata: adapter kind, program, opt-in env var, opt-in state, expected marker, env allowlist count, redaction rule count, output limit, planned workspace path, and planned artifact path.
- The command deliberately does not run Codex or Claude, inspect vendor subscription state, read provider credentials, or create smoke directories.
- Keep `ready_for_real_agent_dogfood=false` until a real opt-in smoke is separately recorded and scanned. This prevents a readiness/config check from being mistaken for real connector proof.

Verification:

- `cargo test -p capo-cli adapter_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- After explicit user opt-in, the real smoke should write a durable readiness/evidence result that can change this dogfood blocker from `real_subscription_smoke_not_recorded`.

## F1/AC5 - Durable Connector Readiness State

Status: completed on 2026-05-25.

Decisions:

- Add `AdapterReadinessProjection` so connector readiness is queryable state, not only transient CLI text.
- Add `capo adapter readiness --record` to write Codex/Claude readiness rows through `adapter.readiness_checked`.
- Include recorded adapter readiness in the shared dashboard query and CLI dashboard rendering.
- Keep the recorded status conservative: rows record opt-in status, smoke-plan metadata, credential policy, and `dogfood_blocker=real_subscription_smoke_not_recorded`; they do not claim that real agent execution is proven.
- The command still does not launch provider CLIs, inspect subscriptions, or read credentials.

Verification:

- `cargo test -p capo-state adapter_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_readiness -- --nocapture`: passed.

Follow-up:

- A future opt-in smoke command should update or supersede these rows with real-smoke evidence and credential-scan results.

## F1/AC6 - Real Smoke Evidence Contract

Status: completed on 2026-05-25.

Decisions:

- Add `AdapterSmokeReportProjection` so real-smoke outcomes have a durable state contract before any subscription-backed smoke is run.
- Add `capo adapter smoke-report record` for explicit operator/test recording of skipped, failed, or passed smoke results.
- Refuse `status=passed` unless `--credential-scan clean` and `--marker-found` are both present. This prevents a passing connector claim without the two evidence gates AC1 requires.
- Render smoke reports in `capo dashboard` through the shared query contract.
- The command records evidence only; it does not launch provider CLIs or scan artifacts itself.

Verification:

- `cargo test -p capo-state adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.

Follow-up:

- The opt-in real smoke runner should write a passed or failed `AdapterSmokeReportProjection` from actual runtime artifacts after marker and credential scans.

## F7/RR1 - Loopback Remote Runtime Contract

Status: completed on 2026-05-25.

Decisions:

- Add `RuntimeRunner::RemoteProcess(RemoteProcessRunner)` and keep it separate from `ConnectivityTunnel`.
- Start with a loopback remote runner that wraps `LocalProcessRunner` for deterministic tests. It proves remote-shaped refs and lifecycle semantics without SSH, Tailscale, cloud credentials, or public exposure.
- Remote process refs include `remote_target_id` and `endpoint_ref` in the opaque runtime reference. This preserves the distinction between process ownership and endpoint reachability.
- Remote lifecycle/control facts are emitted as runtime events: target resolution, remote process start, interrupt sent, terminate sent, and recovery classification.
- Do not claim remote execution is production-ready. RR1 proves the contract shape only; RR2 still needs a tunnel adapter stub, and RR3 needs explicit exposure policy.

Verification:

- `cargo test -p capo-runtime remote_runtime -- --nocapture`: passed.

Follow-up:

- RR2 should add a tunnel/endpoint adapter stub and keep endpoint health/readiness records separate from runtime process refs.

## F7/RR2 - Tunnel Adapter Stub

Status: completed on 2026-05-25.

Decisions:

- Add `PLANNED_TUNNELS = ["fake", "local-loopback", "endpoint-stub"]` beside planned runtimes so connectivity variants are explicit without becoming runtime runners.
- Add `EndpointStubTunnel` as a deterministic non-credentialed connectivity adapter for endpoint resolution, reachability, and exposure policy. It does not start or control agent processes.
- Keep endpoint records distinct from runtime process refs:
  - `ResolvedEndpoint` records endpoint ID, owner, channel, URI, exposure, permission scope, and permission-required status.
  - `ConnectivityHealth` records endpoint reachability, status, exposure, and detail.
  - `ExposureReport` records permission scope and the `connectivity.exposure_changed` audit event kind.
- Model exposure permission scope at the connectivity boundary: loopback maps to `network:connect:localhost`, private maps to `network:connect:private_tunnel`, and public maps to `network:expose:public`.
- Keep `LocalLoopbackTunnel` strict: it resolves dashboard/control/artifact-style local channels and rejects stdio channels so the local tunnel does not accidentally look like a remote runtime transport.

Verification:

- `cargo test -p capo-runtime tunnel -- --nocapture`: passed.

Follow-up:

- RR3 should wire explicit exposure policy into durable permission events/read models before public or remote-control exposure is treated as available.

## F7/RR3 - Explicit Exposure Policy

Status: completed on 2026-05-25.

Decisions:

- Add `ConnectivityExposureProjection` to the state layer instead of storing exposure state inside runtime process refs. This keeps endpoint reachability and public/private exposure separate from agent process ownership.
- Add durable connectivity event kinds for requested, changed, revoked, and health-changed exposure facts.
- Link active exposure to a `CapabilityGrantProjection`; the regression path leaves private remote-control exposure `blocked_pending_permission` until a durable `capability.grant_created` event/projection exists.
- Keep revocation visible in the same read model with `status=revoked`, `reachable=false`, disabled health, and `revoked_at`.
- This is still not production remote execution. RR3 proves durable exposure policy state; real remote operation still depends on the F1 real-agent connector proof and later concrete SSH/Tailscale/worker adapters.

Verification:

- `cargo test -p capo-state connectivity_exposure -- --nocapture`: passed.

Follow-up:

- A future CLI/API surface should render connectivity exposure rows and drive approval decisions using the existing permission approval queue.
- Concrete SSH/Tailscale adapters must attach real identity/auth refs and health checks to this projection before any dogfood remote-control path.

## F7/RR4 - Dashboard Exposure Visibility

Status: completed on 2026-05-25.

Decisions:

- Add connectivity exposures to `ProjectDashboard` so CLI, voice, web, mobile, and future TUI surfaces can share the same read contract for remote/public-access state.
- Render exposure rows in the CLI dashboard with endpoint, owner, channel, exposure scope, status, health, reachability, permission scope, grant, and revocation timestamp.
- Keep the dashboard read-only. It shows blocked/active/revoked exposure state but does not decide permissions or mutate tunnel state; approval decisions stay in the existing permission queue.

Verification:

- `cargo test -p capo-query connectivity -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_connectivity -- --nocapture`: passed.

Follow-up:

- A future approval UI/API should use these rows plus `permission_approvals` to let operators grant or revoke exposure without parsing raw event logs.

## F7/RR5 - Connectivity Exposure Operator Surface

Status: completed on 2026-05-26.

Decisions:

- Add `capo connectivity expose-stub` as a provider-free operator surface for planning and recording connectivity exposure intent.
- Keep this command on the connectivity boundary. It resolves endpoint metadata through `ConnectivityTunnel`, writes `ConnectivityExposureProjection` rows when `--record` is used, and does not start runtime processes or provider CLIs.
- Private and public exposures fail closed with `status=blocked_pending_permission` and a permission scope such as `network:connect:private_tunnel`. Loopback exposure can be active because it does not require remote/public permission.
- Public stub exposure keeps its allowed-channel list narrow; unsupported channel requests fail before any state write.

Verification:

- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- A future slice should connect exposure approval decisions to this operator surface so a matching durable grant can transition a blocked exposure to active without hand-writing projections in tests.
- Concrete SSH/Tailscale/cloud adapters remain deferred until local real-agent dispatch is proven.

## F7/RR6 - Connectivity Exposure Approval Bridge

Status: completed on 2026-05-26.

Decisions:

- Add `capo connectivity request-approval` to derive a scoped permission approval from a blocked connectivity exposure row.
- Add `capo connectivity activate-exposure` to transition a blocked exposure to active only after a matching allow grant exists.
- Match grants by scope and subject subset: the grant scope must include the exposure's permission scope, and the grant subject must include the exposure ID, endpoint, owner kind, owner ID, channel, and exposure scope. Extra permission metadata such as approval ID is allowed.
- Reuse the existing permission approval and `CapabilityGrantProjection` machinery. Do not create a second connectivity-specific permission model.
- Activation remains a metadata/read-model transition. It does not create a real tunnel, start a runtime process, launch provider CLIs, or inspect credentials.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Concrete tunnel adapters should replace stub health with real endpoint health before active exposure is interpreted as reachable in production.
- A later revocation command should pair with this activation surface so operators can revoke exposure without hand-writing projection rows in tests.

## F7/RR7 - Connectivity Exposure Revocation Surface

Status: completed on 2026-05-26.

Decisions:

- Add `capo connectivity revoke-exposure` as the operator command for disabling a recorded connectivity exposure.
- Revocation writes `ConnectivityExposureRevoked` and updates the exposure read model to `status=revoked`, `health_status=disabled`, `reachable=false`, and a `revoked_at` timestamp.
- Preserve the linked `capability_grant_id` as audit history. The command does not delete or rewrite grants.
- Keep this as a state/audit transition only. Concrete tunnel shutdown remains future adapter work because the current endpoint is still a stub.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- When concrete tunnels exist, revocation should call the connectivity adapter shutdown path and record adapter-specific cleanup evidence before projecting disabled health.

## F1/AC21 - Real Dispatch Runner Preflight

Status: completed on 2026-05-26.

Decisions:

- Add `capo adapter run-preflight` as the provider-free seam immediately before future real provider execution.
- Compose four independent facts before any provider command can run: recorded dispatch plan, recorded execution request, recorded prompt materialization, and explicit provider opt-in env.
- Keep inline CLI prompts blocked because Capo intentionally does not retain raw prompt text.
- Workpad-derived prompts can pass prompt materialization, but still block on `CAPO_RUN_CODEX_LOCAL_DISPATCH=1` or the adapter-specific equivalent until the user explicitly opts in.
- Do not call `LocalProcessRunner` or provider CLIs in this slice. The command reports readiness only.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- The future real runner command should consume this preflight result and refuse execution unless status is `ready_to_execute_provider_cli`.
- The future runner must use `LocalProcessRunner`, record bounded/redacted runtime artifacts, scan artifacts before any passed report, and persist execution outcomes without raw prompt/provider text.

## F1/AC22 - Guarded Local Dispatch Runner Surface

Status: completed on 2026-05-26.

Decisions:

- Add `capo adapter run-local` as the first command that can cross the local provider execution boundary, but keep it fail-closed unless the existing preflight says `ready_to_execute_provider_cli`.
- Do not render or store raw prompts in the command surface. The runner reconstructs only workpad-derived prompts whose source hash and prompt hash were already proven by `materialize-prompt`.
- Inline CLI prompt dispatch plans stay blocked for execution because Capo does not retain the raw prompt.
- Use `LocalProcessRunner` for actual execution after explicit opt-in, preserving runtime ownership of process launch and bounded/redacted output artifacts.
- Scan stdout/stderr artifacts for credential/session markers after execution before returning a successful local-run result. If the marker scan fails, delete the captured runtime artifacts before returning the error.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Review:

- Focused provider-safety review found a blocker in the first draft: failed marker scans happened after runtime artifacts were written, so sensitive stdout/stderr could remain persisted. The fix deletes captured stdout/stderr artifacts on scan failure and adds regression coverage.
- The same review noted that `run-local` recomputes the preflight from recorded facts instead of requiring a separately recorded preflight row. Accepted for AC22 because Capo does not yet persist preflight rows and the execution predicate still requires recorded plan, execution request, materialization, dogfood gate evidence, and explicit opt-in.

Skipped verification:

- Real Codex or Claude provider execution through `run-local` was not run because `CAPO_RUN_CODEX_LOCAL_DISPATCH=1` or `CAPO_RUN_CLAUDE_LOCAL_DISPATCH=1` requires explicit user opt-in.

Follow-up:

- Persist local dispatch execution outcomes as dedicated state/query/dashboard rows before using `run-local` to satisfy AC3 real-agent controller path.
- After explicit opt-in, run a small workpad-derived Codex dispatch, inspect artifact scans, and record the result without raw prompt/provider text.

## F1/AC23 - Dispatch Execution Outcome Read Model

Status: completed on 2026-05-26.

Decisions:

- Add `AdapterDispatchExecutionProjection` as the durable outcome row for `run-local`, separate from dispatch plans, gates, fixture replays, execution requests, prompt sources, and materialization rows.
- Make blocked preflight outcomes recordable with `run-local --record` while keeping `provider_cli_executed=false`. This gives operator surfaces durable evidence without crossing the provider boundary.
- Successful future provider executions record only runtime/process/artifact refs, exit code, scan status, and redaction policies. Raw prompts and provider output text stay out of inline state and dashboard rendering.
- Add execution outcomes to the shared project dashboard query so CLI, voice, web, and mobile surfaces can inspect the same execution result contract.

Verification:

- `cargo test -p capo-state adapter_dispatch_execution -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Skipped verification:

- Real Codex or Claude execution outcome recording was not smoke-tested because the provider execution env gates still require explicit user opt-in.

Follow-up:

- After explicit opt-in, run `capo adapter run-local --record` against a hash-verified workpad plan and verify that the successful execution row, artifact refs, and credential scan status rebuild correctly after restart.
