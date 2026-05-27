# Scaffold Alignment Tasks

## Objective

Recenter the implemented scaffold around the intended Capo product shape before adding more breadth.

Capo is a server/control plane with clients. The local CLI is one client for inspecting and steering tracked agents. Tracked agents are represented through ACP-compatible protocol boundaries. Project/workpad/task memory is data in Capo's DB that points to markdown source files and can be exposed to agents through tools/context. Existing `capo workpad ...` commands are transitional scaffolding for this repository, not the future product surface.

## S0 - Product Spine Audit And Rename Plan

Status: completed on 2026-05-26

Acceptance:

- Audit current user-facing CLI commands, state projections, query names, and docs for concepts that incorrectly make `workpad` a top-level Capo product surface.
- Decide which names are transitional and which should become product-facing names such as `project`, `task`, `memory`, `context`, `agent`, `session`, `dispatch`, and `evidence`.
- Produce a rename/deprecation plan that preserves current tests where useful but directs new work away from `capo workpad ...`.
- Record what should be immediate code change, what can remain as compatibility alias, and what should be deferred.

Evidence:

- `project.md`
- `TASKS.md`
- `workpads/WORKPADS.md`
- `crates/capo-cli/src/`
- `crates/capo-state/src/`
- `crates/capo-query/src/`
- `crates/capo-workpads/src/`

Findings:

- `capo workpad ...` is the only current CLI surface for markdown task/context import, proposal, and apply flows.
- State and query layers expose `workpad` as product vocabulary through events, projections, dashboard fields, readiness flags, and voice rendering output.
- `capo.workpad_read` exposes the same concept as an agent-visible tool name.
- `crates/capo-workpads` is useful implementation code, but its product role should be "markdown project-memory source adapter", not top-level Capo domain.
- Feature docs still frame dogfood readiness around a workpad bridge. That is acceptable as implementation history, but new task language should route toward project memory and agent context.

Rename and compatibility plan:

- Immediate product-facing target: add a `project memory` or `project context` command surface that can index markdown sources, import task/context records, show next task candidates, propose evidence updates, and apply approved source updates.
- Compatibility alias: keep `capo workpad ...` working while tests and dogfood scripts depend on it. Treat it as transitional and avoid adding new user-facing behavior only under `workpad`.
- Tool alias: add a product-language agent tool such as `capo.project_memory_read` or `capo.context_read`; keep `capo.workpad_read` as a compatibility alias until existing tests and docs move.
- Query/dashboard alias: introduce `project_memory_*`, `context_*`, or `source_task_*` names for new outputs; keep existing `workpad_*` fields only as compatibility/stability affordances.
- Internal deferral: do not rename event types, persisted projection fields, or the `capo-workpads` crate until the project-memory model is settled and migration risk is justified.
- Documentation direction: future tasks should describe server/controller, clients, tracked agents, project memory, context packets, dispatch, state, and evidence. Workpads remain repository process and source material, not the Capo product model.

## S0a - Project Memory CLI Alias Surface

Status: completed on 2026-05-26

Acceptance:

- Add product-language CLI aliases for the current `capo workpad ...` flows without removing the existing compatibility commands.
- Prefer a narrow command shape such as `capo project memory ...` unless implementation context shows a clearer existing pattern.
- Add deterministic tests showing the product-language aliases route to the same behavior as the existing workpad commands.
- Update command help or docs touched by the change so new implementation points users toward project memory/context language.
- Do not broaden into dashboard, voice, source-writing, graph/vector memory, or remote clients in this task.

Evidence:

- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/project_memory.rs`
- `crates/capo-cli/src/workpad.rs`
- Existing CLI tests for `capo workpad ...`
- New alias tests

Result:

- Added `capo project memory index|next|plan-next|start-next|import|propose|apply` as the product-language CLI surface for markdown-backed project memory.
- Kept existing `capo workpad ...` compatibility commands intact.
- Allowed product-language `--source-task` for import/propose while still supporting the underlying compatibility `--workpad-task` path.
- Product-memory commands emit product-language status keys such as `project_memory_indexed`, `project_memory_next_found`, `project_memory_task_imported`, and `source_task_id`, while preserving compatibility output for existing consumers.
- Help now points users toward `capo project memory ...` in addition to the transitional workpad commands.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli`

## S1 - Narrow E2E Spine Definition

Status: completed on 2026-05-26

Acceptance:

- Define the smallest e2e flow Capo must prove next: client instruction -> controller dispatch -> tracked agent/protocol events -> DB state -> project memory/context exposure -> evidence export.
- Use the existing scripted mock agent and bounded Codex proof as evidence inputs, but state any gaps between current implementation and the desired product spine.
- Identify which broad features are not scaffold-critical: voice, mobile, remote CLI/app, rich dashboard, remote runtime adapters, graph/vector memory, and source-writing dogfood.
- Add or update tests only if needed to make the e2e gate unambiguous.

Evidence:

- `workpads/prototype/spec.md`
- `workpads/features/agent-connectors.md`
- `workpads/features/dogfood-bridge.md`
- `workpads/dogfood/knowledge.md`
- Scripted mock agent tests
- Bounded Codex dispatch proof evidence

Result:

- Added `workpads/scaffold/e2e-spine.md` as the narrow scaffold target.
- Defined the required product loop: client instruction -> controller dispatch -> tracked adapter/ACP-shaped events -> SQLite state -> markdown-backed project memory/context exposure -> read-model inspection -> recovery -> evidence export.
- Classified fake-controller e2e, scripted mock agent, fixture replay, bounded Codex proof, and project-memory aliases as reusable evidence.
- Recorded current gaps: no single deterministic e2e test proves the full product spine, ACP is not yet first-class tracked-session execution, project memory internals are still workpad-named, and the CLI is not yet the intended conversational local client loop.
- Explicitly deferred voice, rich dashboard, remote clients, remote runtime, source writeback, graph/vector memory, and broad provider automation.
- No test changes were needed for S1 because it is a definition/planning task.

Verification:

- `git diff --check`

## S1a - Scripted Project-Memory Dispatch E2E

Status: completed on 2026-05-26

Acceptance:

- Add a deterministic e2e test that starts from `capo project memory ...`, imports a markdown-backed source task, dispatches it through a scripted mock or ACP-shaped adapter turn, and verifies controller-owned read models.
- The scripted turn must include at least one assistant update, one project-memory/context tool request or observation, and one completion event.
- The test must verify agent/session/run state, task binding, tool activity, context or memory refs, summary/confidence, recovery without duplicate read models, and evidence export.
- Source markdown must remain unchanged.
- Primary assertions should use product-language keys/surfaces where new behavior is introduced; compatibility `workpad_*` assertions are allowed only where internals still require them.

Evidence:

- `workpads/scaffold/e2e-spine.md`
- `crates/capo-cli/src/project_memory.rs`
- `crates/capo-controller/src/adapter_replay.rs`
- `crates/capo-adapters/src/scripted_mock_agent.rs`
- Existing `prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`
- Existing `scripted_mock_agent_drives_multi_turn_controller_state`
- New `project_memory_scripted_dispatch_proves_narrow_spine`

Result:

- Added a deterministic e2e test that starts from `capo project memory index|next|start-next`.
- The test dispatches a markdown-backed source task through controller state, applies a scripted mock adapter turn, and verifies a project-memory tool request/completion.
- It checks task/session/run binding, adapter-native tool call and observation read models, memory packet refs, session status output, recovery without duplicate tool/memory rows, evidence export, and unchanged source markdown.
- The test intentionally uses a scripted mock instead of a provider subscription, making the product spine repeatable in normal CI/local test runs.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-cli`

## S2 - Project Memory Model Alignment

Status: completed on 2026-05-26

Acceptance:

- Reframe current workpad indexing/import state as project memory records backed by markdown sources.
- Define the DB hierarchy and naming for project, workpad-like grouping, task, source file, source anchor, context request, memory packet, and evidence refs.
- Decide whether existing `workpad_*` projection names remain internal compatibility details or should be migrated.
- Ensure the plan exposes project memory to agents through tools/context rather than through a top-level `capo workpad` product command.

Evidence:

- `workpads/architecture/state-model.md`
- `workpads/architecture/memory-architecture.md`
- `workpads/architecture/tool-exposure.md`
- `crates/capo-state/src/projections.rs`
- `crates/capo-workpads/src/lib.rs`
- `crates/capo-tools/src/`

Result:

- Added `workpads/scaffold/project-memory-model.md`.
- Defined the V0 hierarchy: Project -> SourceDocument -> SourceSection -> SourceTask, plus Task/SourceBinding, Agent/Session/Run/Turn, ContextRequest, ToolActivity, MemoryRecord/MemorySource, MemoryPacket, and EvidenceRef.
- Classified current `workpad_*` state, query, CLI, and tool names as compatibility where appropriate.
- Chose product-facing names: `source_task_id`, `source_document`, `source_section`, `source_binding`, `project_memory`, `context_request`, `memory_packet`, and `evidence_ref`.
- Deferred persisted event/table renames until product-language command/tool/query aliases are in place.

Verification:

- `git diff --check`

## S2a - Governed Project Memory Read Tool Alias

Status: completed on 2026-05-26

Acceptance:

- Add `capo.project_memory_read` as a product-language Capo tool and runtime wrapper alias for the existing constrained markdown source reader.
- Keep `capo.workpad_read` available as compatibility.
- Ensure permissions include `tool:invoke:capo.project_memory_read` and the same workspace-read/task-read scopes.
- Add deterministic tests proving both aliases resolve through the same path and path constraints remain enforced.
- Update tool docs/help touched by the change to prefer project-memory language.

Evidence:

- `workpads/scaffold/project-memory-model.md`
- `crates/capo-tools/src/lib.rs`
- `crates/capo-tools/src/runtime_wrappers.rs`
- `crates/capo-tools/src/permission.rs`
- `crates/capo-tools/src/tests.rs`

Result:

- Added `capo.project_memory_read` to the Capo-owned tool registry and runtime wrapper tool list.
- Kept `capo.workpad_read` as a compatibility tool.
- Added read-only/reviewer policy scopes for `tool:invoke:capo.project_memory_read`.
- Runtime wrapper alias uses the same constrained markdown-source path policy as `capo.workpad_read`, while writing `project_memory_read` output artifacts.
- Updated `workpads/architecture/tool-exposure.md` to prefer `capo.project_memory_read`.

Verification:

- `cargo fmt`
- `cargo test -p capo-tools`
- `cargo test`
- `git diff --check`

## S3 - Client/Server Boundary Alignment

Status: completed on 2026-05-26

Acceptance:

- Clarify which parts of `capo-cli` are local client behavior and which parts should belong to the server/controller API.
- Identify any command implementation that couples CLI UX directly to domain concepts that should live behind controller/query APIs.
- Plan the minimum code movement needed so future remote CLI, app, or voice clients can reuse the same controller/query surface.
- Keep voice as a future client surface unless a small existing voice module must be retained as a compatibility test.

Evidence:

- `workpads/architecture/boundaries.md`
- `workpads/architecture/protocol-provider.md`
- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/dashboard.rs`
- `crates/capo-cli/src/workpad.rs`
- `crates/capo-query/src/lib.rs`
- `crates/capo-controller/src/lib.rs`

Result:

- Added `workpads/scaffold/client-server-boundary.md`.
- Classified the local CLI as a client that parses args, submits controller/query requests, renders read models, and keeps compatibility aliases.
- Classified controller/query/state/tools as the product surface for task/session/agent/run transitions, source binding, project memory/context, adapter dispatch, permissions, tool activity, and evidence.
- Identified current CLI coupling in `workpad.rs`, `project_memory.rs`, `dashboard.rs`, `dogfood.rs`, `voice.rs`, `voice_render.rs`, `adapter_dispatch_prepare.rs`, `adapter_dispatch_run.rs`, and `evidence.rs`.
- Decided the next code movement should be product-language query aliases for source tasks/project memory before storage renames or a server transport.

Verification:

- `git diff --check`

## S3a - Project Memory Query Alias Surface

Status: completed on 2026-05-26

Acceptance:

- Add product-language query types/accessors for source tasks or project-memory tasks backed by existing `WorkpadTaskProjection`.
- Keep existing `workpad_tasks`, `next_workpad_task`, and workpad filters as compatibility.
- Update `capo project memory next` or the deterministic spine test to use/assert the new product-language query alias where practical.
- Do not rename SQLite tables or remove `capo workpad ...`.
- Add focused tests in `capo-query` and/or `capo-cli`.

Evidence:

- `crates/capo-query/src/types.rs`
- `crates/capo-query/src/summary.rs`
- `crates/capo-query/src/lib.rs`
- `crates/capo-query/src/tests.rs`
- `crates/capo-cli/src/project_memory.rs`
- `crates/capo-cli/src/tests.rs`

Result:

- Added `SourceTaskProjection` as the product-language query alias backed by the existing `WorkpadTaskProjection`.
- Added `ProjectDashboard::source_tasks`, `next_source_task`, and `next_source_task_candidate_count` while keeping `workpad_tasks`, `next_workpad_task`, and workpad filters intact as compatibility.
- Updated `capo project memory next` to select through the source-task query alias and emit product-language fields such as `source_task_id`, `source_path`, `observed_source_status`, and `capo_binding_status`.
- Preserved compatibility output by appending the existing `workpad next` rendering and keeping `compatibility_workpad_task_id`.
- Did not rename SQLite tables, event kinds, projection storage, or remove `capo workpad ...`.

Verification:

- `cargo fmt`
- `cargo test -p capo-query project_dashboard_includes_workpad_tasks -- --nocapture`
- `cargo test -p capo-query project_dashboard_selects_next_actionable_workpad_task -- --nocapture`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S4 - Scaffold Gate Review

Status: completed on 2026-05-26

Acceptance:

- Review whether the scaffold now supports the intended product spine with clear naming and modularity.
- List what is real, what is stubbed, what is transitional compatibility, and what is deferred.
- Confirm the next implementation tasks are ordered one at a time and commit-ready.
- Do not claim completion if `capo workpad ...` remains the only way to express project memory/task context in the user-facing model.

Evidence:

- Updated `project.md`
- Updated `TASKS.md`
- Updated `workpads/WORKPADS.md`
- This workpad's `knowledge.md`
- Relevant diffs and test results from S0-S3

Result:

- Added `workpads/scaffold/gate-review.md`.
- Gate passes for continued scaffold implementation with constraints: keep compatibility names stable, prefer product-language project-memory/source-task/controller/query surfaces in new work, and avoid broad client/voice/dashboard expansion before the core spine is cleaner.
- Classified real implementation, stubbed/limited areas, transitional compatibility, and deferred breadth.
- Set the ordered next-task queue: source-binding projection, reusable controller/query project-memory helpers, deterministic ACP-shaped mock connector, presentation vocabulary cleanup, then bounded real local connector proof refresh.

Verification:

- `cargo fmt`
- `cargo test`
- `git diff --check`

## S5 - Explicit Source Binding Projection

Status: completed on 2026-05-26

Acceptance:

- Add a product-language source-binding read model that links executable Capo `Task` records to markdown-backed source tasks/documents without parsing source facts from task summaries.
- Keep existing `WorkpadTaskProjection`, workpad tables, workpad events, and `capo workpad ...` compatibility unchanged.
- Record source-binding projections when project-memory/workpad task import or start-next binds a source task to a Capo task.
- Add query accessors so controller/client surfaces can read source bindings by task and project.
- Update the deterministic project-memory spine test or focused state tests to prove the binding is persisted, rebuilt, and queryable.
- Do not broaden into storage renames, source writeback, graph/vector memory, voice, or remote clients.

Evidence:

- `workpads/scaffold/gate-review.md`
- `workpads/scaffold/project-memory-model.md`
- `workpads/architecture/state-model.md`
- `crates/capo-state/src/`
- `crates/capo-cli/src/workpad.rs`
- `crates/capo-cli/src/project_memory.rs`
- `crates/capo-cli/src/tests.rs`

Result:

- Added `SourceBindingProjection` as a product-language read model linking Capo `Task` IDs to markdown-backed source task/document refs.
- Added `source_bindings` SQLite projection storage, projection encode/decode/rebuild support, and state queries by project and task.
- Added `ProjectDashboard::source_bindings` so client/query surfaces can read source bindings without touching workpad compatibility tables directly.
- Updated project-memory/workpad import to record a source binding whenever a markdown-backed source task is bound to an executable Capo task.
- Preserved `WorkpadTaskProjection`, workpad tables/events, `capo workpad ...`, and compatibility output.
- Updated deterministic project-memory tests to assert source binding persistence during import/start-next and added focused state/query rebuild coverage.

Verification:

- `cargo fmt`
- `cargo test -p capo-state source_binding_projection_is_persisted_and_rebuilt -- --nocapture`
- `cargo test -p capo-query project_dashboard_includes_source_bindings -- --nocapture`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-state`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S6 - Project Memory Controller/Query Helper Surface

Status: completed on 2026-05-26

Acceptance:

- Move project-memory workflow composition behind reusable product-language helper APIs instead of keeping the behavior primarily inside CLI text wrappers.
- `capo project memory plan-next|start-next|import` should call product-language helpers where practical; `capo workpad ...` may remain as compatibility wrappers.
- Helpers should consume `SourceTaskProjection` and `SourceBindingProjection` where possible rather than requiring callers to parse `WorkpadTaskProjection` or `TaskProjection.latest_summary`.
- Keep storage/event/table compatibility unchanged.
- Add deterministic tests proving both product-language CLI paths and compatibility workpad paths still behave correctly.
- Do not broaden into ACP mock connector, voice, remote clients, or source writeback in this task.

Evidence:

- `crates/capo-cli/src/project_memory_flow.rs`
- `crates/capo-cli/src/project_memory.rs`
- `crates/capo-cli/src/workpad.rs`
- `crates/capo-cli/src/tests.rs`

Result:

- Added `project_memory_flow` as a product-language helper surface for markdown source-task import and source binding.
- Moved source-task import composition out of the workpad CLI handler and into `import_markdown_source_task`.
- Updated `capo project memory import` to call the product helper directly and render product-language fields first, while preserving compatibility `workpad_task_imported` output.
- Updated `capo workpad import` to use the same helper as a compatibility wrapper.
- Updated `capo project memory next|plan-next|start-next` to run through source-task selection helpers before compatibility rendering/dispatch paths.
- Kept storage/event/table compatibility and avoided ACP mock connector, voice, remote clients, and source writeback.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S7 - Deterministic ACP-Shaped Mock Connector

Status: completed on 2026-05-26

Acceptance:

- Add deterministic ACP-shaped mock connector coverage so the narrow product spine looks closer to tracked ACP session traffic while staying provider-free.
- Reuse existing scripted mock concepts where practical, but expose ACP-shaped session/tool/message events at the adapter boundary.
- Update the project-memory spine test or add a focused e2e test proving source-task dispatch, context/tool activity, state/recovery, and evidence through the ACP-shaped mock path.
- Keep real Codex/Claude connector execution opt-in only.
- Do not broaden into voice, remote clients, source writeback, or full ACP live-server implementation.

Evidence:

- `crates/capo-adapters/src/scripted_mock_agent.rs`
- `crates/capo-controller/src/adapter_replay.rs`
- `crates/capo-cli/src/tests.rs`

Result:

- Added `ScriptedMockAgent::acp_shaped` to reuse deterministic scripted turns while emitting `NormalizedAdapterKind::Acp` events, `acp:*` timeline keys, and `acp.mock.*` provider event kinds.
- Kept the existing scripted mock adapter behavior intact for compatibility.
- Added controller replay helper `apply_scripted_acp_mock_turn`.
- Updated the deterministic project-memory spine test to replay source-task dispatch, project-memory tool activity, recovery, and evidence through the ACP-shaped mock path.
- Real Codex/Claude execution remains opt-in only.
- Did not broaden into voice, remote clients, source writeback, or a full ACP live-server implementation.

Verification:

- `cargo fmt`
- `cargo test -p capo-adapters scripted -- --nocapture`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-adapters`
- `cargo test -p capo-controller`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S8 - Presentation Vocabulary Cleanup

Status: completed on 2026-05-26

Acceptance:

- Move shared client-facing readouts toward project-memory/source-task naming first, keeping workpad keys only as compatibility where needed.
- Prioritize dashboard/dogfood/voice surfaces that still expose `workpad_*` as primary product language.
- Use `SourceTaskProjection` and `SourceBindingProjection` where practical.
- Keep `capo workpad ...` compatibility commands and storage unchanged.
- Add focused tests proving new source-task/project-memory fields are present while existing compatibility assertions still pass.
- Do not broaden into remote clients, voice feature expansion, source writeback, or storage renames.

Evidence:

- `crates/capo-query/src/types.rs`
- `crates/capo-query/src/dogfood.rs`
- `crates/capo-cli/src/dashboard.rs`
- `crates/capo-cli/src/dogfood.rs`
- `crates/capo-cli/src/voice_render.rs`
- `crates/capo-query/src/tests.rs`
- `crates/capo-cli/src/tests.rs`

Result:

- Added project-memory/source-task readiness fields to the shared dogfood query result while retaining workpad bridge fields as compatibility.
- Updated dashboard output to render `project_memory_source`, `source_tasks`, `source_task`, and `source_bindings` before the compatibility `workpad_tasks` rows.
- Updated dogfood readiness text and markdown evidence to prefer project-memory/source-task language while keeping compatibility workpad fields.
- Updated voice readouts for next-work and dogfood readiness to emit `spoken_source_*` and `spoken_project_memory_*` fields while preserving existing `spoken_workpad_*` assertions.
- Kept `capo workpad ...`, persisted storage, event kinds, and source writeback behavior unchanged.

Verification:

- `cargo fmt`
- `cargo test -p capo-query project_dogfood_readiness_reports_blockers_and_ready_counts -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli voice_next_work_reads_workpad_queue_without_mutating -- --nocapture`
- `cargo test -p capo-cli voice_dogfood_readiness_reads_shared_query_without_mutating -- --nocapture`
- `cargo test -p capo-cli voice_confirmed_start_next_work_imports_and_dispatches_after_approval -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate_blocks_until_real_smoke_evidence_is_recorded -- --nocapture`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S9 - Bounded Real Local Connector Proof Refresh

Status: completed on 2026-05-26

Acceptance:

- Refresh the approved real local connector proof against the current project-memory/source-task scaffold.
- Keep provider execution opt-in, bounded, subscription-safe, and redacted; never render credentials, raw subscription material, raw prompts, or raw provider output.
- Prefer the existing Codex/Claude/local adapter smoke and dispatch-gate machinery rather than adding a new provider path.
- Record clean evidence that at least one authorized local connector can be prepared or smoke-tested through the current boundaries, and clearly separate what was executed from what remained gated.
- Do not broaden into full live ACP server implementation, remote runtime adapters, voice expansion, source writeback, or storage renames.

Evidence:

- `workpads/features/agent-connectors.md`
- `workpads/architecture/protocol-provider.md`
- `crates/capo-adapters/src/local_subscription.rs`
- `crates/capo-adapters/src/tests.rs`
- `crates/capo-cli/src/adapter_smoke.rs`
- Ignored local state/evidence under `.capo-dev/scaffold-s9`

Result:

- Confirmed local connector programs are present: `codex-cli 0.133.0` and `Claude Code 2.1.150`.
- Ran the Codex local smoke once without `CAPO_RUN_CODEX_LOCAL_SMOKE=1`; it passed as an opt-in-gated no-op, proving the smoke remains gated.
- Ran the authorized opt-in Codex smoke with `CAPO_RUN_CODEX_LOCAL_SMOKE=1`; it passed through the existing restrictive smoke harness.
- Scanned the resulting Codex smoke artifacts with `capo adapter smoke-report scan`; the scan reported `credential_scan_status=clean` and `files_scanned=2`.
- Recorded passed smoke report `adapter-smoke-codex_exec-d38d3f3fee60856c` in ignored local state with `smoke_status=passed`, `credential_scan_status=clean`, `marker_found=true`, and `dogfood_readiness_effect=real_agent_connector_proven`.
- Exported redacted smoke evidence `artifact-adapter-smoke-evidence-4ed49a3bdbe85cd6.md`; the evidence renders metadata only and does not render stdout, stderr, prompts, provider output, tokens, cookies, or subscription session material.
- Indexed current markdown-backed project memory into the same ignored state to confirm the refreshed connector evidence coexists with `project_memory_source=markdown`, `source_tasks=214`, and `project_memory_ready=true`.
- Confirmed current dogfood readiness is honest: `real_agent_connector_ready=true` and `project_memory_ready=true`, while `runtime_target_ready=false` and `dispatch_chain_ready=false` remain gated.

Executed:

- `codex --version`
- `claude --version`
- `cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`
- `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`
- `cargo run -q -p capo-cli -- adapter smoke-report scan --artifact-root <local-temp-codex-smoke-artifacts> --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- adapter smoke-report record --adapter codex --status passed --credential-scan clean --marker-found --artifact-root <local-temp-codex-smoke-artifacts> --reason "S9 bounded Codex smoke refreshed against project-memory scaffold" --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- adapter smoke-report status --smoke-report adapter-smoke-codex_exec-d38d3f3fee60856c --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- adapter dogfood-gate --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- adapter smoke-report evidence --smoke-report adapter-smoke-codex_exec-d38d3f3fee60856c --out .capo-dev/scaffold-s9/evidence --state .capo-dev/scaffold-s9`
- `rg -a` credential/session-marker scan over `.capo-dev/scaffold-s9` and the local smoke artifacts
- `cargo run -q -p capo-cli -- project memory index --root . --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- dashboard --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- dogfood readiness --state .capo-dev/scaffold-s9`

Still gated:

- Claude Code real smoke was not executed in this task.
- Broad real-provider source-task dispatch was not executed in this task.
- Full live ACP server/session implementation remains future work.
- Runtime target registration and dispatch-chain proof remain required before dogfood readiness can be fully ready.

Verification:

- `cargo fmt`
- `cargo test -p capo-adapters`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S10 - Public CLI Surface Reduction

Status: completed on 2026-05-26

Acceptance:

- Reduce the public prominence of transitional `capo workpad ...` commands so the CLI presents project-memory/source-task/client-control language first.
- Keep compatibility routing for existing tests and local scripts, but mark workpad commands as internal/transitional in help/docs or move them behind a clearly compatibility-oriented surface.
- Ensure new examples and help text prefer `capo project memory ...`, agent/session/dispatch/evidence commands, and project-memory readouts.
- Add focused tests proving help/output no longer positions workpad as a primary product command while compatibility commands still execute.
- Do not rename persisted storage/events or remove compatibility routing in this task.

Evidence:

- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-cli/src/tests.rs`
- Rendered `capo --help`
- Existing project-memory and workpad compatibility CLI tests

Result:

- Moved `capo workpad ...` commands out of the main usage list and into a clearly labeled `Compatibility commands` section.
- Added a `Primary model` section that states Capo is a local-first controller/server, the CLI is one client, markdown-backed planning files enter as project memory, and new workflows should prefer project-memory/source-task/agent/session/dispatch/evidence language.
- Kept all `capo workpad ...` compatibility routes intact for existing scripts and regression coverage.
- Updated help tests to assert project-memory commands appear before the compatibility section and `capo workpad ...` appears only after the compatibility label.
- Verified both product-facing `capo project memory ...` and compatibility `capo workpad ...` paths still execute.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo run -q -p capo-cli -- --help`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S11 - Product Alignment Completion Audit

Status: completed on 2026-05-26

Acceptance:

- Audit the current scaffold against the original objective: project direction, `project.md`, CLI/code alignment, ACP-tracked-agent direction, DB-backed markdown project memory, task ordering, and remaining workpad compatibility.
- Identify every remaining mismatch that prevents calling the alignment goal complete.
- If completion is not proven, add concrete next tasks ordered one at a time.
- If completion is proven, record the evidence and final residual risks before marking the scaffold workpad complete.
- Do not treat passing tests alone as sufficient; inspect current source/docs/CLI behavior.

Evidence:

- `workpads/scaffold/completion-audit.md`
- `project.md`
- `TASKS.md`
- `AGENTS.md`
- Rendered `capo --help`
- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-cli/src/dashboard.rs`
- `crates/capo-cli/src/evidence.rs`
- `crates/capo-cli/src/tool_wrapper.rs`
- `crates/capo-query/src/dogfood.rs`
- `crates/capo-cli/src/dogfood.rs`
- `crates/capo-cli/src/voice_render.rs`

Result:

- Added `workpads/scaffold/completion-audit.md`.
- Confirmed the high-level project direction is aligned: Capo is documented as a local-first server/control plane, the CLI is one client, ACP is the preferred tracked-agent boundary where it fits, and markdown-backed project memory is the v0 memory path.
- Confirmed DB-backed markdown project-memory scaffolding is real enough for v0: `SourceTaskProjection`, `SourceBindingProjection`, `capo.project_memory_read`, `capo project memory ...`, deterministic ACP-shaped mock coverage, and bounded Codex connector proof exist.
- Confirmed public help no longer presents `capo workpad ...` as primary usage; workpad commands are now in a compatibility section.
- Did not mark the overall alignment goal complete. The audit found remaining product-facing mismatches in dashboard filters, review follow-up links, dogfood readiness blocker/action wording, and tool wrapper shorthand/error text.

Verification:

- `git diff --check`

## S12 - Product-Language CLI Option Aliases

Status: completed on 2026-05-26

Acceptance:

- Add product-language aliases for dashboard source filters, such as `--source-path` and `--source-status`, while retaining `--workpad-path` and `--workpad-status` compatibility.
- Add product-language review follow-up alias such as `--follow-up-source-task`, while retaining `--follow-up-workpad-task` compatibility.
- Render review evidence with source-task wording first and workpad wording only as compatibility.
- Update help text to prefer the product-language aliases.
- Add focused tests proving product aliases work and compatibility aliases still work.
- Do not rename persisted review/workpad fields or remove compatibility arguments.

Result:

- `capo dashboard` help now presents `--source-path` and `--source-status` as the primary filters.
- `capo dashboard` accepts `--source-path`/`--source-status` and retains `--workpad-path`/`--workpad-status` as compatibility aliases, with ambiguous mixed alias use rejected.
- `capo review record` help now presents `--follow-up-source-task` as the primary follow-up link.
- `capo review record` accepts `--follow-up-source-task` and retains `--follow-up-workpad-task` as a compatibility alias, with ambiguous mixed alias use rejected.
- Review markdown now renders `Follow-up source task` first and includes `Compatibility workpad task` for transitional readers.
- Persisted review/workpad fields were not renamed.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo test -p capo-cli dashboard_rejects_malformed_filters -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli review_record_accepts_follow_up_source_task_alias -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S13 - Product-Language Readiness Reasons

Status: completed on 2026-05-26

Acceptance:

- Add product-language dogfood readiness blocker and next-action aliases for project-memory/source-task state, such as `project_memory_index_missing` and `run_project_memory_index`.
- Prefer product-language readiness reasons in CLI and voice readouts while preserving workpad compatibility fields where existing consumers rely on them.
- Update tests so primary assertions use product-language readiness fields/reasons.
- Do not remove workpad compatibility fields in this task.

Result:

- Dogfood readiness blockers now use product-language reasons: `project_memory_index_missing` and `source_task_dispatch_chain_missing`.
- Dogfood readiness next actions now use product-language actions: `run_project_memory_index` and `record_or_replay_source_task_dispatch_plan`.
- Added explicit compatibility reason fields for old workpad-oriented consumers: `compatibility_blockers` and `compatibility_next_actions`.
- CLI, dashboard, evidence markdown, and voice readouts render primary product-language reasons while exposing compatibility reason fields separately.
- Existing workpad readiness booleans/counts/refs were preserved.

Verification:

- `cargo fmt`
- `cargo test -p capo-query project_dogfood_readiness_reports_blockers_and_ready_counts -- --nocapture`
- `cargo test -p capo-cli adapter_dogfood_gate_requires_passed_codex_smoke_report -- --nocapture`
- `cargo test -p capo-cli voice_dogfood_readiness_reads_shared_query_without_mutating -- --nocapture`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`

## S14 - Project Memory Tool Wrapper Shorthand

Status: completed on 2026-05-26

Acceptance:

- Add `project_memory_read` / `project-memory-read` shorthand for `capo.project_memory_read` in `capo tool run-wrapper`.
- Update wrapper input handling so `capo.project_memory_read` and shorthand use the project-memory wrapper path.
- Update unknown-tool error text to list `project_memory_read` before compatibility `workpad_read`.
- Add focused tests proving the project-memory shorthand works and workpad shorthand remains compatible.

Result:

- Added `project_memory_read` and `project-memory-read` as `capo tool run-wrapper` shorthands for `capo.project_memory_read`.
- Updated wrapper input handling so `capo.project_memory_read` and its shorthands accept the same `--path` input as the compatibility source reader.
- Updated unknown wrapper tool guidance to list `project_memory_read` before compatibility `workpad_read`.
- Added regression coverage for `project_memory_read`, fully qualified `capo.project_memory_read`, compatibility `workpad_read`, and unknown-tool guidance.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli tool_run_wrapper_exposes_governed_runtime_wrappers_without_providers -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`
