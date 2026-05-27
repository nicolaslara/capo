# Scaffold Alignment Knowledge

## Objective

Record decisions from the scaffold alignment pass.

## Initial Direction

Status: started on 2026-05-26.

Decisions:

- Capo should be framed as a server/control plane. The local CLI is one client; future clients can include remote CLI, dashboard/app, and voice.
- Tracked agents should be represented through the agent/protocol boundary, with ACP as the preferred shape where it fits.
- Project/workpad/task concepts should live as DB-backed project memory records pointing to markdown sources. Markdown remains the human-auditable fallback and source material.
- `capo workpad ...` is confusing as a top-level product surface. It may remain temporarily as transitional scaffolding for the current repository, but new product-facing surfaces should use project/task/memory/context terminology.
- Voice, mobile, remote runtime, rich dashboard, graph/vector memory, and source-writing dogfood are future breadth unless they directly prove the core scaffold spine.

Open questions:

- Should current `workpad_*` projection names be migrated immediately or retained as internal compatibility while new aliases expose project memory concepts?
- Should CLI compatibility aliases remain indefinitely, or should `capo workpad ...` become hidden/internal once project-memory commands exist?
- How much ACP should be implemented before real tracked-agent support, versus continuing with normalized adapter events plus ACP-shaped tests?

## S0 - Product Spine Audit And Rename Plan

Status: completed on 2026-05-26.

Findings:

- The implementation currently exposes workpads as product vocabulary in several places: `capo workpad ...`, `workpad_*` query and dashboard fields, voice-rendered workpad summaries, `capo.workpad_read`, and workpad-named state events/projections.
- This is broader than a small CLI rename. The product-facing rename needs compatibility because tests, dogfood history, and architecture notes already depend on these names.
- The `capo-workpads` crate can remain as implementation detail for scanning markdown source files. Its domain role should be a markdown-backed project-memory adapter.

Decisions:

- New user-facing behavior should use project memory, task/context, agent, session, dispatch, state, and evidence language.
- `capo workpad ...` remains as a compatibility surface while aliases and tests are added. It should not be the only user-facing path for project memory.
- Product-language aliases should be implemented before internal storage/event renames. That keeps the scaffold usable and avoids a risky migration before the desired model is settled.
- `capo.workpad_read` should gain a product-language alias such as `capo.project_memory_read` or `capo.context_read`; the exact tool name should be chosen when updating the tool registry.
- Dashboard, voice, and query names should be updated after the CLI alias path is proven, because they are presentation/read-model concerns rather than the core product spine.

Deferred:

- Renaming persisted event kinds and projection fields.
- Renaming `crates/capo-workpads`.
- Removing `capo workpad ...`.
- Adding graph/vector memory, source-writing dogfood, voice, rich dashboard, or remote clients.

## S0a - Project Memory CLI Alias Surface

Status: completed on 2026-05-26.

Implementation:

- Added `crates/capo-cli/src/project_memory.rs` as a thin product-facing CLI layer over the existing markdown/workpad adapter.
- Added `capo project memory index|next|plan-next|start-next|import|propose|apply`.
- Kept `capo workpad ...` unchanged as compatibility scaffolding.
- Added product-language output keys so new callers do not have to key only on `workpad_*` names.
- Added `--source-task` for project-memory import/propose; it normalizes to the existing internal `--workpad-task` argument.

Decision:

- This is intentionally an alias layer, not an internal schema/event migration. It moves the user-facing surface in the right direction while preserving dogfood history and existing tests.
- The compatibility output still includes workpad keys because the underlying adapter and read models are still workpad-named. S2 should decide whether to migrate those internals after the project-memory model is tighter.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli`

## S1 - Narrow E2E Spine Definition

Status: completed on 2026-05-26.

Artifact:

- `workpads/scaffold/e2e-spine.md`

Definition:

- The next scaffold proof should be one deterministic path: client instruction, controller dispatch, tracked adapter/ACP-shaped events, SQLite event/read-model state, markdown-backed project memory/context exposure, recovery, and evidence export.
- The path should start from the new product-language project-memory surface rather than the transitional `capo workpad ...` command.
- The deterministic scripted mock agent is the right test driver for regression coverage. The bounded Codex proof remains important provider evidence but should not be the main repeatable test.

Evidence reused:

- Fake-controller e2e proves basic orchestration, tools, memory refs, recovery, and evidence export.
- Scripted mock agent proves deterministic normalized adapter events through static dispatch.
- Codex/Claude/ACP fixture replay proves provider/protocol events can feed controller read models without raw provider text retention.
- Bounded Codex proof proves at least one opt-in real local provider stream can be ingested cleanly.
- Project-memory CLI aliases prove new work no longer has to use `capo workpad ...` as the only user-facing path.

Gaps:

- No single deterministic e2e test yet proves the full product spine.
- ACP is not yet the first-class tracked-session execution path; it is fixture/capability-shaped evidence.
- Project-memory internals still use workpad-named events, projections, dashboard fields, and tool IDs.
- The CLI remains command-oriented; the future local client agent loop is still design/implementation work.

Decision:

- Add `S1a - Scripted Project-Memory Dispatch E2E` before more breadth. It should prove the spine with deterministic scripted events, then later tasks can migrate naming and client/server shape.

## S1a - Scripted Project-Memory Dispatch E2E

Status: completed on 2026-05-26.

Implementation:

- Added `project_memory_scripted_dispatch_proves_narrow_spine` in `crates/capo-cli/src/tests.rs`.
- The test indexes markdown project memory through `capo project memory index`, selects the next source task, registers a fake agent, starts the task through `capo project memory start-next`, and then applies a deterministic scripted mock adapter turn.
- The scripted turn includes an assistant update, `capo.project_memory_read` request, `capo.project_memory_read` completion, assistant completion, and turn completion.
- The test verifies task/session/run binding, memory packet refs, adapter-native tool call and observation read models, session status rendering, recovery without duplicate tool/memory rows, evidence export, and unchanged source markdown.

Finding:

- `capo.project_memory_read` is currently observed as adapter-native `observed_only` activity in this path. That is honest for the current implementation but means S2/S3 should still add a fully governed product-language Capo tool alias over `capo.workpad_read`.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-cli`

## S2 - Project Memory Model Alignment

Status: completed on 2026-05-26.

Artifact:

- `workpads/scaffold/project-memory-model.md`

Decisions:

- Treat workpads as one markdown source convention inside project memory, not a top-level Capo product model.
- Use `SourceDocument`, `SourceSection`, and `SourceTask` as the target model for observed markdown files/headings/task-like sections.
- Use `SourceBinding` as the target link between an executable Capo `Task` and source material. This should replace parsing source facts from `TaskProjection.latest_summary`.
- Use `ContextRequest` and `MemoryPacket` for agent-facing context delivery; generated memory remains derived and provenance-backed.
- Keep current `workpad_*` event kinds, tables, projections, and crate names as compatibility until aliases and source-binding projections are in place.

Migration order:

- Add `capo.project_memory_read` as a governed tool alias over the constrained markdown reader.
- Add product-language query aliases for source tasks/project memory.
- Add explicit source-binding projection.
- Build memory packets from source bindings and source refs rather than hard-coded prototype workpad refs.
- Rename storage only after clients and tests use product language consistently.

Risk:

- The current `capo.project_memory_read` usage in S1a is adapter-native observed-only activity. S2a should make it a fully governed Capo tool/wrapper alias.

## S2a - Governed Project Memory Read Tool Alias

Status: completed on 2026-05-26.

Implementation:

- Added `capo.project_memory_read` to `CAPO_OWNED_TOOLS` and `CAPO_WRAPPER_TOOLS`.
- Added registry and runtime wrapper definitions with `tool:invoke:capo.project_memory_read`, `filesystem:read:workspace`, and `state:read:task` scopes.
- Added read-only and reviewer static policy grants for the product-language tool.
- Kept `capo.workpad_read` available for compatibility.
- The runtime wrapper enforces the same constrained markdown source paths as the compatibility workpad reader and records `project_memory_read` artifacts.
- Updated `workpads/architecture/tool-exposure.md` to document `capo.project_memory_read` as the preferred product-language reader.

Decision:

- The tool alias is a real governed Capo tool/wrapper, not just an adapter-native observed name.
- `capo.workpad_read` remains compatibility until query/dashboard/voice and tests have moved to project-memory/source-task language.

Verification:

- `cargo fmt`
- `cargo test -p capo-tools`
- `cargo test`
- `git diff --check`

## S3 - Client/Server Boundary Alignment

Status: completed on 2026-05-26.

Artifact:

- `workpads/scaffold/client-server-boundary.md`

Decisions:

- The CLI is one client. It should parse terminal input, submit controller/query requests, render read models, and keep compatibility aliases.
- The controller/query/state/tool layers are the product surface for durable decisions: task/session/agent/run transitions, source binding, project-memory/context delivery, adapter dispatch, permissions, tools, and evidence.
- Do not start a daemon/RPC/server rewrite yet. The immediate issue is not transport; it is product-language controller/query surfaces.
- The next code movement should add product-language query aliases for source tasks/project memory, backed by the current `WorkpadTaskProjection`, before storage renames.

Findings:

- `workpad.rs` still owns too much source-memory workflow composition.
- `project_memory.rs` is currently only a CLI alias; this is acceptable while query/controller aliases are added.
- Dashboard, dogfood readiness, and voice still expose workpad names in presentation/query fields.
- Adapter dispatch preparation still materializes workpad prompt sources directly from workpad projections.
- `tests.rs` is very large and should split later, but splitting it now is less important than cleaning product boundaries.

## S3a - Project Memory Query Alias Surface

Status: completed on 2026-05-26.

Implementation:

- Added `SourceTaskProjection` in `capo-query` as the product-language read model for markdown-backed source tasks.
- Added `ProjectDashboard::source_tasks`, `next_source_task`, and `next_source_task_candidate_count` as query accessors backed by existing workpad task projections.
- Updated `capo project memory next` to use `next_source_task()` and emit source-task fields before appending the compatibility `workpad next` output.
- Added focused query and CLI assertions for `source_task_id`, `source_path`, `observed_source_status`, `capo_binding_status`, and `compatibility_workpad_task_id`.

Decision:

- Query aliases are enough for this step. Persisted `workpad_*` storage, event kinds, and compatibility command output should stay stable until source-binding projections and broader clients use product-language names consistently.

Verification:

- `cargo fmt`
- `cargo test -p capo-query project_dashboard_includes_workpad_tasks -- --nocapture`
- `cargo test -p capo-query project_dashboard_selects_next_actionable_workpad_task -- --nocapture`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`

## S8 - Presentation Vocabulary Cleanup

Status: completed on 2026-05-26.

Implementation:

- Added product-language dogfood readiness aliases: `project_memory_ready`, source-task counts, and `source_task_refs`, while retaining `workpad_bridge_ready`, workpad counts, and `workpad_task_refs`.
- Updated dashboard presentation to render project-memory/source-task/source-binding rows before workpad compatibility rows.
- Updated dogfood readiness CLI and markdown evidence so the primary narrative is project memory and tracked agents rather than moving project workpads into dogfood.
- Updated voice readouts for next work and dogfood readiness with `spoken_source_*` and `spoken_project_memory_*` fields, without expanding voice behavior.

Decision:

- This stays additive. Existing scripts that read workpad keys should keep working, but new client code should key on `source_task`, `source_binding`, and `project_memory_ready`.
- Workpad storage/events remain compatibility internals until more product-facing surfaces have moved and migration risk is justified.

Verification:

- `cargo fmt`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S9 - Bounded Real Local Connector Proof Refresh

Status: completed on 2026-05-26.

Implementation:

- Re-ran the Codex local connector smoke through the existing opt-in, restrictive local subscription harness.
- Recorded the clean passed smoke report in ignored local state at `.capo-dev/scaffold-s9`.
- Exported smoke evidence from Capo read models rather than from raw provider stdout/stderr.
- Indexed markdown-backed project memory into the same ignored state to confirm connector proof and source-task/project-memory readouts coexist in the current scaffold.

Findings:

- Codex CLI is installed as `codex-cli 0.133.0`; Claude Code is installed as `2.1.150`.
- The Codex ignored smoke remains opt-in gated when `CAPO_RUN_CODEX_LOCAL_SMOKE=1` is absent.
- The authorized Codex smoke passed when `CAPO_RUN_CODEX_LOCAL_SMOKE=1` was set.
- Capo's artifact scan reported the smoke artifacts clean, and an additional `rg -a` marker scan over `.capo-dev/scaffold-s9` plus the smoke artifacts returned no credential/session-marker matches.
- The smoke report `adapter-smoke-codex_exec-d38d3f3fee60856c` clears the real-agent connector gate in the isolated S9 state.
- Dogfood readiness remains correctly blocked on runtime target and dispatch-chain evidence; this task did not run a broad real-provider source-task dispatch.

Decision:

- S9 proves the current scaffold can still run a bounded, subscription-backed Codex connector smoke without weakening the credential boundary.
- Do not treat this as full ACP live-server proof or full dogfood readiness. It is connector readiness evidence only.
- The next alignment issue is now the visible CLI surface: `capo workpad ...` still exists for compatibility and should be less prominent than project-memory/source-task commands.

Verification:

- `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`
- `cargo run -q -p capo-cli -- adapter smoke-report scan --artifact-root <local-temp-codex-smoke-artifacts> --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- adapter smoke-report record --adapter codex --status passed --credential-scan clean --marker-found --artifact-root <local-temp-codex-smoke-artifacts> --reason "S9 bounded Codex smoke refreshed against project-memory scaffold" --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- project memory index --root . --state .capo-dev/scaffold-s9`
- `cargo run -q -p capo-cli -- dogfood readiness --state .capo-dev/scaffold-s9`

## S10 - Public CLI Surface Reduction

Status: completed on 2026-05-26.

Implementation:

- Moved `capo workpad ...` commands out of the primary `Usage` list and under a `Compatibility commands` section in CLI help.
- Added a `Primary model` section explaining Capo as a local-first controller/server, the CLI as one client, and markdown-backed planning files as project memory.
- Strengthened the help test so `capo project memory ...` must remain before the compatibility section and `capo workpad ...` must remain after it.

Decision:

- Keep `capo workpad ...` executable for now. Removing it would break existing tests, scripts, and historical dogfood evidence before the replacement source-task paths have fully absorbed those workflows.
- New examples and tests should prefer `capo project memory ...`; workpad commands are now explicitly transitional in user-facing help.

Residual:

- Some command options and historical docs still contain `workpad` vocabulary, such as dashboard compatibility filters and old dogfood/feature evidence. Those are compatibility and history rather than the preferred product surface, but the final alignment audit should decide whether any need another cleanup task.

Verification:

- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`

## S11 - Product Alignment Completion Audit

Status: completed on 2026-05-26.

Artifact:

- `workpads/scaffold/completion-audit.md`

Findings:

- `project.md`, `TASKS.md`, `AGENTS.md`, and `workpads/WORKPADS.md` now point in the requested direction: Capo as server/control plane, CLI as one client, ACP-shaped tracked-agent boundary, and DB-backed markdown project memory.
- The current scaffold has real implementation evidence for product-memory/source-task naming, source bindings, governed `capo.project_memory_read`, deterministic ACP-shaped mock coverage, and bounded Codex connector proof.
- `capo --help` now makes `capo project memory ...` primary and pushes `capo workpad ...` into a compatibility section.
- The goal is still not complete because current operator-facing surfaces still expose workpad-only names in places that are not merely storage internals: dashboard filters, review follow-up options/evidence, dogfood readiness reasons, and tool-wrapper shorthand/error text.

Decision:

- Keep the goal active.
- Continue with small product-language alias tasks before running another completion audit.

Next tasks:

- S12: product-language CLI option aliases for dashboard filters and review follow-up links.
- S13: product-language dogfood readiness reasons.
- S14: project-memory tool wrapper shorthand.

## S4 - Scaffold Gate Review

Status: completed on 2026-05-26.

Artifact:

- `workpads/scaffold/gate-review.md`

Decision:

- The scaffold passes for continued one-task-at-a-time implementation, not for a final architecture freeze.
- The project is now pointed at the intended product spine: local-first control plane, tracked agents, controller/query/state boundaries, project memory/context, governed tools, and evidence.
- Existing workpad vocabulary remains compatibility. New code should prefer project-memory/source-task/source-binding naming.
- Voice, remote clients, rich dashboard work, graph/vector memory, source writeback, and broad provider automation remain deferred until source binding and ACP-shaped tracked-agent paths are cleaner.

Next order:

- Add explicit source-binding projections.
- Move project-memory workflow composition behind reusable controller/query helpers.
- Add deterministic ACP-shaped mock connector coverage.
- Retire workpad vocabulary from shared presentation surfaces.
- Refresh bounded real local connector proof after those foundations are cleaner.

Verification:

- `cargo fmt`
- `cargo test`
- `git diff --check`

## S5 - Explicit Source Binding Projection

Status: completed on 2026-05-26.

Implementation:

- Added `SourceBindingProjection` in `capo-state` with durable projection-log encoding, SQLite storage, rebuild support, and queries by project/task.
- Added `source_bindings` to `ProjectDashboard` so clients can read source bindings through the shared query layer.
- Updated project-memory/workpad import to emit a source binding when binding a markdown source task to an executable Capo task.
- Updated project-memory CLI tests to assert bindings during import and deterministic start-next/dispatch flows.

Decision:

- Source binding is now the product-language link between executable task state and markdown-backed source memory. Existing workpad task projections remain compatibility snapshots of markdown source status.
- Import still writes compatibility summaries and workpad projections for older callers, but new code should read source refs from `SourceBindingProjection`.

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

Follow-up:

- S6 should move project-memory workflow composition behind reusable helper APIs that read `SourceTaskProjection` and `SourceBindingProjection`, leaving workpad commands as compatibility wrappers.

## S6 - Project Memory Controller/Query Helper Surface

Status: completed on 2026-05-26.

Implementation:

- Added `crates/capo-cli/src/project_memory_flow.rs` with product-language helper types and `import_markdown_source_task`.
- `capo project memory import` now calls the helper directly and renders product-language source-task/source-binding fields before compatibility output.
- `capo workpad import` now acts as a compatibility wrapper over the same helper.
- `capo project memory next`, `plan-next`, and `start-next` run source-task selection through `SourceTaskProjection` before appending compatibility behavior.

Decision:

- The helper currently lives in `capo-cli` because there is no durable server/request crate yet. It still moves composition out of text wrappers and gives the next task a concrete surface to move into a controller/server crate.
- `plan-next` and `start-next` still reuse compatibility workpad dispatch rendering internally because adapter prompt sources are still workpad-shaped. That is deliberate until the ACP-shaped mock connector and prompt-source model move to source bindings.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli project_memory_aliases_route_to_markdown_source_adapter -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S7 - Deterministic ACP-Shaped Mock Connector

Status: completed on 2026-05-26.

Implementation:

- Added `ScriptedMockAgent::acp_shaped`, which reuses scripted mock turns but emits ACP-shaped normalized adapter events.
- ACP-shaped mock events use `NormalizedAdapterKind::Acp`, `acp:*` timeline keys, `acp.mock.*` provider event kinds, and the existing stable idempotency path.
- Added `FakeBoundaryController::apply_scripted_acp_mock_turn`.
- Updated `project_memory_scripted_dispatch_proves_narrow_spine` to replay the project-memory turn through the ACP-shaped mock path.

Decision:

- This is intentionally not a full ACP live server/client. It is deterministic connector coverage that brings the scaffold spine closer to the product invariant: tracked agents are observed through ACP-shaped protocol events.
- Real Codex/Claude execution stays opt-in and separate from deterministic CI/local tests.

Verification:

- `cargo fmt`
- `cargo test -p capo-adapters scripted -- --nocapture`
- `cargo test -p capo-cli project_memory_scripted_dispatch_proves_narrow_spine -- --nocapture`
- `cargo test -p capo-adapters`
- `cargo test -p capo-controller`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`

## S12 - Product-Language CLI Option Aliases

Status: completed on 2026-05-26.

Implementation:

- Added `--source-path` and `--source-status` dashboard filters as product-language aliases over the existing source/workpad task query fields.
- Kept `--workpad-path` and `--workpad-status` as compatibility aliases and rejected mixed alias pairs to avoid ambiguous filters.
- Added `--follow-up-source-task` for review findings while retaining `--follow-up-workpad-task` as a compatibility alias.
- Review finding artifacts now render source-task wording first and include a compatibility workpad line without changing persisted `workpad_task_id` or `follow_up` fields.
- CLI help now presents source-task/source-path option names in primary usage and documents compatibility options separately.

Decision:

- Alias parsing is intentionally CLI-local for now. The state/query layers still use compatibility `workpad_*` fields internally until a later migration is justified.
- Mixed product and compatibility aliases are rejected rather than merged or last-write-wins because both names target the same underlying filter/follow-up field.

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

Status: completed on 2026-05-26.

Implementation:

- Replaced workpad-first readiness reasons with product-language reasons:
  - `workpad_index_missing` -> `project_memory_index_missing`
  - `run_workpad_index` -> `run_project_memory_index`
  - `dispatch_chain_missing` -> `source_task_dispatch_chain_missing`
  - `record_or_replay_workpad_dispatch_plan` -> `record_or_replay_source_task_dispatch_plan`
- Added `compatibility_blockers` and `compatibility_next_actions` to dogfood readiness read models.
- CLI, dashboard, dogfood evidence markdown, and voice readouts now prefer product-language blocker/action fields and expose compatibility fields separately.

Decision:

- Compatibility readiness reasons are additive fields, not mixed into the primary blocker/action lists. This keeps new callers on product vocabulary while giving transitional scripts a stable place to read old reason codes.
- Existing workpad readiness booleans, counts, and refs remain in place because they are compatibility read-model fields and still appear in tests and current output.

Verification:

- `cargo fmt`
- `cargo test -p capo-query project_dogfood_readiness_reports_blockers_and_ready_counts -- --nocapture`
- `cargo test -p capo-cli adapter_dogfood_gate_requires_passed_codex_smoke_report -- --nocapture`
- `cargo test -p capo-cli voice_dogfood_readiness_reads_shared_query_without_mutating -- --nocapture`
- `cargo test -p capo-query`
- `cargo test -p capo-cli`
- `cargo test`

## S14 - Project Memory Tool Wrapper Shorthand

Status: completed on 2026-05-26.

Implementation:

- Added `project_memory_read` and `project-memory-read` shorthands for `capo.project_memory_read` in `capo tool run-wrapper`.
- Routed fully qualified `capo.project_memory_read` through the path-based wrapper input handling.
- Kept `workpad_read` and `workpad-read` as compatibility shorthands for `capo.workpad_read`.
- Updated unknown wrapper tool guidance so the product-language shorthand appears before compatibility `workpad_read`.
- Extended wrapper CLI tests to cover product shorthand, fully qualified product tool, compatibility shorthand, and unknown-tool guidance.

Decision:

- `capo.project_memory_read` is now the primary CLI wrapper vocabulary. `capo.workpad_read` remains available only as compatibility for older callers and existing local scripts.

Verification:

- `cargo fmt`
- `cargo test -p capo-cli tool_run_wrapper_exposes_governed_runtime_wrappers_without_providers -- --nocapture`
- `cargo test -p capo-cli`
- `cargo test`
- `git diff --check`
