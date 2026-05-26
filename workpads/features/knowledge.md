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
- As of 2026-05-26, real local Codex/Claude connector proof is the highest feature-phase priority and the user has explicitly approved using the local Codex / ChatGPT Pro subscription and Claude Code subscription for the gated proof paths. Approval removes the opt-in blocker, but does not relax credential/session non-retention, restrictive launch defaults, artifact scanning, or evidence gates.
- Deterministic agent-interaction tests need a scriptable mock agent, not only fixed Codex/Claude/ACP fixture files. Add the mock behind the existing static-dispatch adapter/controller/runtime boundaries, with `../aget` mock site/tool tests as the reference style for explicit scripted behavior.
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

## F2/DB6 - Dogfood Readiness Surface

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDogfoodReadiness` to `capo-query` so CLI, dashboard, voice, web, and mobile can share one readiness contract.
- Add `capo dogfood readiness` as the operator command for deciding whether Capo can move its own workpads into Capo-managed dogfood.
- Keep the readiness summary read-only and evidence-derived. It does not inspect provider credentials, launch provider CLIs, rematerialize prompts, open tunnels, or edit markdown.
- Report three independent readiness dimensions: real-agent connector proof, indexed workpad bridge state, and recorded dispatch-chain state.
- Keep readiness blocked when any dimension is missing, with explicit blockers and next actions.

Verification:

- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Once a real opt-in Codex run is recorded, use `capo dogfood readiness` as the final checkpoint before moving the Capo project execution loop into Capo.

## F2/DB7 - Dogfood Readiness Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Extend `capo dogfood readiness` with `--out DIR` so the same shared readiness query can produce a durable Capo-owned markdown report.
- Use a project-level evidence record for readiness reports because the report is about migration readiness, not output from one agent session or provider run.
- Keep the artifact prompt-redacted and read-model-derived. The export does not inspect credentials, run provider CLIs, materialize prompts, open tunnels, or edit source markdown.
- Refuse to overwrite non-Capo files or changed Capo readiness reports; exact reruns remain idempotent because the artifact path includes the rendered content hash.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Use readiness exports as the reviewed checkpoint artifact before switching the active project loop into Capo-managed dogfood.

## F3/DS5 - Project Evidence Visibility

Status: completed on 2026-05-26.

Decisions:

- Add a state/query/dashboard path for project-level evidence so dogfood readiness and migration checkpoint artifacts are visible without binding them to a fake session.
- Define project evidence for the current query surface as evidence rows with `session_id IS NULL`. Session-owned evidence remains visible through session dashboard rows and status commands.
- Render project evidence IDs, kinds, artifact refs, and confidence in `capo dashboard` while keeping the view read-only and projection-derived.

Verification:

- `cargo test -p capo-query project_dashboard_includes_project_level_evidence -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Future richer dashboards can group project evidence by readiness checkpoint, migration gate, or review artifact without changing the state authority.

## F3/DS6 - Dogfood Readiness Dashboard Summary

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::dogfood_readiness()` so dashboard, voice, web, mobile, and CLI consumers can derive the same overall migration verdict from the shared dashboard model.
- Render `project_dogfood_readiness`, status, component readiness booleans, blockers, and next actions in `capo dashboard`.
- Keep `capo dogfood readiness` as the dedicated operator command for readiness/export workflows, while the dashboard shows the same decision alongside component rows.

Verification:

- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- When the dogfood gate finally clears, use the dashboard readiness row plus the project readiness artifact as the visible migration checkpoint.

## F1/AC32 - Latest Adapter Smoke Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add latest-selector ergonomics to `capo adapter smoke-report evidence`, matching the existing exact/latest smoke-report status command.
- Reuse `ProjectDashboard::latest_adapter_smoke_report(...)` instead of adding CLI-specific smoke selection logic.
- Preserve the existing adapter smoke evidence artifact format, guarded writer, artifact kind, evidence kind, and confidence logic.
- Keep exact `--smoke-report` and latest `--latest` mutually exclusive. `--adapter` is valid only with `--latest`.
- Treat the export as read-model-derived provider-free evidence. It records project evidence metadata/artifacts, but does not launch provider CLIs, inspect credentials, render smoke stdout/stderr, materialize prompts, open tunnels, request approvals, activate grants, or mutate connector state.

Verification:

- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.

## F1/AC33 - Adapter Dogfood Gate Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo adapter dogfood-gate evidence --out DIR` as the connector-level evidence checkpoint for first real-agent dogfood readiness.
- Keep the report narrower than full dogfood readiness. It covers only the adapter dogfood gate: required adapters, proven adapters, blocked adapters, gate reasons, and smoke-report refs.
- Record the report as project-level evidence with `kind=adapter_dogfood_gate_evidence` so dashboard and future dogfood review flows can cite the gate directly.
- Preserve provider safety: the export is read-model-derived and does not launch provider CLIs, inspect credentials, render smoke stdout/stderr, materialize prompts, open tunnels, request approvals, activate grants, or mutate connector state beyond recording evidence.

Verification:

- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.

## F1/AC34 - Scriptable Mock Agent Harness

Status: completed on 2026-05-26.

Decisions:

- Add `ScriptedMockAgent` / `ScriptedMockTurn` in `capo-adapters` as a reusable provider-free test harness for explicit agent behavior scripts.
- Represent scripted behavior as normalized adapter events with `adapter_kind=mock`, stable timeline keys, stable idempotency keys, and observed-only native tool observations.
- Apply scripted turns through `AgentAdapter::ScriptedMock` plus `FakeBoundaryController::apply_scripted_mock_turn`, which delegates to the existing normalized adapter replay pipeline. This avoids a test-only controller shortcut while still making prompt/response, tool, redirect, permission, failure, interruption, evidence, and interrupt flows deterministic.
- Keep this harness separate from real connector proof. It strengthens deterministic architecture tests, but Codex/Claude subscription-backed smoke evidence is still required before first real-agent dogfood readiness.
- Fix adapter replay event identity to include stable adapter event identity instead of only session plus local index. This prevents multi-turn scripted streams, and later ACP replay/load streams, from colliding on event IDs while preserving idempotency-key dedupe.
- Include the mock event index in scripted timeline keys so repeated streaming deltas for the same item remain distinct while reruns of the same script still dedupe.
- Focused review found and resolved three medium gaps before completion: permission/failure/interruption mock events were not projected, the mock path bypassed static adapter dispatch, and duplicate deltas could collide.

Verification:

- `cargo test -p capo-adapters scripted_mock_agent -- --nocapture`: passed.
- `cargo test -p capo-controller scripted_mock_agent_drives_multi_turn_controller_state -- --nocapture`: passed.

## F1/AC1 - Codex Opt-In Smoke

Status: completed on 2026-05-26.

Decisions:

- Run the real local Codex smoke only after explicit user authorization and only through the existing restrictive local harness.
- Add `--skip-git-repo-check` only to the Codex smoke plan because the smoke workspace is intentionally an isolated temporary directory. This addresses Codex's repo trust check for Capo-owned smoke workspaces without relaxing sandboxing, credential handling, artifact scanning, or normal dispatch trust behavior.
- Treat the first approved smoke run as useful evidence even though it failed: Codex executed through the harness, artifacts were created, the marker was absent, and the blocker was the missing `--skip-git-repo-check` compatibility flag.
- Treat the second approved run as the accepted Codex connector proof: marker present, scanner clean, passed smoke report recorded, and adapter dogfood gate ready for first real-agent dogfood.
- Do not claim full Capo dogfood readiness from Codex proof alone. `capo dogfood readiness` still reports missing available runtime target, missing workpad index, and missing dispatch-chain evidence.
- Ignore `.capo-dev/` in git because it is generated local Capo state. Durable task evidence belongs in the workpad docs and explicit artifacts, not in committed SQLite runtime state.

Verification:

- `codex --version`: `codex-cli 0.133.0`.
- `codex exec --help | rg "skip-git|sandbox|ephemeral|ignore"` confirmed `--skip-git-repo-check`, `--sandbox`, `--ephemeral`, `--ignore-user-config`, and `--ignore-rules`.
- First `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`: failed with missing `CAPO_CODEX_SMOKE_OK` marker because Codex refused the untrusted temp workspace.
- `cargo test -p capo-adapters codex -- --nocapture`: passed after adding the smoke-only flag.
- Second `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`: passed.
- `capo adapter smoke-report scan --artifact-root <local-temp-codex-smoke-artifacts>`: `credential_scan_status=clean`, `files_scanned=2`.
- `rg -a` over `.capo-dev` for credential/session marker names returned no matches.
- Recorded passed smoke report `adapter-smoke-codex_exec-b2e582887f9c0820` with `dogfood_readiness_effect=real_agent_connector_proven`.
- `capo adapter dogfood-gate`: `ready_for_first_real_agent_dogfood=true`.
- `capo dogfood readiness`: `real_agent_connector_ready=true`, still overall blocked on runtime target, workpad index, and dispatch chain.

## F7/RR23 - Latest Runtime Target Control Readiness

Status: completed on 2026-05-26.

Decisions:

- Add latest-selector ergonomics to `capo runtime target readiness`, matching the latest-selector shape already used by runtime target status.
- Reuse `ProjectDashboard::latest_runtime_target(...)` for runner/status filtering, then derive control readiness through `ProjectDashboard::runtime_target_control_readiness(...)`.
- Keep exact `--target` and latest `--latest` mutually exclusive. `--runner` and `--status` filters are valid only with `--latest`.
- Render selector and filter fields before the readiness row so operator surfaces can explain which target Capo selected.
- Keep the command read-only and provider-free: it does not launch runtimes, provider CLIs, tunnels, approvals, grants, credential inspection, raw transcript retention, or state mutation.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F6/V4 - Dogfood Readiness Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for simple dogfood-readiness questions such as "Are we ready to dogfood?"
- Route the answer through the shared project dashboard readiness query, not through voice-specific readiness logic.
- Render readiness status, real-agent connector readiness, workpad bridge readiness, dispatch-chain readiness, blockers, and next actions in the voice read contract.
- Preserve the existing voice safety boundary: no raw transcript retention, no mutation, no provider execution, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-voice dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.

Follow-up:

- Extend this pattern to richer conversational status questions only after dogfood traces show which readiness or agent-state summaries operators actually ask for.

## F6/V5 - Recent Work Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for simple recent-work questions at project scope, including "What have my agents done?"
- Add agent-level recent-work questions such as "What has fake-codex done?" using the existing agent read-model scope.
- Route both paths through the shared project dashboard query so voice, dashboard, CLI, and future mobile/web surfaces use the same read contract.
- Render latest summaries, evidence refs, recent-event counts, active sessions, and project evidence counts from persisted projections only.
- Preserve the existing voice safety boundary: no raw transcript retention, no mutation, no provider execution, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-voice recent_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.

Follow-up:

- Dogfood actual voice conversations before adding broader natural-language parsing; the current grammar deliberately stays narrow and auditable.

## F1/AC26 - Dispatch Status Query Contract

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::adapter_dispatch_status(...)` and `AdapterDispatchStatus` so dispatch-chain status is a shared read-model contract rather than CLI-only aggregation.
- Keep the CLI responsible for text formatting, but move plan/gate/replay/execution selection and next-action derivation into `capo-query`.
- Preserve the provider safety boundary: the status contract exposes metadata, artifact IDs, booleans, counts, statuses, and reason codes only. It does not render raw prompts, raw provider fixture text, or raw provider output.
- Leave `dispatch-evidence` on its existing projection inputs for now because it has broader markdown artifact rendering needs than the compact status contract.

Verification:

- `cargo test -p capo-query adapter_dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Future voice/web/mobile/TUI status surfaces should consume `AdapterDispatchStatus` instead of reimplementing dispatch-chain lookup.

## F6/V9 - Dispatch Status Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for dispatch-chain status questions keyed by dispatch plan ID.
- Route the answer through `ProjectDashboard::adapter_dispatch_status(...)`, reusing the shared plan/gate/replay/execution summary contract from `capo-query`.
- Render only status metadata: plan ID, adapter/provider metadata, dogfood gate, latest gate/replay/execution status, provider execution flags, credential scan status, and next action.
- Preserve the voice and provider safety boundaries: no raw transcript retention, no mutation, no provider execution, no prompt materialization, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-voice dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dispatch_status -- --nocapture`: passed.

Follow-up:

- Dogfood whether operators naturally refer to dispatch plan IDs directly or whether Capo needs a read-only voice helper for "latest dispatch for agent/task" once real dispatch traces exist.

## F1/AC27 - Latest Dispatch Status Selection

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::latest_adapter_dispatch_status(...)` so operator surfaces can inspect the latest dispatch-chain status without requiring a copied dispatch-plan ID.
- Select latest by the maximum activity sequence across the dispatch plan and its related gate, replay, and execution rows. This treats follow-up gate/replay/execution records as activity on the same dispatch chain.
- Support an optional agent-name filter for "latest dispatch for this agent" while keeping exact `--dispatch-plan` lookup unchanged.
- Preserve provider safety: the selector reads projections only and does not rerun gates, materialize prompts, launch providers, inspect credentials, or open tunnels.

Verification:

- `cargo test -p capo-query latest_adapter_dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Voice can use this latest selector in a future natural-language convenience path if direct dispatch-plan IDs prove awkward during dogfooding.

## F6/V10 - Latest Dispatch Status Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice path for "latest dispatch status" questions so operators do not need to know a dispatch-plan ID during conversational control.
- Support an optional agent filter for "latest dispatch status for AGENT" using `ProjectDashboard::latest_adapter_dispatch_status(...)`.
- Reuse the same voice rendering helper as exact dispatch-status questions, so exact and latest status output stay aligned.
- Preserve safety boundaries: no raw transcript retention, no mutation, no provider execution, no prompt materialization, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-voice latest_dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dispatch_status -- --nocapture`: passed.

Follow-up:

- Once real dispatch traces exist, dogfood whether latest-by-agent is enough or whether voice needs latest-by-workpad-task and latest-by-session selectors.

## F1/AC28 - Latest Dispatch Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Extend `dispatch-evidence` with `--latest [--agent NAME]` so reviewed evidence can be exported without copying dispatch-plan IDs.
- Reuse `ProjectDashboard::latest_adapter_dispatch_status(...)` for selection and the existing dispatch-evidence renderer for artifact contents.
- Preserve provider safety: read projections and write Capo-owned evidence only; no provider execution, prompt materialization, credential inspection, tunnels, or workpad edits.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Future voice/web/mobile evidence export should use the same selector once those surfaces can request reviewed artifacts.

## F1/AC1-AC2 - Local Connector Preflight

Status: in progress on 2026-05-25.

Decisions:

- Do not run the real Codex subscription-backed smoke without explicit user opt-in. The ignored test was executed without `CAPO_RUN_CODEX_LOCAL_SMOKE=1` and stayed inside the opt-in gate.
- Installed Codex is `codex-cli 0.133.0`; `codex exec --help` currently supports the planned safe smoke flags: JSONL output, read-only sandbox, ephemeral mode, ignored user config/rules, and isolated `--cd`.
- Installed Claude Code is `2.1.150`; its help currently supports the restricted noninteractive stream path.
- Tighten the Claude smoke plan with `--no-session-persistence`, `--disable-slash-commands`, and `--tools ""` in addition to plan permission mode, disallowed tools, empty MCP config, and strict MCP config.
- Codex was later proven by AC1 on 2026-05-26 with a clean passed smoke report; this earlier preflight note is retained as historical context.
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
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
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

## F5/ME4 - Review Finding Dashboard Visibility

Status: completed on 2026-05-26.

Decisions:

- Add project-scoped review finding lookup to the state boundary so query surfaces can retrieve all recorded findings without parsing artifacts.
- Add `review_findings` to `ProjectDashboard` and per-session `review_findings` to `SessionDashboardRow`.
- Render project-level and session-level review findings in `capo dashboard`, including kind, severity, status, reviewer, evidence artifact, follow-up, and summary.
- Keep dashboard review visibility read-only and projection-derived. Review markdown artifacts remain durable evidence, but operators no longer need to open them to see blockers.

Verification:

- `cargo test -p capo-query review_findings -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_review_findings -- --nocapture`: passed.

Follow-up:

- Future workpad writeback can use these review-finding rows as the durable source for creating or resolving follow-up tasks.

## F5/ME5 - Task Outcome Dashboard Visibility

Status: completed on 2026-05-26.

Decisions:

- Add project-scoped and session-scoped task outcome report lookups to the state boundary.
- Add `task_outcome_reports` to `ProjectDashboard` and per-session `task_outcome_reports` to `SessionDashboardRow`.
- Render project-level and session-level task outcome reports in `capo dashboard`, including outcome status, review outcome, action/tool/evidence/memory counts, confidence, blocker, and report artifact.
- Keep dashboard outcome visibility read-only and projection-derived. Markdown reports remain durable evidence, not the only operator surface for performance/review summaries.

Verification:

- `cargo test -p capo-query task_outcome_reports -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_task_outcome_reports -- --nocapture`: passed.

Follow-up:

- Future voice/web/mobile surfaces can use the same dashboard fields to answer outcome and performance questions without reading report markdown.

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

## F1/AC24 - Dispatch Status Execution Introspection

Status: completed on 2026-05-26.

Decisions:

- Extend `capo adapter dispatch-status` to include the latest dispatch execution outcome from the shared dashboard query contract.
- Keep the status command read-only. It does not recompute preflight, materialize prompts, create runtime directories, inspect credentials, or launch provider CLIs.
- Show execution outcome metadata only: execution ID, status, provider execution flags, credential scan status, stdout/stderr artifact refs, and reason codes. Raw prompts and provider output remain outside command output.
- Use `resolve_latest_execution_blocker` as the next action when a blocked execution outcome is the latest useful fact and no fixture replay or successful execution has superseded it.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- After an explicit real provider opt-in run, verify that `dispatch-status` reports successful execution artifact refs and points the operator toward evidence export/review.

## F1/AC25 - Dispatch Chain Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo adapter dispatch-evidence` as the review artifact surface for the dispatch chain.
- Keep the command provider-free: it reads shared query projections and writes a Capo-owned markdown artifact plus evidence projection. It does not run providers, recompute preflight, materialize prompts, or inspect credentials.
- Include enough state to review the chain: dispatch plan metadata, dogfood gate, latest dispatch gate, latest fixture replay, and latest local execution outcome.
- Keep raw prompts, raw fixture text, and raw provider output out of the artifact. Runtime stdout/stderr are referenced by artifact ID only.
- Use a specific artifact marker, `<!-- capo:adapter-dispatch-evidence -->`, and refuse changed or non-Capo overwrites.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- After explicit provider opt-in, use the same export command to review a successful local execution row before using it as dogfood evidence.

## F6/V6 - Review Needs Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for review/outcome questions such as "What needs review?"
- Route the answer through the shared project dashboard query instead of adding voice-specific state reads.
- Render review-finding counts, open blockers, task outcome report counts, reports with findings, latest review outcome, and linked finding/report rows.
- Preserve the existing voice safety boundary: no raw transcript retention, no mutation, no provider execution, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-voice review_needs -- --nocapture`: passed.
- `cargo test -p capo-cli voice_review_needs -- --nocapture`: passed.

Follow-up:

- After more dogfood traces, decide whether review-needs responses should group findings by agent/session or stay project-level by default.

## F6/V7 - Next Work Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for next-work questions such as "What should we do next?"
- Move the next-workpad selection rule into `ProjectDashboard` so CLI, voice, web, mobile, and future TUI surfaces can share the same workpad queue semantics.
- Select only workpad rows with actionable observed markdown statuses and `capo_execution_status=observed_only`, then use deterministic path, anchor, and task ID ordering.
- Render candidate count, selected source, title, observed status, Capo execution status, and default Capo task ID in the voice read contract.
- Preserve the existing voice safety boundary: no raw transcript retention, no mutation, no provider execution, no credential inspection, and no workpad edits.

Verification:

- `cargo test -p capo-query next_actionable_workpad -- --nocapture`: passed.
- `cargo test -p capo-voice next_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_next_work -- --nocapture`: passed.

Follow-up:

- A future voice steering slice can explicitly lower "start the next task" into the existing `workpad start-next` command path, with confirmation and provider gating.

## F6/V8 - Confirmed Start Next Work Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a voice intent for commands such as "Start next task with fake-codex."
- Treat start-next as privileged because it imports a workpad task and starts a controller session. It requires visible confirmation and records a voice approval plus once-scoped grant before mutation.
- Reuse the existing `workpad start-next` semantics after confirmation instead of adding a new workpad mutation path.
- Keep execution fake/local. The command registers no provider readiness claim, does not run Codex or Claude, does not inspect credentials, and does not retain the raw transcript.

Verification:

- `cargo test -p capo-voice start_next_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`: passed.

Follow-up:

- Once real connector opt-in evidence is recorded, add an explicit voice path for planning or requesting provider-backed dispatch without bypassing the dispatch gate.

## F3/DS7 - Shared Next Workpad Selection

Status: completed on 2026-05-26.

Decisions:

- Route `workpad next`, `workpad plan-next`, and `workpad start-next` through `ProjectDashboard::next_workpad_task()` and `next_workpad_candidate_count()`.
- Remove the duplicated CLI sorting/filtering helper so the workpad priority rule lives in `capo-query`, the same read-model surface consumed by voice and future dashboard clients.
- Preserve existing semantics: optional path filter, actionable observed markdown statuses, `observed_only` Capo execution status, and deterministic source ordering.

Verification:

- `cargo test -p capo-query next_actionable_workpad -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`: passed.

## F7/RR8 - Connectivity Exposure Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo connectivity exposure-evidence --exposure EXPOSURE_ID --out DIR` as a provider-free review artifact for recorded connectivity exposure state.
- Record the exported artifact as project-level evidence so dashboard and future dogfood migration checks can inspect remote-control exposure decisions without binding them to one agent session.
- Render only Capo connectivity metadata: endpoint, owner, channel, exposure scope, permission scope, status, health, reachability, linked grant, revocation time, and update sequence.
- Preserve the runtime/tunnel boundary: evidence export does not open tunnels, launch runtimes, launch provider CLIs, inspect credentials, materialize prompts, or mutate exposure state.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Once a real tunnel adapter exists, add a separate reachability evidence artifact instead of overloading this read-model review report.

## F7/RR9 - Latest Connectivity Exposure Status

Status: completed on 2026-05-26.

Decisions:

- Add shared exact/latest connectivity exposure selectors to `ProjectDashboard` so CLI, dashboard, voice, web, and mobile consumers can use one read-model rule.
- Add `capo connectivity exposure-status --exposure EXPOSURE_ID` for exact inspection and `--latest` with optional owner/channel filters for operator ergonomics.
- Select latest exposure by the newest projection sequence, with exposure ID as a deterministic tie breaker.
- Keep the command read-only and connectivity-boundary safe: no tunnel opening, runtime launch, provider launch, credential inspection, approval queueing, activation, revocation, or state mutation.

Verification:

- `cargo test -p capo-query latest_connectivity -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Voice and future mobile/web surfaces should use this selector for "latest remote-control exposure" questions instead of requiring exposure IDs.

## F6/V11 - Latest Connectivity Exposure Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a read-only voice intent for latest connectivity exposure questions so Capo can answer remote-control status questions conversationally.
- Support unscoped latest exposure questions plus scoped runtime-target, Capo-server, and channel filters using `ProjectDashboard::latest_connectivity_exposure(...)`.
- Render only connectivity metadata: exposure ID, endpoint, owner, channel, exposure scope, permission scope, status, health, reachability, linked grant, and revocation time.
- Preserve voice and connectivity safety boundaries: no raw transcript retention, no mutation, no tunnel opening, no runtime/provider launch, no credential inspection, no approval/activation/revocation, and no workpad edits.

Verification:

- `cargo test -p capo-voice latest_connectivity -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

Follow-up:

- Once real tunnel adapters exist, add a separate voice path for reachability evidence rather than overloading exposure metadata.

## F7/RR10 - Latest Connectivity Exposure Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Extend `connectivity exposure-evidence` with `--latest` plus optional owner/channel filters so reviewed remote-control evidence can be exported without copying exposure IDs.
- Reuse `ProjectDashboard::latest_connectivity_exposure(...)` for selection and the existing connectivity exposure evidence renderer/writer for artifact contents.
- Preserve the runtime/tunnel boundary: read projections and write Capo-owned evidence only; no tunnel opening, runtime/provider launch, credential inspection, approval request, activation, revocation, or exposure mutation.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Follow-up:

- Future voice/web/mobile evidence export should use the same selector if those surfaces request reviewed connectivity artifacts.

## F4/PT4 - ACP Client Capability Gating

Status: completed on 2026-05-26.

Decisions:

- Add `AcpClientCapabilityPlan` in `capo-tools` as the executable gate for ACP filesystem and terminal client capability advertisement.
- Require both a registered backing wrapper tool and an allowing `PermissionPolicy` decision before advertising `filesystem.read_text_file`, `filesystem.write_text_file`, or `terminal`.
- Treat missing wrappers as fail-closed even when the selected profile is trusted-local.
- Update static read-only/reviewer profiles to allow read-only wrapper invocation scopes for file read and git status/diff; continue denying file write and shell/terminal execution.
- Keep the helper provider-free and setup-only: no ACP agent launch, provider CLI launch, runtime start, tunnel opening, or credential/session inspection.

Verification:

- `cargo test -p capo-tools acp_client_capabilities -- --nocapture`: passed.
- `cargo test -p capo-tools static_read_only_policy_allows_read_tools_and_denies_writes -- --nocapture`: passed.

Follow-up:

- The ACP adapter/session setup path should consume this plan before advertising client capabilities.

## F4/PT5 - ACP Session Setup Capability Plan

Status: completed on 2026-05-26.

Decisions:

- Add `AcpAdapter::session_setup_plan(...)` as the adapter-facing setup scaffold for ACP client capability advertisement.
- Reuse `AcpClientCapabilityPlan` from `capo-tools`; the ACP adapter does not implement a parallel permission vocabulary.
- Include setup safety metadata in `AcpSessionSetupPlan`: protocol version, advertised capability list, per-capability decisions, MCP server count, credential policy, runtime-started flag, and provider-executed flag.
- Keep the setup plan provider-free and runtime-free. It does not launch ACP agents, provider CLIs, runtimes, or tunnels, and it does not inspect credential/session material.
- Keep MCP at zero advertised configs until Capo has a user-approved MCP config path.

Verification:

- `cargo test -p capo-adapters acp_session_setup -- --nocapture`: passed.

Follow-up:

- When the real ACP stdio client is added, use `AcpSessionSetupPlan` to construct the JSON-RPC initialize/session setup payload.

## F4/PT6 - ACP Client Handler Wrapper Routing

Status: completed on 2026-05-26.

Decisions:

- Add `AcpSessionSetupPlan::wrapper_request_for_client_call(...)` as the adapter-side routing seam from ACP client handler methods to Capo wrapper tool requests.
- Map `fs/read_text_file` to `capo.file_read`, `fs/write_text_file` to `capo.file_write`, and `terminal/run` to `capo.shell_run`.
- Refuse recognized methods when the setup plan did not advertise the matching capability. The advertised plan remains the authority for what the ACP agent may ask Capo to do.
- Keep routing provider-free and execution-free. It does not launch ACP agents, provider CLIs, runtimes, or tunnels; actual execution remains in `capo-tools` wrappers through controller/tool authorization.

Verification:

- `cargo test -p capo-adapters acp_client -- --nocapture`: passed.
- `cargo test -p capo-adapters acp_terminal -- --nocapture`: passed.

Follow-up:

- The future ACP stdio loop should call this routing seam for client handler requests, then invoke the returned wrapper request through the controller/tool boundary.

## F4/PT7 - Adapter Native Tool Observation Contract

Status: completed on 2026-05-26.

Decisions:

- Add `AdapterToolObservation` as the adapter-layer record for provider/ACP native tool updates that Capo did not execute through a wrapper.
- Derive observations from normalized adapter tool events and mark them `instrumentation_level=observed_only`.
- Preserve source adapter, external tool ref, tool name, observed status, raw event hash, and confidence for future state/query/evaluation ingestion.
- Use adapter timeline confidence to set observation confidence: stable -> high, heuristic -> medium, none -> low.
- Keep observed-only classification separate from governed `WrapperToolRequest`/tool invocation paths.

Verification:

- `cargo test -p capo-adapters adapter_tool_observations -- --nocapture`: passed.

Follow-up:

- Persist observed-only tool observations in the state/query layer once the read model grows a dedicated `ToolObservation` projection.

## F4/PT8 - Observed-Only Tool Observation State Projection

Status: completed on 2026-05-26.

Decisions:

- Add `ToolObservationProjection` and a `tool_observations` SQLite table for observed-only native tool facts.
- Keep observed-only native tool observations separate from governed `tool_calls`/invocations so dashboards and evaluations can label partial visibility correctly.
- Preserve session, optional tool-call link, source, external tool ref, tool name, observed status, instrumentation level, confidence, raw event hash, optional artifact, and sequence.
- Add `tool.observation_recorded` as the durable event kind for observation projection records.

Verification:

- `cargo test -p capo-state tool_observations -- --nocapture`: passed.

Follow-up:

- Surface `tool_observations` through `capo-query` and dashboard/session evidence views.

## F4/PT9 - Query And Evidence Visibility For Tool Observations

Status: completed on 2026-05-26.

Decisions:

- Add observed-only native tool observations to `SessionDashboardRow` in `capo-query`, keeping CLI/dashboard/voice/web/mobile consumers on one query contract.
- Render tool observations separately from governed tool calls in `capo dashboard` and session evidence exports. This preserves the distinction between Capo-executed wrapper tools and provider/adapter-native tool facts that Capo only observed.
- Keep the operator-visible fields sufficient for review and future evaluation: source, external ref, tool name, observed status, instrumentation level, confidence, raw event hash, and artifact ref.
- Keep the slice provider-free and execution-free. It reads existing projections and writes only evidence artifacts through the existing export path.

Verification:

- `cargo test -p capo-query project_dashboard_aggregates_agents_sessions_runs_evidence_and_events -- --nocapture`: passed.
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.

Follow-up:

- Future adapter fixture replay should append `ToolObservationProjection` rows automatically when normalized adapter events contain observed native tool updates.

## F4/PT10 - Adapter Replay Tool Observation Ingestion

Status: completed on 2026-05-26.

Decisions:

- Route normalized adapter-native tool events into both the timeline `ToolCall` projection and a separate observed-only `ToolObservationProjection`.
- Append `tool.observation_recorded` events during fixture replay so rebuilt state and dashboard/evidence views can distinguish native provider tool activity from Capo-governed wrapper execution.
- Use stable observation IDs based on adapter kind plus external tool/timeline reference. Started/completed updates for the same native tool project to one current observation row instead of duplicate UI entries.
- Preserve earlier tool names when later provider result updates omit the name, as Claude Code does for `tool_result` events.
- Keep replay provider-free and execution-free. No provider CLI, runtime, tunnel, credential/session inspection, or raw prompt/content persistence is introduced.

Verification:

- `cargo test -p capo-controller fixture_replay -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_fixture_replay_cli_exports_evidence_without_raw_provider_text -- --nocapture`: passed.

Follow-up:

- When real adapter streams are enabled, reuse the same observation projection helper for live stream ingestion rather than adding a separate native-tool observation path.

## F4/PT11 - Session Status Tool Introspection

Status: completed on 2026-05-26.

Decisions:

- Add governed tool-call and observed-only tool-observation rows to `capo session status --agent NAME`.
- Keep per-agent status aligned with dashboard/evidence terminology: `tool_call` rows are Capo-governed timeline/tool records, while `tool_observation` rows are adapter/provider-native facts with partial visibility.
- Render source, observed status, instrumentation level, confidence, external ref, artifact ref, and raw event hash for observations so operators can review native tool activity from the compact status surface.
- Keep status read-only over persisted projections. It does not launch provider CLIs, start runtimes, open tunnels, materialize prompts, inspect credentials/sessions, or mutate state.

Verification:

- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`: passed.

Follow-up:

- Consider moving per-session status assembly into `capo-query` once another consumer besides CLI needs the exact compact status contract.

## F6/V12 - Recent Work Tool Activity Conversation

Status: completed on 2026-05-26.

Decisions:

- Extend project-level and agent-level recent-work voice answers with governed tool-call and observed-only native tool-observation activity from `ProjectDashboard`.
- Keep voice output terminology aligned with dashboard/status/evidence: `spoken_tool_call` means Capo-governed tool timeline state, while `spoken_tool_observation` means adapter/provider-native activity with observed-only instrumentation.
- Add `tool_calls` and `tool_observations` to the voice read contract so future voice clients know the recent-work answer depends on tool activity fields.
- Preserve the voice safety boundary. Recent-work questions remain read-only, do not retain raw transcripts, do not mutate state, and do not launch providers, runtimes, tunnels, prompt materialization, or credential/session inspection.

Verification:

- `cargo test -p capo-voice recent_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.

Follow-up:

- If recent-work answers become too verbose, add a query-level compact activity summary instead of making voice apply its own filtering rules.

## F1/AC29 - Dispatch Tool Observation Evidence

Status: completed on 2026-05-26.

Decisions:

- Dispatch-chain evidence now includes observed-only native tool observations from the dispatch plan's Capo session when fixture replay records them.
- Keep the evidence section explicitly about observed tool activity, not governed Capo tool calls. Governed tool calls remain in the session/evidence surfaces; this slice makes adapter/provider-native tool use reviewable from dispatch evidence.
- Render only metadata: observation ID, tool name, adapter-event source, observed status, instrumentation level, confidence, external ref, artifact ref, and raw-event hash.
- Preserve the existing redaction boundary. Dispatch evidence still excludes raw dispatch prompts, raw provider fixture text, provider output, and tool input/output.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Follow-up:

- Once real opt-in local dispatch runs are authorized, verify that successful provider execution outcomes either produce structured observed-only tool observations or clearly report that the provider stream did not expose safe tool metadata.

## F6/V13 - Explicit Tool Activity Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a distinct voice intent for explicit tool-activity questions instead of relying on recent-work phrasing to carry that operator workflow.
- Support both project-level questions such as "What tools have my agents used?" and agent-scoped questions such as "What tools has fake-codex used?"
- Keep the read path over shared `ProjectDashboard` session rows. Voice does not perform its own state reads or infer hidden tool activity.
- Preserve the governed-vs-observed split in voice output: `spoken_tool_call` remains Capo-governed timeline/tool state, while `spoken_tool_observation` remains adapter/provider-native observed-only activity.
- The fake controller already creates `capo.session_summary` tool calls for seeded agents; tests intentionally count those governed calls alongside the extra observed-only fixture row.

Verification:

- `cargo test -p capo-voice tool_activity -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.

## F6/V14 - Adapter Smoke Status Conversation

Status: completed on 2026-05-26.

Decisions:

- Add a voice intent for connector smoke-report status so Capo can answer whether Codex/Claude connector proof is recorded, blocked, or still missing.
- Support exact smoke-report IDs and latest smoke-report selection, with optional adapter filtering for Codex or Claude.
- Answer from shared `ProjectDashboard` selectors instead of adding voice-specific state reads.
- Render connector readiness metadata and no-side-effect markers only. The voice answer does not render smoke stdout/stderr, raw prompts, provider output, tokens, cookies, or subscription session material.
- Preserve the provider boundary: the voice query does not launch provider CLIs, materialize prompts, open tunnels, inspect credentials, request approvals, activate grants, retain raw transcripts, or mutate connector state.

Verification:

- `cargo test -p capo-voice adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli voice_adapter_smoke -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F3/DS8 - Shared Tool Activity Summary

Status: completed on 2026-05-26.

Decisions:

- Move compact tool-activity totals into `capo-query` as `ProjectDashboard::tool_activity_summary(...)`.
- The summary counts agents, active sessions, governed tool calls, and observed-only native tool observations for either the whole project or one named agent.
- Voice tool-activity rendering now consumes the query summary instead of computing totals in CLI code. Detailed tool rows still come from the existing session dashboard rows.
- Keep the summary read-model-derived and side-effect-free. It does not inspect providers, runtimes, credentials, prompt materialization, tunnels, or raw tool input/output.

Verification:

- `cargo test -p capo-query project_dashboard_aggregates_agents_sessions_runs_evidence_and_events -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.

## F3/DS9 - Dashboard Tool Activity Summary

Status: completed on 2026-05-26.

Decisions:

- Render `ProjectDashboard::tool_activity_summary(None)` in `capo dashboard` as project-wide tool activity totals.
- Keep the summary separate from per-session rows: aggregate counts support quick operator scanning, while existing detailed rows remain the audit surface.
- Summary fields are `tool_activity_agents`, `tool_activity_active_sessions`, governed `tool_calls`, and observed-only `tool_observations`.

Verification:

- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`: passed.

## F2/DB8 - Dogfood Readiness Component Refs

Status: completed on 2026-05-26.

Decisions:

- Extend `ProjectDogfoodReadiness` with component refs so the readiness answer points at persisted evidence instead of only counts and booleans.
- Record four ref groups: connector smoke report IDs, workpad task IDs, dispatch chain IDs, and project evidence IDs.
- Render refs in `capo dogfood readiness`, dogfood readiness evidence artifacts, and read-only voice readiness answers.
- Keep refs metadata-only. Raw dispatch prompts, provider output, credential/session material, and source markdown bodies remain excluded.

Verification:

- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.

## F3/DS10 - Dashboard Dogfood Readiness Component Refs

Status: completed on 2026-05-26.

Decisions:

- Render dogfood readiness component refs in `capo dashboard` from the same shared readiness query used by the readiness command and voice.
- Include connector evidence refs, workpad task refs, dispatch chain refs, and project evidence refs in the project readiness summary line.
- Keep dashboard refs metadata-only. Raw dispatch prompts, provider output, credential/session material, tunnel details, and source markdown bodies remain excluded.

Verification:

- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

## F3/DS11 - Dashboard Latest Adapter Smoke Summary

Status: completed on 2026-05-26.

Decisions:

- Render latest adapter smoke-report shortcuts in `capo dashboard` for any adapter, Codex, and Claude.
- Source the rows from `ProjectDashboard::latest_adapter_smoke_report(...)` so dashboard, CLI status, and voice share selector semantics.
- Keep latest rows as scanning shortcuts only. The full smoke-report list and evidence exports remain the audit surface.
- Keep the dashboard metadata-only and read-model-derived. It does not launch provider CLIs, inspect credentials, materialize prompts, open tunnels, request approvals, activate grants, render smoke stdout/stderr, or mutate state.

Verification:

- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.

## F4/PT12 - Git Commit Wrapper

Status: completed on 2026-05-26.

Decisions:

- Add `capo.git_commit` to the governed runtime wrapper catalog as a high-risk wrapper for committing already-staged workspace changes.
- Keep staging and push out of scope. The wrapper only runs `git commit` with an explicit message through `LocalProcessRunner`.
- Reject empty or control-character commit messages before invoking the runtime.
- Keep static read-only/reviewer profiles denied because they do not include `git:commit:workspace`; trusted-local remains the broad local prototype profile that can invoke it.
- Preserve the existing wrapper audit/artifact path: input artifact, runtime stdout/stderr artifacts, permission decision, grant use, invocation started, output observed, completion, and delivery events.

Verification:

- `cargo test -p capo-tools git_commit -- --nocapture`: passed.

## F4/PT13 - Wrapper Tool CLI Surface

Status: completed on 2026-05-26.

Decisions:

- Add `capo tool run-wrapper` as the first direct CLI operator surface over governed runtime wrapper tools.
- Require explicit workspace and artifact roots for every invocation.
- Default the surface to `read-only-local`; trusted-local must be requested explicitly for mutating/high-risk wrapper execution.
- Render the wrapper result contract: permission effect/source, input artifact, output artifact rows, audit lifecycle events, and summary.
- Keep the command provider-free and state-light. It invokes `RuntimeToolWrappers` directly, does not launch provider CLIs, does not inspect credential/session material, does not open tunnels, does not materialize prompts, and does not write wrapper results into Capo state projections.

Verification:

- `cargo test -p capo-cli tool_run_wrapper -- --nocapture`: passed.

## F4/PT14 - Recorded Wrapper Tool Invocations

Status: completed on 2026-05-26.

Decisions:

- Add `--record` to `capo tool run-wrapper` so direct operator wrapper executions can be persisted as Capo-governed tool activity.
- Persist wrapper input/output artifacts with project/session/run ownership before appending the tool-call projection.
- Project recorded wrapper runs under a Capo-owned synthetic `cli-wrapper` agent/session/run instead of attaching them to a provider agent. This keeps the existing dashboard/session/tool-activity query path useful without misattributing operator-invoked tools to Codex or Claude.
- Render `recorded=true`, `tool_call`, `session_id`, `run_id`, and `recorded_sequence` in CLI output when recording succeeds.
- Preserve unrecorded wrapper runs for quick diagnostics. Recording remains opt-in.
- Keep the path provider-free and tunnel-free. It does not launch provider CLIs, inspect subscription credentials/sessions, materialize prompts, open tunnels, or persist raw provider output.

Verification:

- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p capo-cli tool_run_wrapper -- --nocapture`: passed.
- `cargo test`: passed.

## F7/RR19 - Latest Runtime Target Status

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::latest_runtime_target(...)` so CLI, voice, web, and mobile clients can reuse one latest-target selector instead of duplicating target ordering and filters.
- Extend `capo runtime target status` with `--latest`, plus optional `--runner` and `--status` filters. Exact `--target` lookup remains available and mutually exclusive with latest lookup.
- Keep the selector read-model-derived and metadata-only. It does not launch runtimes, launch provider CLIs, open tunnels, inspect credentials, request approvals, activate grants, retain raw transcripts, or mutate runtime target state.
- Allow the query selector to match legacy underscore runner labels and current hyphenated CLI labels so older target rows remain inspectable.

Verification:

- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p capo-query runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test`: passed.

## F6/V15 - Latest Runtime Target Status Conversation

Status: completed on 2026-05-26.

Decisions:

- Add `VoiceReadScope::ProjectLatestRuntimeTargetStatus` so voice can answer latest runtime placement/status questions through the shared dashboard query instead of requiring a target ID.
- Support optional voice filters for runner kind and target status using the same vocabulary as the CLI selector: `local-process`, `remote-process`, `container`, `available`, `disabled`, and `unhealthy`.
- Keep the voice path read-only and transcript-safe. It does not retain raw transcripts, launch runtimes, run provider CLIs, open tunnels, inspect credentials, request approvals, activate grants, edit workpads, or mutate state.

Verification:

- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p capo-voice runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test`: passed.

## F2/DB9 - Runtime Target Dogfood Readiness

Status: completed on 2026-05-26.

Decisions:

- Add runtime target readiness to `ProjectDogfoodReadiness` so the dogfood gate tracks execution placement separately from connector proof, workpad bridge state, and dispatch-chain state.
- Require at least one `available` runtime target before reporting `ready_for_first_dogfood`. Disabled and unhealthy targets remain visible through target status surfaces but do not clear the gate.
- Render runtime target readiness, counts, and refs through CLI readiness, dashboard readiness, voice dogfood answers, and readiness evidence artifacts.
- Keep the check read-model-derived and metadata-only. It does not launch runtime processes, launch provider CLIs, open tunnels, inspect credentials, materialize prompts, request approvals, activate grants, retain raw transcripts, or edit markdown.

Verification:

- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.
- `cargo test`: passed.

## F7/RR11 - Runtime Target Inventory

Status: completed on 2026-05-26.

Decisions:

- Add runtime targets as a first-class persisted read model for execution-machine metadata instead of relying on opaque `runtime_target` owner IDs in connectivity exposure rows.
- Keep the execution placement boundary separate from connectivity and provider dispatch. Runtime targets record runner kind, workspace/artifact roots, default cwd, capability profile, optional connectivity endpoint, and status; they do not represent a running process or an open tunnel.
- Add `capo runtime target register` and `capo runtime target list` as provider-free operator surfaces over the registry.
- Render runtime targets in `capo dashboard` through `ProjectDashboard.runtime_targets`, keeping CLI/dashboard/voice/web consumers on the shared query boundary.
- Preserve the safety boundary: no runtime process launch, provider CLI execution, credential/session inspection, tunnel opening, prompt materialization, or exposure activation.

Verification:

- `cargo test -p capo-state runtime_targets -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR12 - Runtime Target Exposure Validation

Status: completed on 2026-05-26.

Decisions:

- Fail closed before recording a `runtime_target` connectivity exposure unless the target exists in the runtime target inventory.
- Keep dry-run exposure planning available without requiring registered target state because it does not mutate Capo state.
- Keep `capo_server` exposure owners independent from runtime target registration.
- Preserve the boundary split: validation only connects exposure metadata to execution-machine metadata; it does not launch runtime processes, provider CLIs, tunnels, approvals, grants, or prompt materialization.
- Enforce this at the CLI/operator write surface for now. A future service/controller write path should enforce the same invariant if connectivity writes move out of the CLI.

Verification:

- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR13 - Runtime Target Endpoint Consistency

Status: completed on 2026-05-26.

Decisions:

- Treat a runtime target's configured `connectivity_endpoint_id` as a binding constraint for recorded `runtime_target` connectivity exposures.
- Fail closed if an operator tries to record a runtime-target exposure against a different endpoint.
- Keep targets without a configured endpoint flexible for now because real remote target discovery and concrete tunnel adapters are still deferred.
- Preserve the boundary split: this connects persisted runtime-target metadata to connectivity exposure metadata only; it does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, or prompt materialization.

Verification:

- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR14 - Runtime Target Availability Guard

Status: completed on 2026-05-26.

Decisions:

- Treat runtime target status as an exposure precondition for recorded `runtime_target` connectivity exposure rows.
- Fail closed for disabled or unhealthy runtime targets while keeping those targets visible in the inventory and dashboard.
- Keep target status update mechanics as future metadata work; this slice only enforces the status already stored on the target.
- Preserve the boundary split: this validates metadata before writing exposure rows and does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, or prompt materialization.

Verification:

- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR15 - Runtime Target Status Update Surface

Status: completed on 2026-05-26.

Decisions:

- Add `capo runtime target set-status` as a provider-free metadata command over the runtime target inventory.
- Record status changes as `runtime.target_status_changed` events while projecting into the existing `RuntimeTargetProjection` row.
- Preserve all target placement metadata during status changes; only `status` changes.
- Keep this separate from runtime lifecycle and connectivity exposure. The command does not start processes, open tunnels, inspect credentials, request approvals, activate grants, or mutate exposure rows.
- Use the existing exposure guard as the behavioral proof: a disabled target cannot be exposed until `set-status --status available` updates the target row.

Verification:

- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR16 - Runtime Target Status Query Surface

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::runtime_target_status(...)` as the shared exact selector for runtime target metadata.
- Add `capo runtime target status --target TARGET_ID` as a read-only operator surface over that selector.
- Render the same target metadata shape as runtime target list/dashboard rows and include explicit no-side-effect markers for provider CLI execution, tunnel opening, runtime process start, and state mutation.
- Keep missing target lookup fail-closed with a clear operator error.
- Preserve the boundary split: this query does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, or prompt materialization.

Verification:

- `cargo test -p capo-query runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR17 - Voice Runtime Target Status Query

Status: completed on 2026-05-26.

Decisions:

- Add a dedicated voice/input intent for runtime target status instead of overloading connectivity exposure status.
- Lower runtime target status questions into read-only command envelopes with `ProjectRuntimeTargetStatus` read scope.
- Render target placement/status metadata from the shared dashboard query selector: runner, workspace, artifact root, default cwd, capability profile, endpoint, status, and sequence.
- Return a spoken missing-target row for unknown target IDs.
- Preserve the boundary split: the voice query does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or state mutation.

Verification:

- `cargo test -p capo-voice runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR18 - Runtime Target Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo runtime target evidence --target TARGET_ID --out DIR` as a provider-free evidence export for one runtime target.
- Write a Capo-marked markdown artifact with guarded overwrite behavior and record it as project-level evidence.
- Use `kind=runtime_target_evidence` for both artifact and evidence rows so dashboards and future readiness checks can distinguish target placement evidence from connectivity exposure evidence.
- Treat runtime target evidence as reviewable placement/status metadata, not proof that a runtime process is live.
- Preserve the boundary split: the export does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or target-state mutation.

Verification:

- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

## F1/AC30 - Adapter Smoke Report Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::adapter_smoke_report_status(...)` as the shared exact selector for adapter smoke report metadata.
- Add `capo adapter smoke-report evidence --smoke-report SMOKE_REPORT_ID --out DIR` as a provider-free evidence export for connector proof or blocker records.
- Write a Capo-marked markdown artifact with guarded overwrite behavior and record it as project-level evidence.
- Use `kind=adapter_smoke_evidence` for both artifact and evidence rows so dashboards and future readiness checks can distinguish smoke proof/blocker artifacts from dispatch-chain evidence.
- Render metadata and artifact-root references only. The evidence artifact does not render smoke stdout/stderr, raw prompts, provider output, tokens, cookies, or subscription session material.
- Preserve the provider boundary: the export does not launch provider CLIs, materialize prompts, open tunnels, inspect credentials, request approvals, activate grants, or mutate connector state.

Verification:

- `cargo test -p capo-query adapter_smoke_report -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F1/AC31 - Adapter Smoke Report Status Query

Status: completed on 2026-05-26.

Decisions:

- Add shared exact and latest selectors for adapter smoke report metadata.
- Add `capo adapter smoke-report status --smoke-report SMOKE_REPORT_ID` and `capo adapter smoke-report status --latest [--adapter codex|claude]`.
- Keep exact ID lookup and latest lookup separate; adapter filtering is only valid for latest lookup.
- Render connector readiness metadata and no-side-effect markers only. The status command does not render smoke stdout/stderr, raw prompts, provider output, tokens, cookies, or subscription session material.
- Preserve the provider boundary: the query does not launch provider CLIs, materialize prompts, open tunnels, inspect credentials, request approvals, activate grants, or mutate connector state.

Verification:

- `cargo test -p capo-query adapter_smoke_report -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.

## F7/RR20 - Latest Runtime Target Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo runtime target evidence --latest [--runner ...] [--status ...] --out DIR` as a provider-free latest-selector evidence export.
- Reuse `ProjectDashboard::latest_runtime_target(...)` so status, voice, and evidence surfaces resolve latest runtime targets consistently.
- Reuse the existing Capo-marked runtime target evidence artifact and guarded writer instead of adding a second evidence format.
- Keep exact target and latest selector modes mutually exclusive; runner/status filters are valid only with `--latest`.
- Preserve the boundary split: the export records project evidence from read models and does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or target-state mutation.

Verification:

- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.

## F7/RR21 - Runtime Target Control Readiness

Status: completed on 2026-05-26.

Decisions:

- Add `ProjectDashboard::runtime_target_control_readiness(...)` as the shared query contract for target control readiness.
- Add `capo runtime target readiness --target TARGET_ID` as a read-only operator command.
- Define readiness as target `available` plus latest runtime-target-owned `control` exposure `active` and reachable.
- Report blockers and next actions from read models so operators can distinguish missing exposure, pending permission, revoked exposure, unhealthy target, and ready states.
- Keep runtime target inventory and connectivity exposure rows separate. The readiness view aggregates them for operator ergonomics but does not mutate either source model.
- Preserve the boundary split: the query does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or state mutation.

Verification:

- `cargo test -p capo-query runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F6/V16 - Runtime Target Control Readiness Conversation

Status: completed on 2026-05-26.

Decisions:

- Add `VoiceIntentKind::RuntimeTargetReadiness` and `VoiceReadScope::ProjectRuntimeTargetControlReadiness`.
- Recognize read-only questions such as "Is runtime target remote target 1 ready for remote control?"
- Render readiness from `ProjectDashboard::runtime_target_control_readiness(...)` so voice uses the same target/exposure aggregation as CLI and future UI surfaces.
- Return target readiness, control exposure readiness, blocker codes, and next action without retaining the raw transcript.
- Preserve the boundary split: the voice query does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, workpad edits, or state mutation.

Verification:

- `cargo test -p capo-voice runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F3/DS12 - Dashboard Runtime Target Control Readiness

Status: completed on 2026-05-26.

Decisions:

- Render runtime target control-readiness rows in `capo dashboard` next to each runtime target row.
- Source each row from `ProjectDashboard::runtime_target_control_readiness(...)` so dashboard, CLI status, and voice share the same target/exposure readiness contract.
- Keep runtime target rows and connectivity exposure rows visible separately for audit. The readiness row is an aggregate operator shortcut, not a replacement for source projections.
- Preserve the boundary split: dashboard rendering does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or state mutation.

Verification:

- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F7/RR22 - Runtime Target Control Readiness Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo runtime target readiness-evidence --target TARGET_ID --out DIR` as a provider-free evidence export for the aggregate target/control-exposure readiness state.
- Write a Capo-marked markdown artifact with guarded overwrite behavior and record it as project-level evidence.
- Use `kind=runtime_target_readiness_evidence` for both artifact and evidence rows so dashboards and future readiness checkpoints can distinguish aggregate readiness artifacts from runtime target placement evidence and connectivity exposure evidence.
- Preserve the boundary split: the export records project evidence from read models and does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or target/exposure state changes.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F7/RR24 - Latest Runtime Target Control Readiness Evidence Export

Status: completed on 2026-05-26.

Decisions:

- Add `capo runtime target readiness-evidence --latest [--runner ...] [--status ...] --out DIR` as a provider-free latest-selector export for aggregate target/control-exposure readiness.
- Reuse `ProjectDashboard::latest_runtime_target(...)` before deriving `ProjectDashboard::runtime_target_control_readiness(...)`, matching `runtime target readiness --latest`.
- Reuse the existing Capo-marked readiness evidence artifact and guarded writer instead of adding a second evidence format.
- Keep exact target and latest selector modes mutually exclusive; runner/status filters are valid only with `--latest`.
- Preserve the boundary split: the export records project evidence from read models and does not launch runtime processes, provider CLIs, tunnels, approvals, grants, credentials, prompt materialization, raw transcript retention, or target/exposure state changes.

Verification:

- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.

## F8/SS1 - State Store Library Decision

Status: completed on 2026-05-26.

Decisions:

- Manual `rusqlite` SQL was appropriate for the first SQLite event-store scaffold, but it is now a resilience risk because each projection family repeats schema DDL, upsert SQL, row mapping, projection-log encoding/decoding, read queries, and rebuild coverage.
- Do not keep adding broad state-model surfaces with ad hoc SQL before a hardening pass.
- Keep `rusqlite` in place for now so ongoing feature work is not blocked by an abrupt persistence migration.
- Evaluate Diesel first for a contained projection-family spike because Capo is currently sync, local SQLite-first, schema-sensitive, and Rust-first. Diesel's schema-aware query builder and migration tooling fit those constraints better than an async-first library at this point.
- Keep SQLx as the second candidate if Capo's server/Postgres path becomes async-first. SQLx keeps SQL visible and supports compile-time checked queries, but it would force an async-state discussion earlier.
- Defer SeaORM for the controller core because Capo's state shape is append-only events plus rebuildable projections, not primarily active-record CRUD.
- Keep a typed in-house `rusqlite` projection registry as the fallback if Diesel proves too invasive. The registry should centralize projection descriptors, DDL/upsert/read mapping, projection-log codecs, and rebuild tests.

Verification:

- Documentation-only decision. `git diff --check`: passed.

## F8/SS2 - State Crate Test Module Split

Status: completed on 2026-05-26.

Decisions:

- Treat file-size reduction as architecture work because very large files make Capo harder for LLMs and humans to navigate.
- Follow the split-review recommendation: keep crate boundaries stable and decompose modules internally first.
- Start with `capo-state` tests because moving the inline test module is the safest split. It reduces `lib.rs` size without changing event append, schema migration, projection encoding, rebuild behavior, or public projection type names.
- Keep crate-root APIs stable. Downstream crates currently import `capo-state` projection and store types directly, so deeper splits should use `pub use` rather than moving public names out of reach.
- The first mechanical move reduced `crates/capo-state/src/lib.rs` to 6,109 lines and isolated the former inline tests in `crates/capo-state/src/tests.rs` at 1,876 lines.

Verification:

- `git diff --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo fmt --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2a - State Event And Error Module Split

Status: completed on 2026-05-26.

Decisions:

- Continue the state-crate decomposition with stable boundary vocabulary rather than deeper SQL helpers first.
- Move event envelope, redaction, artifact, recovery, and state-error types into focused modules while preserving crate-root re-exports. This keeps downstream imports stable and reduces risk before touching schema, projection codecs, or read queries.
- Treat event kind strings as persisted compatibility data. This slice moved the `EventKind` definition without changing any `as_str()` output.
- Keep `error.rs` separate from `event.rs` even though `StateError` references `RedactionState`; errors are a cross-cutting store API concern, not event payload vocabulary.

Verification:

- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2e - State Projection Apply Module Split

Status: completed on 2026-05-26.

Decisions:

- Move read-model mutation SQL and projection watermark updates into `apply.rs` as the projection runtime apply boundary.
- Keep projection-log encode/decode in `codec.rs`; row compatibility and read-model mutation are separate concerns.
- Keep event append and `insert_projection_record` in `lib.rs` for now because they are part of appending events and recording projection-log entries, not applying records into read-model tables.
- Preserve schema, projection codec behavior, rebuild semantics, query behavior, and crate-root public APIs.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI1 - CLI Test Module Split

Status: completed on 2026-05-26.

Decisions:

- Move the large inline `capo-cli` test module into `src/tests.rs`.
- Keep `main.rs` as the runtime CLI implementation for this slice. Command routing, provider-safety guards, renderers, and helper functions remain unchanged.
- Use this as the first CLI maintainability slice before deeper command-family extraction. The test split is behavior-preserving and gives future command splits a smaller runtime file to inspect.
- Keep the tests as a sibling module using `super::*` so existing private helper coverage remains intact.

Verification:

- `cargo test -p capo-cli`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI2 - CLI Surface Parsing Module Split

Status: completed on 2026-05-26.

Decisions:

- Move help text, global `--state` parsing, and generic flag/option helpers into `cli_surface.rs`.
- Keep command routing and command-family implementations in `main.rs` for this slice. This separates the common CLI surface from command behavior without forcing a broad command-module refactor.
- Expose `ParsedArgs` and argument helpers as `pub(crate)` only. They are internal CLI implementation details, not library APIs.
- Preserve the existing `capo --help`, `--state`, and command output contract.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-cli`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI3 - Runtime Target Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move runtime target command handling into `runtime_target.rs`: registration, list/status/readiness, status changes, shared render helpers, and runtime target parsers.
- Move runtime target readiness/evidence exports into `runtime_target_evidence.rs`: evidence selection, markdown rendering, artifact/evidence records, and guarded evidence writers. This keeps both runtime target modules in the preferred 300-500 LOC range instead of leaving a near-warning-zone command file.
- Keep connectivity ownership validation in `main.rs` because it belongs to the connectivity exposure command path, even though it reads runtime target state.
- Re-export only the runtime target render helpers needed by dashboard and voice rendering. Command functions remain `pub(crate)` for `run_cli` routing.
- Keep shared CLI primitives in `main.rs`/`cli_surface.rs` for now (`state`, `envelope`, `project_id`, `stable_cli_hash`, `escape_json`, and argument parsing) so this slice does not force a broad CLI framework abstraction.

Verification:

- `cargo test -p capo-cli runtime_target -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## Cross-Cutting - LLM-Friendly File Boundaries

Status: adopted on 2026-05-26.

Decisions:

- Treat source files around 300-500 LOC as the ideal target when splitting does not disrupt the current task.
- Treat 800-1,000+ LOC as a refactor-soon warning zone, and 1,500+ LOC as unacceptable unless the file is generated, highly mechanical, or a temporary test fixture.
- Split by responsibility and edit surface, not arbitrary chunks. A file should have one conceptual purpose and be understandable in one pass by a human or coding agent.
- Keep active workpads short and cockpit-like. Move accumulated background, canonical decisions, invariants, and open questions into layered linked docs when markdown grows past comfortable working size.
- Test files may be larger than source modules, but split them by scenario or command family when they stop being locally navigable.

## F9/CLI4 - Connectivity Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move connectivity exposure command handling into `connectivity.rs`: exposure planning, runtime-target owner validation, approval request, activation, revocation, status queries, grant matching, subject/scope helpers, and channel/exposure parsing.
- Move connectivity exposure evidence export into `connectivity_evidence.rs`: latest/exact evidence selection, artifact/evidence persistence, markdown rendering, and guarded Capo-owned evidence writes.
- Keep dashboard and voice connectivity rendering in `main.rs` for this slice because those functions belong to broader dashboard/voice command surfaces and already format multiple feature families.
- Reuse the existing shared `scope_values` helper from `main.rs` rather than introducing a second parser in the connectivity module.

Verification:

- `cargo test -p capo-cli connectivity_exposure -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F10/Q1 - Query Test Module Split

Status: completed on 2026-05-26.

Decisions:

- Move the `capo-query` test module from `lib.rs` into `tests.rs` using Rust's module-file convention.
- Keep tests as a child module of `lib.rs`, not integration tests, so they can continue exercising private helpers such as dashboard selection and status-ranking behavior.
- Do not split production query structs or aggregation yet. This slice is a mechanical reduction before deeper dashboard/readiness concern separation.

Verification:

- `cargo test -p capo-query`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F11/T1 - Tools Test Module Split

Status: completed on 2026-05-26.

Decisions:

- Move the `capo-tools` test module from `lib.rs` into `tests.rs` using Rust's module-file convention.
- Keep tests as a child module of `lib.rs`, not integration tests, so wrapper authorization and private helper behavior remain directly testable.
- Defer production splitting until the wrapper, registry, and permission-policy boundaries can be separated without mixing behavior changes into this mechanical slice.

Verification:

- `cargo test -p capo-tools`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F11/T2 - Tools Permission Policy Module Split

Status: completed on 2026-05-26.

Decisions:

- Move `PermissionPolicy`, fake/trusted/static policy implementations, `PermissionRequest`, `PermissionDecision`, JSON scope parsing, and scoped grant ID helpers into `permission.rs`.
- Re-export the permission API from the crate root so `capo-adapters`, `capo-controller`, `capo-cli`, and tests keep the same import surface.
- Keep ACP capability gating in `lib.rs` for this slice because it depends on tool definitions and wrapper catalog checks, even though it consumes `PermissionPolicy`.
- Keep `content_hash` in `lib.rs`; only the permission-specific stable grant hash moved with the policy implementation.

Verification:

- `cargo test -p capo-tools`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F11/T3 - Tools Runtime Wrapper Module Split

Status: completed on 2026-05-26.

Decisions:

- Move runtime wrapper config, wrapper executor, wrapper request/result/artifact records, workspace path guards, redaction, runtime artifact conversion, and wrapper input hashing into `runtime_wrappers.rs`.
- Re-export the wrapper API from the crate root so adapters, controller, CLI, and tests keep the same import surface.
- Keep shared `ToolDefinition`, `ToolAuthorization`, `ToolAuditEvent`, Capo registry behavior, ACP capability planning, and the wrapper tool catalog in `lib.rs` for this slice because they are used across registry and wrapper concerns.
- Keep `runtime_wrappers.rs` as a single wrapper concern for now even though it is above the ideal range. A later split can separate wrapper request/result types from execution helpers without changing behavior.

Verification:

- `cargo test -p capo-tools`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI5 - Adapter Smoke Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter smoke-report command handling into `adapter_smoke.rs`: report recording, exact/latest status, exact/latest evidence export, artifact-root credential scan, smoke evidence rendering, guarded Capo-owned evidence writes, and smoke scan error formatting.
- Keep adapter dogfood gate and dispatch execution in `main.rs` for this slice. They depend on broader dashboard/readiness and dispatch-chain behavior, while the smoke-report surface is a smaller cohesive operator command family.
- Reuse crate-root shared helpers for command envelopes, project IDs, state access, adapter label normalization, stable hashes, escaping, and debug error formatting.
- Keep `scan_dispatch_artifacts_or_delete` in `main.rs`, but use the smoke module's public scan-error formatter so dispatch cleanup behavior remains unchanged.

Verification:

- `cargo test -p capo-cli adapter_smoke -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI6 - Adapter Dogfood Gate Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter dogfood gate command handling into `adapter_dogfood.rs`: status rendering, gate evidence export, evidence markdown rendering, confidence scoring, and guarded Capo-owned evidence writes.
- Keep the gate renderer exported for dashboard reuse, so the CLI dashboard and `adapter dogfood-gate` command continue sharing one output contract.
- Leave project-wide dogfood readiness and adapter dispatch gating in `main.rs` for now. They compose additional readiness, workpad, runtime target, and dispatch-chain concepts beyond connector gate evidence.
- Reuse crate-root helpers for command envelopes, project IDs, state access, stable hashes, escaping, list formatting, and debug errors.

Verification:

- `cargo test -p capo-cli adapter_dogfood -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F9/CLI7 - Adapter Dispatch Status And Evidence Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter dispatch gate, dispatch status, and dispatch evidence command handling into `adapter_dispatch.rs`.
- Keep dispatch gate projection, dispatch status rendering, dispatch evidence rendering, confidence scoring, and guarded dispatch evidence writes private inside the module. The module exposes only the three CLI command handlers needed by `main.rs`.
- Leave execution request, prompt materialization, run preflight, local run execution, fixture replay, dashboard rendering, and voice summaries in `main.rs` for this slice. Those functions compose runtime execution and replay behavior beyond the read/status/evidence surface moved here.
- Accept a 542-line module for now because it is one conceptual unit and keeps the dispatch evidence redaction policy next to the command that writes it. The next split should target the execution-request/prompt-materialization/local-run family.
- Review noted that dispatch concepts remain split across `adapter_dispatch.rs` and `main.rs`. This is accepted for CLI7 because the current module is the status/evidence surface; the follow-up execution/replay split should remove that residual ambiguity.

Verification:

- `cargo test -p capo-cli adapter_dispatch -- --nocapture`: passed.
- `cargo check -p capo-cli`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no behavior or helper-visibility issues found; residual cohesion risk accepted as the next split target.

## F9/CLI8 - Adapter Dispatch Preparation And Local Run Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter dispatch execution-request, prompt materialization, run preflight, opt-in environment mapping, prompt source validation, and redacted prompt materialization rendering into `adapter_dispatch_prepare.rs`.
- Move adapter dispatch local provider execution, subscription-safe launch-plan construction, dispatch artifact secret scanning/deletion, execution projection, and execution event recording into `adapter_dispatch_run.rs`.
- Keep dispatch replay in `main.rs` for this slice. Replay applies normalized fixture events through the controller and is closer to adapter fixture replay than runtime provider execution.
- Keep dashboard and voice summaries in `main.rs` for this slice because they aggregate multiple feature families rather than owning dispatch execution state.
- Split the first extraction into two modules after seeing that a single execution module would be 889 lines. The final split keeps `adapter_dispatch_prepare.rs` at 552 lines and `adapter_dispatch_run.rs` at 384 lines, closer to the LLM-friendly target.

Verification:

- `cargo test -p capo-cli adapter_dispatch -- --nocapture`: passed.
- `cargo test -p capo-cli prototype_e2e_smoke -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dispatch_status -- --nocapture`: passed.
- `cargo check -p capo-cli`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; residual risk is that real provider CLI dispatch remains manually opt-in and not run by default gates.

## F9/CLI9 - Tool Wrapper Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move the CLI-owned `tool run-wrapper` surface into `tool_wrapper.rs`: option validation, wrapper tool aliases, JSON input shaping, CLI policy selection, wrapper invocation, artifact rendering, and state projection recording.
- Keep the lower-level wrapper implementations in `capo-tools`; this split only moves the CLI adapter surface that turns command-line input into governed wrapper requests.
- Keep command routing and dashboard rendering in `main.rs`. The dashboard reads shared query state and should not own wrapper invocation behavior.
- Preserve the `tool_origin=capo_wrapper` instrumentation and the `--record` gate for writing wrapper artifacts/events.
- Add explicit test imports for `RunId` and dispatch artifact scan helpers now that tests no longer inherit those names through `main.rs` root imports.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli tool_run_wrapper -- --nocapture`: passed.
- `cargo test -p capo-tools -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; permission defaults, trusted-local requirement, `--record` behavior, artifact/event recording, and `tool_origin=capo_wrapper` were preserved.

## F9/CLI10 - Adapter Fixture Replay Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter fixture replay into `adapter_replay.rs`: generic adapter fixture replay, dispatch fixture replay, adapter fixture parsing, parse-error rendering, and adapter label normalization.
- Keep replay provider-safe by preserving fixture-only execution. Replay applies normalized fixture events through the controller and records dispatch replay metadata with `provider_cli_executed=false` and `raw_content_policy=content_hashed_not_rendered`.
- Make `adapter_label` crate-visible because adapter smoke reports already share the same adapter normalization vocabulary. This avoids duplicating Codex/Claude/ACP label aliases across modules.
- Make `controller` and `export_evidence` crate-visible so replay can chain into existing controller and evidence surfaces without duplicating those workflows.
- Review noted that importing `adapter_label` from `adapter_replay.rs` into `adapter_smoke.rs` is acceptable for this slice but could become a cohesion smell if adapter normalization spreads further. A future adapter/common helper split should move shared label normalization if another module needs it.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli adapter_fixture_replay -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-adapters -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; provider execution stays false, raw content policy stays `content_hashed_not_rendered`, and replay outputs counts/hashes rather than raw provider text.

## F9/CLI11 - Adapter Readiness And Launch Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter readiness and launch planning command handling into `adapter_launch.rs`.
- Keep readiness projection, dispatch-plan projection, prompt-source projection, local adapter validation, and launch/readiness rendering with the command surface because they define the pre-execution connector proof contract.
- Export only the adapter readiness/launch entrypoints, dispatch-plan composition types, validation helper, and dispatch-plan renderer needed by current CLI routing and `workpad plan-next`.
- Keep replay, dogfood readiness, dashboard aggregation, and voice summaries out of this module.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-adapters -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; residual gap is that this slice preserves existing provider-free planning behavior and still does not run real subscription-backed provider CLIs.

## F9/CLI12 - Workpad Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move the workpad command family into `workpad.rs`: index, next, plan-next, start-next, import, propose, apply guard, workpad projection conversion, deterministic task IDs, and proposal artifact rendering/writing.
- Keep markdown source safety with the workpad commands. Proposal generation still writes only Capo-owned artifacts, refuses changed/non-Capo proposal overwrites, and leaves source markdown writeback disabled behind the explicit apply guard.
- Keep `workpad_task_goal` in `workpad.rs` and export it for dispatch preparation/local-run code so workpad-derived prompt materialization uses the same canonical prompt text as `plan-next`.
- Keep dashboard and voice rendering in `main.rs` for this slice; they aggregate multiple feature families but reuse exported workpad helpers.
- Accept `workpad.rs` at 728 lines for now because it is one conceptual unit. A later split can separate proposal artifact writing from read-only index/next handling if this file grows further.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-workpads -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; residual gap is that this split preserves existing fake/local start-next dispatch behavior and does not add real provider execution.

## F9/CLI13 - Dashboard Module Split

Status: completed on 2026-05-26.

Decisions:

- Move dashboard command handling into `dashboard.rs`: dashboard filter parsing, shared `ProjectDashboardQuery` execution, dashboard text rendering, and latest adapter smoke report summaries.
- Keep dashboard read-only. The module opens state only to run the shared query surface and does not append events, write artifacts, or execute providers.
- Reuse existing feature renderers from their owner modules for runtime target rows and adapter dogfood gate rows. This keeps dashboard as an aggregator instead of duplicating feature-specific formatting rules.
- Keep voice summaries in `main.rs` for this slice because voice is a separate conversational input/control surface, even though it reads the same dashboard query.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli dashboard -- --nocapture`: passed.
- `cargo test -p capo-query project_dashboard -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; residual gap is that dashboard output compatibility remains assertion-based rather than byte-for-byte golden snapshots.

## F9/CLI14 - Voice Command And Render Split

Status: completed on 2026-05-26.

Decisions:

- Move voice command/control handling into `voice.rs`: transcript option parsing, retention policy selection, visible approval queue/decision, reviewed redacted summary ingestion, read-only dashboard query execution, and confirmed controller mutations.
- Split pure voice output rendering into `voice_render.rs` so spoken read-contract formatting, labels, and per-surface status summaries are separate from the mutation/approval flow.
- Keep `capo-voice` as the intent-planning crate. The CLI voice modules remain the local command adapter that turns a `VoiceCommandPlan` into state reads, approval events, controller calls, and rendered output.
- Make the shared permission decision primitives crate-visible because both the generic permission CLI and voice approval path use the same ACP-style decision vocabulary.
- Preserve the no-raw-transcript default and require `--reviewed-summary` before redacted summary memory ingestion.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli voice -- --nocapture`: passed.
- `cargo test -p capo-voice -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_redacted_summary -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; residual gap is that voice output compatibility remains covered by assertions rather than full golden snapshots.

## F9/CLI15 - Evidence And Review Module Split

Status: completed on 2026-05-26.

Decisions:

- Move session evidence export, task-outcome report export, and review finding recording into `evidence.rs`.
- Keep review outcome derivation, evidence/review markdown rendering, and guarded evidence/review artifact writers with the command surfaces that use them.
- Keep dogfood readiness artifact writing in `main.rs` because it belongs to project readiness, not session evidence/review.
- Keep session status rendering in `main.rs` for this slice because it is CLI observation output, not artifact export.
- Update adapter fixture replay to depend on `evidence::export_evidence` explicitly for replay evidence chaining.

Verification:

- `cargo check -p capo-cli`: passed.
- `cargo test -p capo-cli evidence_export_handles_completed_runs_and_refuses_foreign_files -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_review_findings -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_renders_task_outcome_reports -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_fixture_replay_cli_exports_evidence_without_raw_provider_text -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no issues found; output strings, overwrite guards, idempotency keys, redaction states, and adapter replay evidence chaining were preserved.

## F9/CLI16 - Permission Command Module Split

Status: completed on 2026-05-26.

Decisions:

- Move permission approval queue/list/decision commands into `permission.rs`.
- Keep ACP-style decision mapping (`allow_once`, `allow_always`, `reject_once`, `reject_always`), durable `allow_always` scope validation, approval subject shaping, and JSON scope parsing together with the command surface.
- Update voice approval handling to import `approval_decision_effect` and `approval_subject_json` from the permission module instead of depending on `main.rs`.
- Update connectivity grant matching to import shared `scope_values` from the permission module.
- Keep command routing in `main.rs`; the root no longer carries permission projection imports only for command implementation.
- Add direct test imports for `SessionId` and `ToolCallId` so tests do not rely on root-module incidental imports.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-cli permission_approval_queue_maps_decisions_to_scoped_grants -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_stop -- --nocapture`: passed.
- `cargo test -p capo-cli connectivity_exposure_approval_activates_only_with_matching_grant -- --nocapture`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- Focused read-only review subagent: no behavior regressions found; it identified missing positive coverage for restricted `allow_always`, which was fixed by extending the permission CLI regression test to accept durable Capo read/status scopes.

## F8/SS2g - State Projection Codec Encoder Split

Status: completed on 2026-05-26.

Decisions:

- Move projection-log row encoding into `codec_encode.rs`, including `ProjectionRecordRow` and `projection_record_to_row`.
- Keep decode, payload parsing, numeric parsing, and projection JSON validation in `codec.rs`.
- Preserve projection kind strings, `a` through `h` column mapping, JSON payload contents, append behavior, rebuild behavior, and public store APIs.
- Do not introduce typed projection descriptors yet. The encoder/decoder split is a lower-risk stepping stone that makes the current durable contract easier to audit before adding abstraction.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2f - State Query Module Split

Status: completed on 2026-05-26.

Decisions:

- Move read-only projection/event query methods into `queries.rs` as a second inherent `impl SqliteStateStore` block.
- Preserve the existing public method surface instead of introducing a query trait in this slice. The goal is concern separation and file-size reduction without changing downstream call sites.
- Keep mutation-heavy paths in `lib.rs`: event append, permission approval decision, active-run recovery mutation, rebuild orchestration, projection-log insertion, and sequence helpers.
- Use explicit imports in `queries.rs` rather than a crate-wide glob so the module's read-model dependencies remain visible to reviewers and LLMs.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2h - State Adapter Decoder Module Split

Status: completed on 2026-05-26.

Decisions:

- Move adapter projection-log decoding into `codec_adapter.rs` because adapter readiness, smoke, dispatch, prompt-source, and prompt-materialization rows form a coherent projection family and were the largest remaining part of `codec.rs`.
- Keep shared decode helpers in `codec.rs` instead of creating a helper module in this slice. Both adapter and non-adapter decoders still need the same missing-field, payload, optional-string, and numeric parsing behavior, and preserving those helpers in place avoids changing error wording.
- Preserve top-level dispatch in `projection_record_from_row`: `adapter_*` rows delegate to the adapter decoder, while unknown non-adapter rows keep the existing unknown-kind behavior.
- Keep the split private to the state crate and preserve crate-root public APIs.

Verification:

- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `git diff --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F2/DB10 - First Local Dogfood Readiness Checkpoint

Status: completed on 2026-05-26.

Decisions:

- Use local ignored `.capo-dev` state to prove the first dogfood readiness checkpoint rather than committing runtime state.
- Treat this as a control-plane rehearsal, not a real provider execution. The recorded dispatch chain uses the Codex connector shape and subscription-safe launch plan, but replays the deterministic Codex fixture with `provider_cli_executed=false`.
- Use the indexed next workpad task `workpads:features:agent-connectors.md#ac3` as the first planned dogfood target because it is already the active real-agent controller-path gap.
- Keep source markdown as the fallback. The checkpoint indexes workpads and records state/evidence but does not import the task as source-of-truth work or edit markdown.
- Next dogfood work should either import AC3 as a Capo task for a managed rehearsal or run a real opt-in Codex local dispatch after review of prompt/source and raw-output policy.

Verification:

- `capo runtime target register --target local-capo --runner local-process --status available`: recorded an available local target with `provider_cli_executed=false` and `tunnel_opened=false`.
- `capo workpad index --root /Users/nicolas/devel/capo`: recorded `files=44`, `tasks=206`.
- `capo workpad next`: selected `workpads:features:agent-connectors.md#ac3`.
- `capo workpad plan-next --agent codex-local --adapter codex --record`: recorded `adapter-dispatch-plan-codex_exec-2e26cf61ba2310e8-7463adb44145eaaf`, prompt not rendered, prompt source kind `workpad_task`, provider CLI not executed.
- `capo adapter dispatch-gate --dispatch-plan adapter-dispatch-plan-codex_exec-2e26cf61ba2310e8-7463adb44145eaaf --record`: recorded `adapter-dispatch-gate-c1e061786ffdd9e6-e76900880ff59e82`, `status=ready_for_execution`, provider CLI not executed.
- `capo adapter replay-dispatch --dispatch-plan adapter-dispatch-plan-codex_exec-2e26cf61ba2310e8-7463adb44145eaaf --fixture crates/capo-adapters/fixtures/codex-exec.jsonl --out .capo-dev/evidence`: recorded `adapter-dispatch-replay-402f3ecd6c003a86`, `appended_events=6`, `tool_events=2`, `raw_content_policy=content_hashed_not_rendered`, provider CLI not executed.
- `capo dogfood readiness`: `ready=true`, `status=ready_for_first_dogfood`, all component booleans true, no blockers, no next actions.
- `capo dogfood readiness --out .capo-dev/evidence`: exported `artifact-dogfood-readiness-38c286e1f2e30354.md` after the earlier readiness artifact had been recorded as project evidence.
- `capo dashboard`: rendered runtime target, workpad rows, connector proof, dispatch plan/gate/replay, observed-only adapter-native tool activity, and `project_dogfood_readiness=true`.
- Provider-secret-shaped marker scans over `.capo-dev` state and existing generated artifact dirs returned no matches after excluding broad false-positive terms such as bare `token` and `sk-` inside `task-`. Raw fixture response scans also returned no matches.

## F8/SS2d - State Projection Codec Module Split

Status: completed on 2026-05-26.

Decisions:

- Move projection-log row encoding and decoding into `codec.rs` as one compatibility unit. Persisted kind strings, `a` through `h` column mapping, `payload_json`, and decode error behavior must stay together.
- Keep projection apply SQL in `lib.rs`. Applying decoded records into read-model tables is a separate runtime concern from encoding records into the append-only projection log.
- Keep shared `optional_id` and `escape_json` helpers in `lib.rs` for now because they are used both by store/query code and the codec. A later helper-module cleanup can move them once query splitting clarifies the right home.
- The move preserved crate-root projection APIs and did not add dependencies or alter SQLite schema.

Verification:

- `git diff --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo fmt`: applied.
- `cargo fmt --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2c - State Schema Module Split

Status: completed on 2026-05-26.

Decisions:

- Move SQLite migration DDL, compatibility column backfills, and projection-table clearing into `schema.rs`.
- Treat schema DDL and projection reset table coverage as one physical-store concern for now. Keeping them together makes it easier to audit which tables exist and which read models are rebuildable.
- Keep projection watermark updates, projection record encoding/decoding, and `apply_projection_record` in `lib.rs` until a dedicated projection-runtime split. Moving them separately would mix a mechanical file-size change with higher-risk rebuild semantics.
- The move preserved the local SQLite source of truth and did not introduce an ORM/runtime dependency.

Verification:

- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

## F8/SS2b - State Projection Type Module Split

Status: completed on 2026-05-26.

Decisions:

- Move projection/read-model type definitions into `projections.rs` before touching SQL schema or projection codec functions.
- Preserve all crate-root projection exports with `pub use projections::*;` so `capo-query`, `capo-controller`, `capo-cli`, and tests keep the same import surface.
- Keep projection persistence, row encoding, row decoding, and SQL migration code in `lib.rs` for this slice. Those areas are higher risk because they define persisted projection kind strings and rebuild behavior.
- Keep `MemoryRecordProjection::is_packet_eligible` beside `MemoryRecordProjection` because it is a read-model eligibility invariant, not database plumbing.

Verification:

- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
