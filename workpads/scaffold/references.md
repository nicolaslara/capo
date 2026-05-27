# Scaffold Alignment References

## Local Sources

- `project.md` - product source of truth updated for server/control-plane, ACP-tracked agents, and DB-backed project memory direction.
- `TASKS.md` - active workpad queue now routes to scaffold alignment.
- `workpads/WORKPADS.md` - scaffold load list and rules.
- `workpads/prototype/spec.md` - original minimal e2e prototype definition.
- `workpads/architecture/boundaries.md` - boundary map for controller, input, protocol, runtime, state, tools, and memory.
- `workpads/architecture/protocol-provider.md` - Codex, Claude Code, ACP, provider connector, and adapter design.
- `workpads/architecture/state-model.md` - state, event log, projections, and source refs.
- `workpads/architecture/memory-architecture.md` - memory records, provenance, indexes, packets, and external adapter path.
- `workpads/architecture/tool-exposure.md` - tool registry and agent-visible tool/context exposure.
- `workpads/features/dogfood-bridge.md` - current transitional workpad indexing/import implementation.
- `crates/capo-cli/src/workpad.rs` - current top-level workpad command implementation to audit.
- `crates/capo-workpads/src/lib.rs` - markdown scanner/indexer.

## S0 Audit Sources

- `crates/capo-cli/src/main.rs` - top-level CLI command routing.
- `crates/capo-cli/src/workpad.rs` - current `capo workpad ...` implementation.
- `crates/capo-cli/src/dashboard.rs` - dashboard readout names and workpad filters/readiness.
- `crates/capo-cli/src/voice_render.rs` - spoken summary output currently using workpad vocabulary.
- `crates/capo-query/src/summary.rs` - summary/read-model fields including next workpad task candidates.
- `crates/capo-state/src/event.rs` - event kinds and serialized workpad event names.
- `crates/capo-state/src/projections.rs` - workpad task projection tables/read models.
- `crates/capo-workpads/src/lib.rs` - markdown workpad scanner and importer implementation.
- `workpads/architecture/state-model.md` - project/task/source state model using current workpad references.
- `workpads/architecture/memory-architecture.md` - memory/context model that should absorb workpad source material.
- `workpads/architecture/tool-exposure.md` - current tool naming including `capo.workpad_read`.
- `workpads/features/dogfood-bridge.md` - transitional feature plan that currently treats workpad import as dogfood bridge.

## S0a Implementation Sources

- `crates/capo-cli/src/project_memory.rs` - product-language CLI alias layer over the markdown/workpad source adapter.
- `crates/capo-cli/src/main.rs` - routes `capo project memory ...` commands.
- `crates/capo-cli/src/cli_surface.rs` - help text for the new command surface.
- `crates/capo-cli/src/tests.rs` - deterministic alias test covering index, next, plan-next, import, start-next, propose, and apply.

## S1 Spine Sources

- `workpads/scaffold/e2e-spine.md` - narrow product-spine target and next deterministic proof.
- `workpads/prototype/knowledge.md` - fake-controller, CLI, runtime, adapter fixture, recovery, memory, and evidence history.
- `workpads/features/agent-connectors.md` - real Codex smoke and bounded dispatch proof evidence.
- `workpads/features/dogfood-bridge.md` - markdown-backed project source indexing/import/proposal history.
- `workpads/dogfood/knowledge.md` - dogfood checkpoint evidence and remaining manual workflow.
- `crates/capo-adapters/src/scripted_mock_agent.rs` - deterministic scripted adapter event source.
- `crates/capo-controller/src/adapter_replay.rs` - normalized adapter-event replay into controller read models.
- `crates/capo-cli/src/tests.rs` - existing prototype e2e, adapter proof, workpad/project-memory tests.

## S1a Implementation Sources

- `crates/capo-cli/src/tests.rs` - `project_memory_scripted_dispatch_proves_narrow_spine` deterministic e2e proof.
- `crates/capo-cli/src/project_memory.rs` - product-language project-memory command surface used by the proof.
- `crates/capo-controller/src/adapter_replay.rs` - `apply_scripted_mock_turn` path used to replay deterministic adapter events.
- `crates/capo-adapters/src/scripted_mock_agent.rs` - scripted mock turn/event definitions.

## S2 Model Sources

- `workpads/scaffold/project-memory-model.md` - target hierarchy, compatibility map, and migration order.
- `workpads/architecture/state-model.md` - current state/event model with workpad compatibility records.
- `workpads/architecture/memory-architecture.md` - memory record/source/packet model.
- `workpads/architecture/tool-exposure.md` - current Capo tool registry including `capo.workpad_read`.
- `crates/capo-state/src/projections.rs` - projection types for task/session/memory/workpad records.
- `crates/capo-state/src/schema.rs` - current SQLite tables including memory and workpad compatibility tables.
- `crates/capo-query/src/types.rs` - dashboard/readiness query types with workpad compatibility fields.
- `crates/capo-tools/src/lib.rs` - current Capo-owned tool registry.
- `crates/capo-tools/src/runtime_wrappers.rs` - constrained workpad markdown reader wrapper.
- `crates/capo-workpads/src/lib.rs` - markdown source scanner.

## S2a Implementation Sources

- `crates/capo-tools/src/lib.rs` - Capo-owned `capo.project_memory_read` registry definition and context output.
- `crates/capo-tools/src/runtime_wrappers.rs` - governed runtime wrapper alias with constrained markdown source path policy.
- `crates/capo-tools/src/permission.rs` - static read-only/reviewer scope grants.
- `crates/capo-tools/src/tests.rs` - registry, wrapper, policy, and rendering coverage for the project-memory alias.
- `workpads/architecture/tool-exposure.md` - architecture tool list updated to prefer `capo.project_memory_read`.

## S3 Boundary Sources

- `workpads/scaffold/client-server-boundary.md` - client/server boundary decisions and migration tasks.
- `workpads/architecture/boundaries.md` - input surface and controller responsibility contract.
- `workpads/architecture/protocol-provider.md` - adapter/provider boundary and ACP role.
- `crates/capo-cli/src/main.rs` - local CLI router.
- `crates/capo-cli/src/workpad.rs` - transitional markdown source workflow implementation.
- `crates/capo-cli/src/project_memory.rs` - product-language CLI alias layer.
- `crates/capo-cli/src/dashboard.rs` - presentation coupling to workpad query fields.
- `crates/capo-cli/src/dogfood.rs` - readiness presentation coupling to workpad bridge fields.
- `crates/capo-cli/src/voice.rs` and `crates/capo-cli/src/voice_render.rs` - future client surface with workpad presentation names.
- `crates/capo-query/src/types.rs` and `crates/capo-query/src/summary.rs` - shared query fields/accessors to alias next.
- `crates/capo-controller/src/lib.rs` - controller command/session API.

## S3a Implementation Sources

- `crates/capo-query/src/types.rs` - `SourceTaskProjection` product-language query type.
- `crates/capo-query/src/summary.rs` - source-task dashboard accessors and compatibility next-task selection.
- `crates/capo-query/src/lib.rs` - exported source-task query type.
- `crates/capo-query/src/tests.rs` - focused source-task alias coverage.
- `crates/capo-cli/src/project_memory.rs` - `capo project memory next` query alias rendering.
- `crates/capo-cli/src/tests.rs` - CLI assertions for product-language source-task fields and compatibility output.

## S4 Gate Review Sources

- `workpads/scaffold/gate-review.md` - scaffold gate decision, real/stubbed/transitional/deferred classification, and ordered next tasks.
- `workpads/scaffold/tasks.md` - completed scaffold task ledger and verification evidence.
- `workpads/scaffold/knowledge.md` - accumulated scaffold decisions.
- `project.md` - product/control-plane direction.
- `TASKS.md` - active scaffold workpad routing.
- `crates/capo-cli/src/tests.rs` - deterministic project-memory spine and client behavior evidence.
- `crates/capo-query/src/types.rs` and `crates/capo-query/src/summary.rs` - source-task query alias evidence.
- `crates/capo-tools/src/lib.rs` and `crates/capo-tools/src/runtime_wrappers.rs` - governed project-memory tool evidence.

## S5 Implementation Sources

- `crates/capo-state/src/projections.rs` - `SourceBindingProjection` and projection record variant.
- `crates/capo-state/src/schema.rs` - `source_bindings` read-model table and rebuild clearing.
- `crates/capo-state/src/codec_encode.rs` and `crates/capo-state/src/codec.rs` - projection log encoding/decoding for rebuild.
- `crates/capo-state/src/apply.rs` - source-binding projection application.
- `crates/capo-state/src/queries.rs` - source-binding queries by project and task.
- `crates/capo-query/src/types.rs` and `crates/capo-query/src/dashboard.rs` - dashboard query exposure for client surfaces.
- `crates/capo-cli/src/workpad.rs` - import-time source-binding projection record.
- `crates/capo-state/src/tests.rs`, `crates/capo-query/src/tests.rs`, and `crates/capo-cli/src/tests.rs` - focused persistence, query, and product-spine coverage.

## S6 Implementation Sources

- `crates/capo-cli/src/project_memory_flow.rs` - product-language source-task import helper and shared source-binding/default-task IDs.
- `crates/capo-cli/src/project_memory.rs` - project-memory command handlers using source-task selection/import helpers.
- `crates/capo-cli/src/workpad.rs` - compatibility workpad import wrapper over the project-memory helper.
- `crates/capo-cli/src/tests.rs` - product-memory and compatibility workpad regression coverage.

## S7 Implementation Sources

- `crates/capo-adapters/src/scripted_mock_agent.rs` - deterministic ACP-shaped scripted mock adapter events.
- `crates/capo-controller/src/adapter_replay.rs` - controller replay helper for ACP-shaped mock turns.
- `crates/capo-cli/src/tests.rs` - project-memory spine replayed through ACP-shaped mock events.

## S8 Implementation Sources

- `crates/capo-query/src/types.rs` - product-language dogfood readiness aliases and source-task projection types.
- `crates/capo-query/src/dogfood.rs` - dogfood readiness derived from source-task/project-memory aliases while preserving workpad compatibility fields.
- `crates/capo-cli/src/dashboard.rs` - dashboard presentation of project memory, source tasks, and source bindings before workpad compatibility rows.
- `crates/capo-cli/src/dogfood.rs` - dogfood readiness CLI and evidence markdown with project-memory/source-task fields.
- `crates/capo-cli/src/voice_render.rs` - voice readout aliases for source tasks and project-memory readiness.
- `crates/capo-query/src/tests.rs` and `crates/capo-cli/src/tests.rs` - focused and package-level assertions for additive presentation fields.

## S9 Evidence Sources

- `workpads/features/agent-connectors.md` - prior Codex/Claude connector authorization, safety boundaries, and real-smoke evidence contract.
- `workpads/architecture/protocol-provider.md` - subscription-backed connector policy, Codex/Claude adapter shape, and ACP boundary.
- `crates/capo-adapters/src/local_subscription.rs` - restrictive local smoke plans, opt-in env gates, runtime launch policy, and artifact scanner.
- `crates/capo-adapters/src/tests.rs` - ignored Codex/Claude real-smoke tests.
- `crates/capo-cli/src/adapter_smoke.rs` - smoke artifact scan, report recording, status, and redacted evidence export.
- `.capo-dev/scaffold-s9` - ignored local S9 state/evidence containing smoke report `adapter-smoke-codex_exec-d38d3f3fee60856c` and evidence artifact `artifact-adapter-smoke-evidence-4ed49a3bdbe85cd6.md`.

## S10 Implementation Sources

- `crates/capo-cli/src/cli_surface.rs` - CLI help text now separates primary product commands from transitional compatibility workpad commands.
- `crates/capo-cli/src/tests.rs` - help ordering assertions plus product-memory and workpad compatibility command coverage.
- Rendered `capo --help` output - confirms the public CLI surface presents the primary model and compatibility section as intended.

## S11 Audit Sources

- `workpads/scaffold/completion-audit.md` - current completion audit and ordered remaining mismatches.
- `project.md` - product direction evidence.
- `TASKS.md`, `AGENTS.md`, and `workpads/WORKPADS.md` - active scaffold routing and workflow direction.
- `crates/capo-cli/src/cli_surface.rs` - current public CLI help surface.
- `crates/capo-cli/src/dashboard.rs` - dashboard filter vocabulary and source-task/readiness rendering.
- `crates/capo-cli/src/evidence.rs` - review follow-up CLI/evidence vocabulary.
- `crates/capo-cli/src/tool_wrapper.rs` - wrapper shorthand vocabulary.
- `crates/capo-query/src/dogfood.rs`, `crates/capo-cli/src/dogfood.rs`, and `crates/capo-cli/src/voice_render.rs` - readiness blocker/action and presentation vocabulary.

## S12 Implementation Sources

- `crates/capo-cli/src/cli_surface.rs` - primary help now uses source-path/source-status and follow-up-source-task option vocabulary, with compatibility option notes.
- `crates/capo-cli/src/dashboard.rs` - dashboard filter aliases for source/workpad path and status.
- `crates/capo-cli/src/evidence.rs` - review follow-up source-task alias, compatibility alias parsing, and source-first artifact rendering.
- `crates/capo-cli/src/tests.rs` - focused product alias, compatibility alias, help, dashboard, and review artifact regression coverage.

## S13 Implementation Sources

- `crates/capo-query/src/types.rs` - `ProjectDogfoodReadiness` compatibility blocker/action fields.
- `crates/capo-query/src/dogfood.rs` - product-language dogfood readiness blockers and next actions.
- `crates/capo-cli/src/dogfood.rs` - CLI and markdown rendering for primary and compatibility readiness reasons.
- `crates/capo-cli/src/dashboard.rs` - dashboard rendering of primary and compatibility readiness reasons.
- `crates/capo-cli/src/voice_render.rs` - voice readout rendering of primary and compatibility readiness reasons.
- `crates/capo-query/src/tests.rs` and `crates/capo-cli/src/tests.rs` - focused readiness reason regression coverage.

## S14 Implementation Sources

- `crates/capo-cli/src/tool_wrapper.rs` - product-language `project_memory_read` wrapper shorthand, qualified-tool input routing, and unknown-tool guidance.
- `crates/capo-cli/src/tests.rs` - wrapper shorthand and compatibility regression coverage.
