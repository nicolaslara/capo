# Project Memory Model

## Objective

Align Capo's markdown/workpad implementation with the product model: DB-backed project memory that points to source files and is exposed to agents through context packets and tools.

This document classifies current `workpad_*` internals as compatibility where appropriate and defines the target naming for the next migration slices.

## Product Model

Capo manages a project. A project has source documents, task records, context requests, memory records, memory packets, sessions, runs, tool activity, and evidence.

Workpads are not a top-level product surface. They are one markdown source convention Capo can observe. The implementation may keep workpad-named tables and crates during the transition, but new product-facing commands, tools, docs, and tests should use project-memory language.

## V0 Hierarchy

```text
Project
  SourceDocument
    SourceSection
      SourceTask
  Task
    SourceBinding
  Agent
    Session
      Run
      Turn
      ContextRequest
      ToolActivity
      MemoryPacket
      EvidenceRef
  MemoryRecord
    MemorySource
```

### Project

The workspace/repository Capo controls.

Current implementation:

- `ProjectProjection`
- fixed `project-capo` CLI default
- project-local SQLite state root

Target additions:

- project settings should eventually include workspace root, source roots, and default memory indexing policy.
- clients should query project state through controller/query APIs, not by reading workpad files directly.

### SourceDocument

A markdown or external file Capo has observed and can cite.

Fields:

- `source_document_id`
- `project_id`
- `source_kind`: `markdown`, `external_tracker`, `event`, `artifact`
- `path_or_external_ref`
- `content_hash`
- `headings`
- `objective?`
- `observed_at`
- `status`: `observed`, `stale`, `removed`

Current compatibility:

- `WorkpadFileProjection`
- `workpad_files`
- `capo-workpads::WorkpadFileRef`

Migration:

- keep `workpad_files` as the compatibility read model for now.
- add product-language query aliases before renaming storage.
- only migrate table/event names after source/task binding behavior is stable.

### SourceSection

A citeable section inside a source document.

Fields:

- `source_section_id`
- `source_document_id`
- `anchor`
- `title`
- `content_hash?`
- `observed_at`

Current compatibility:

- `WorkpadFileProjection.headings`
- `WorkpadTaskProjection.source_anchor`

Migration:

- split headings into first-class source sections when project-memory search/packets need section-level provenance.
- no immediate code change required for scaffold alignment.

### SourceTask

A task-like source section observed in markdown. It is not the execution authority until imported/bound to a Capo `Task`.

Fields:

- `source_task_id`
- `project_id`
- `source_document_id`
- `source_section_id`
- `title`
- `observed_source_status`
- `source_hash`
- `capo_binding_status`: `observed_only`, `bound`, `stale`, `removed`

Current compatibility:

- `WorkpadTaskProjection`
- `workpad_tasks`
- CLI output compatibility key `workpad_task_id`

Product-facing name:

- `source_task_id`

Migration:

- keep `workpad_task_id` internally while product-facing command output and new tests prefer `source_task_id`.
- add query/dashboard aliases before changing storage.
- `TaskProjection` should eventually carry a structured source binding instead of embedding source facts in `latest_summary`.

### Task

A Capo execution unit.

Fields:

- `task_id`
- `project_id`
- `title`
- `execution_status`
- `active_session_id?`
- `source_binding_id?`
- `latest_summary?`
- `evidence_id?`

Current implementation:

- `TaskProjection`
- source binding currently appears in summaries and workpad import events.

Migration:

- add a source-binding projection before removing summary-encoded source details.
- keep task execution status separate from observed source status.

### SourceBinding

The durable link between a Capo task and source material.

Fields:

- `source_binding_id`
- `project_id`
- `task_id`
- `source_kind`
- `source_task_id?`
- `source_document_id?`
- `source_section_id?`
- `source_path?`
- `source_anchor?`
- `source_hash`
- `observed_source_status`
- `bound_at_sequence`

Current compatibility:

- `WorkpadTaskImported` event payload
- `TaskProjection.latest_summary`
- `WorkpadTaskProjection.capo_execution_status=imported`

Current implementation:

- `SourceBindingProjection`
- `source_bindings`
- `SqliteStateStore::source_bindings`
- `SqliteStateStore::source_binding_for_task`
- `ProjectDashboard::source_bindings`

Migration:

- new code should read source refs from `SourceBindingProjection` rather than parsing `TaskProjection.latest_summary`.
- keep compatibility summary text and workpad task status until project-memory workflow helpers and presentation surfaces move fully to source-binding reads.

### ContextRequest

A request from an agent/session for project memory or context.

Fields:

- `context_request_id`
- `project_id`
- `session_id`
- `run_id?`
- `turn_id?`
- `requested_by`
- `purpose`
- `query`
- `selected_source_refs`
- `tool_call_id?`
- `memory_packet_id?`
- `status`

Current compatibility:

- adapter-native tool calls such as `capo.project_memory_read`
- memory packet refs

Migration:

- add only after the governed project-memory read tool exists.
- for now, S1a proves context request shape through tool activity and memory packet refs.

### MemoryRecord And MemorySource

Derived, reviewable facts and provenance.

Current implementation:

- `MemoryRecordProjection`
- `MemorySourceProjection`
- `memory_records`
- `memory_sources`

Target usage:

- store reviewed decisions, preferences, repo conventions, warnings, and summaries.
- every memory record must point back to source documents, events, artifacts, or external imports.
- generated memory is not authoritative until reviewed/promoted.

### MemoryPacket

The small, replayable context packet sent to an agent for a task/session/turn.

Current implementation:

- `MemoryPacketProjection`
- `memory_packet_refs`
- packet artifacts

Target usage:

- include only a bounded set of task-specific memory.
- record source refs and selection reasons.
- serve as prompt-input evidence, not factual authority.

## Current Compatibility Map

| Current name | Target product meaning | Classification |
| --- | --- | --- |
| `capo workpad ...` | markdown project-memory source adapter commands | compatibility CLI |
| `capo project memory ...` | product-facing project memory command surface | preferred CLI |
| `capo-workpads` | markdown source scanner/indexer | internal adapter, keep for now |
| `WorkpadFileProjection` | `SourceDocument` compatibility projection | keep, alias later |
| `WorkpadTaskProjection` | `SourceTask` compatibility projection | keep, alias later |
| `workpad.indexed` | source documents/tasks observed | persisted compatibility event |
| `workpad.task_imported` | source task bound to Capo task | persisted compatibility event |
| `workpad_task_id` | `source_task_id` for markdown source tasks | compatibility output key |
| `capo.workpad_read` | read markdown project memory | compatibility tool |
| `capo.project_memory_read` | read markdown-backed project memory/context | preferred tool |
| dashboard `workpad_*` fields | project-memory/source-task read models | compatibility query fields |

## Immediate Code Direction

Do now:

- Keep `capo project memory ...` as the preferred CLI path.
- Add `capo.project_memory_read` as a governed tool alias over `capo.workpad_read`.
- Add product-language query/read-model aliases for source tasks before changing storage.
- Make new tests assert `source_task_id`, `project_memory_*`, `context`, and `memory_packet` names where possible.

Do not do yet:

- Rename persisted event kinds or database tables.
- Remove `capo workpad ...`.
- Rename `crates/capo-workpads`.
- Build graph/vector memory.
- Add source writeback.

## Migration Order

1. Tool alias: add governed `capo.project_memory_read` while retaining `capo.workpad_read`.
2. Query aliases: expose source-task/project-memory fields alongside `workpad_*` fields.
3. Source binding: introduce explicit task/source binding projection.
4. Memory packet provenance: build packets from source bindings rather than hard-coded prototype workpad refs.
5. Storage migration: only after read/query/client surfaces use product language consistently.
6. Compatibility retirement: hide or deprecate `capo workpad ...` after tests, docs, and dogfood scripts use product-memory surfaces.

## Open Risks

- Storage migration could break existing local `.capo-dev` evidence. Avoid until compatibility queries exist.
- Dashboard and voice still expose workpad names; those should be presentation aliases before broad UI work.
- The project-memory read tool must not become a raw filesystem escape. Keep path constraints and permission scopes.
- If markdown source roots become configurable, source indexing must exclude generated evidence, vendored repos, scratch clones, and ignored artifacts by default.
