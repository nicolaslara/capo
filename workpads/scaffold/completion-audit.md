# Scaffold Product Alignment Audit

Date: 2026-05-26

## Question

Can the scaffold alignment goal be called complete against the user objective?

Short answer: yes for this scaffold-alignment pass. The project is now pointed at the requested product spine: Capo as a local-first server/control plane with clients, ACP-compatible tracked-agent boundaries, DB-backed markdown project memory, and product-language CLI/tool/readout surfaces. `capo workpad ...`, workpad-named storage, and workpad compatibility fields remain only as transitional compatibility and implementation details.

## Requirements Checked

### Project Direction

Evidence:

- `project.md` says Capo is a local-first server/control plane.
- `project.md` says the local CLI is one client and should not become the product domain model.
- `project.md` says tracked agents are represented through the agent/protocol boundary, with ACP preferred where it fits.
- `project.md` says project/workpad/task concepts are Capo memory/planning records, not a top-level CLI product surface.
- `TASKS.md`, `AGENTS.md`, and `workpads/WORKPADS.md` route scaffold work around the same product correction.

Result: satisfied. The product source of truth matches the requested direction.

### Public CLI Shape

Evidence:

- Rendered `capo --help` places `capo project memory ...` in primary usage.
- Rendered `capo --help` moves `capo workpad ...` under `Compatibility commands`.
- Rendered `capo --help` uses `--source-path`, `--source-status`, and `--follow-up-source-task` in primary command usage.
- Rendered `capo --help` documents `--workpad-path`, `--workpad-status`, and `--follow-up-workpad-task` only as compatibility options.
- `crates/capo-cli/src/tests.rs` asserts the help ordering and product-language aliases.

Result: satisfied for the current scaffold. Workpad commands remain executable for existing scripts, but they are no longer the primary public model.

### Code Alignment

Evidence:

- `crates/capo-cli/src/project_memory.rs` exposes product-facing project-memory commands.
- `crates/capo-cli/src/project_memory_flow.rs` centralizes source-task import and source-binding composition.
- `crates/capo-query/src/types.rs` exposes `SourceTaskProjection`, source bindings, and product-language dogfood readiness fields.
- `crates/capo-cli/src/dashboard.rs`, `dogfood.rs`, and `voice_render.rs` emit source-task/project-memory fields before workpad compatibility fields.
- `crates/capo-cli/src/evidence.rs` accepts `--follow-up-source-task` and renders source-task review evidence first.
- `crates/capo-cli/src/tool_wrapper.rs` accepts `project_memory_read` and `project-memory-read` before compatibility `workpad_read`.
- `crates/capo-cli/src/main.rs` still routes `capo workpad ...` compatibility commands.

Result: satisfied for scaffold alignment. Compatibility internals remain by design; new user-facing behavior has product-language paths.

### ACP-Tracked-Agent Direction

Evidence:

- `workpads/architecture/protocol-provider.md` defines Codex, Claude Code, and ACP adapter/provider boundaries.
- `ScriptedMockAgent::acp_shaped` emits ACP-shaped normalized adapter events.
- `project_memory_scripted_dispatch_proves_narrow_spine` exercises source-task dispatch, project-memory tool activity, recovery, and evidence through ACP-shaped mock events.
- Real Codex connector smoke was refreshed in S9 and recorded as bounded connector evidence.

Result: satisfied for scaffold alignment. This is not a full live ACP server/session implementation; that remains future product work, not a hidden blocker for this alignment pass.

### DB-Backed Markdown Project Memory

Evidence:

- `workpads/scaffold/project-memory-model.md` defines the SourceDocument/SourceSection/SourceTask/SourceBinding hierarchy.
- `source_bindings` read-model projection exists and is covered by state/query tests.
- `capo.project_memory_read` exists as the preferred governed tool alias.
- `capo project memory index|next|start-next|import|propose|apply` exists and has deterministic tests.
- The deterministic spine test starts from markdown-backed project memory and verifies state, source binding, project-memory tool activity, recovery, evidence export, and unchanged source markdown.

Result: satisfied enough for v0 scaffold. Workpad-named tables/events remain compatibility internals until a later migration is justified.

### Task Ordering

Evidence:

- `workpads/scaffold/tasks.md` completed S0-S14 in order: product audit, project-memory CLI alias, narrow e2e spine, model alignment, governed tool alias, client/server boundary, query/source-binding, gate review, helper extraction, ACP-shaped mock, presentation cleanup, connector proof, public CLI reduction, completion audit, option/readiness/tool-wrapper cleanup.
- Broad features remain deferred or compatibility-only unless needed for the core spine.
- `cargo test` and `git diff --check` passed after the final S14 implementation.

Result: satisfied. The task queue iterated toward the requested final shape rather than adding more breadth.

## Residual Compatibility

- `capo workpad ...` remains executable under a compatibility help section.
- Workpad-named projection tables, event kinds, and tests remain because renaming persisted storage is a separate migration risk.
- `capo.workpad_read`, `workpad_read`, `workpad_*` output fields, and `spoken_workpad_*` readouts remain where explicitly marked compatibility.

These are documented compatibility surfaces, not primary product direction.

## Decision

The scaffold alignment objective is complete for this pass. Future work should start from the product-language surfaces and treat storage/event renames, hiding/removing `capo workpad ...`, a live ACP session implementation, richer server transports, graph/vector memory, voice UX, and remote clients as normal backlog rather than blockers for this alignment goal.
