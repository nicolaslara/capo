# Capo Memory Architecture

## Objective

Define Capo's prototype memory architecture: v0 storage and indexing, how operational state references derived memory, and the migration path toward layered/fractional memory without turning memory into a second source of truth.

This is the A6 architecture artifact. It keeps Capo local-first and auditable while leaving room for semantic search, temporal graph memory, and optional external sync after dogfood traces prove the need.

## Design Rules

- Operational truth remains in SQLite events, projections, and artifacts.
- Human project truth remains in markdown workpads and source files until an explicit export/writeback feature changes that.
- Memory is derived context with provenance, confidence, scope, and invalidation metadata.
- Every memory record shown to an agent must point back to a local event, artifact, markdown source, or explicitly imported external record.
- Derived memory is rebuildable. Capo can delete and regenerate memory indexes without losing operational history.
- Generated memory is not automatically trusted. Important decisions stay in reviewed markdown or explicit event/evidence records.
- Raw secrets, credential material, vendor subscription sessions, browser cookies, and raw voice transcripts are excluded from long-term memory by default.
- Memory packets are fractional and task-specific. Capo should never dump all project memory into an agent context.
- External memory systems are adapters or mirrors, not authoritative stores.

## Static Dispatch Shape

Prototype enum:

```text
enum MemoryBackend {
  Markdown(MarkdownMemoryBackend),
  SqliteFts(SqliteFtsMemoryBackend),
  External(ExternalMemoryAdapter),
  Fake(FakeMemoryBackend),
}
```

Prototype implementation order:

1. `MarkdownMemoryBackend` for workpad/source pointers and curated markdown facts.
2. `FakeMemoryBackend` for controller e2e tests.
3. `SqliteFtsMemoryBackend` for local full-text search over memory records, selected artifacts, summaries, and workpad sections.
4. `ExternalMemoryAdapter` only after local memory packets and provenance are proven.

No semantic/vector/graph backend is required for the first prototype.

## Memory Layers

| Layer | Authority | Storage | Prototype role |
| --- | --- | --- | --- |
| Raw event memory | SQLite event log | `events`, `artifacts`, raw-event tables | Audit, recovery, and extraction source. |
| Human project memory | Markdown files | `workpads/**`, source docs | Planning and decision source. |
| Curated fact memory | Derived from reviewed sources | `memory_records`, `memory_sources`, optional markdown mirror | Reusable facts and preferences. |
| Retrieval memory | Rebuildable indexes | SQLite FTS/read models; later vector/graph sidecars | Search and ranking. |
| Prompt memory | Generated per run/turn | `memory_packets`, artifacts | Small context packets injected into agents. |
| External mirrors | Optional adapters | Tana/Capacities/mem0/Zep/Letta/etc. | Import/export/sync only, not recovery authority. |

## V0 Storage

V0 uses the existing `.capo/` project-local layout:

```text
.capo/
  capo.sqlite
  artifacts/
    memory/
      packets/
      extracts/
      exports/
```

Markdown remains in the repo:

```text
workpads/
  architecture/knowledge.md
  research/findings/*.md
  prototype/knowledge.md
```

Rules:

- `workpads/*/knowledge.md` is the human-facing decision memory for each phase.
- Generated memory artifacts live under `.capo/artifacts/memory/` unless reviewed into markdown.
- Capo stores source pointers and content hashes for markdown sections it indexes.
- A generated summary cannot supersede a reviewed workpad decision without an explicit `memory.record_promoted` or future markdown export event.

## Core Records

### MemoryRecord

Derived reusable memory item.

Fields:

- `memory_record_id`
- `project_id`
- `scope`: `global`, `project`, `repo`, `task`, `agent`, `session`, `user`, `provider`
- `scope_owner_ref`
- `subject_ref?`
- `sensitivity_classification`: `public`, `internal`, `sensitive`, `secret_derived`
- `record_kind`: `decision`, `preference`, `repo_convention`, `lesson`, `summary`, `fact`, `warning`, `retrieval_hint`
- `subject`
- `predicate`
- `object`
- `body`
- `confidence`: `high`, `medium`, `low`
- `review_state`: `generated`, `reviewed`, `rejected`, `superseded`
- `source_count`
- `valid_from?`
- `valid_until?`
- `supersedes_memory_record_id?`
- `revoked_by_memory_record_id?`
- `redaction_state`
- `created_at`
- `updated_at`

`scope_owner_ref` is a typed resource reference such as project ID, task ID, agent ID, session ID, user ID, provider connector ID, repo root, or global owner. Permission checks and packet filtering use `scope_owner_ref`, `subject_ref`, and `sensitivity_classification`, not free-text `subject` / `predicate` / `object`.

### MemorySource

Provenance edge from a memory record to a source.

Fields:

- `memory_source_id`
- `memory_record_id`
- `source_kind`: `event`, `artifact`, `markdown`, `external_import`
- `source_event_id?`
- `source_artifact_id?`
- `source_path?`
- `source_anchor?`
- `source_content_hash?`
- `source_sequence?`
- `quote_artifact_id?`
- `observed_at`

### MemoryIndexEntry

Rebuildable search index metadata.

Fields:

- `memory_index_entry_id`
- `memory_record_id`
- `index_kind`: `sqlite_fts`, `embedding`, `graph`
- `index_version`
- `indexed_text_hash`
- `backend_ref?`
- `status`: `indexed`, `stale`, `failed`
- `indexed_at`

### MemoryPacket

Task-specific context packet prepared for an agent/session/turn.

Fields:

- `memory_packet_id`
- `project_id`
- `task_id?`
- `agent_id?`
- `session_id?`
- `run_id?`
- `turn_id?`
- `purpose`: `startup`, `turn_context`, `review`, `recovery`, `voice_summary`
- `budget_tokens`
- `selection_policy`
- `included_items_json`
- `excluded_items_json`
- `explanation_artifact_id?`
- `packet_artifact_id`
- `created_at`

Packet candidates that are still being planned live in `MemoryJob` state, not `memory_packets`. A `MemoryPacket` row is written only once `packet_artifact_id` exists. The packet artifact is the replayable prompt-input evidence; source events and workpads remain the factual authority.

### MemoryJob

Async or inline extraction/rebuild job.

Fields:

- `memory_job_id`
- `project_id`
- `source_query_json`
- `job_kind`: `extract_facts`, `index_fts`, `build_packet`, `invalidate`, `export`, `rebuild`
- `status`: `queued`, `running`, `completed`, `failed`, `canceled`
- `started_at?`
- `completed_at?`
- `emitted_sequence_start?`
- `emitted_sequence_end?`
- `error?`

## MemoryBackend Contract

Implementation-facing contract:

```text
ingest(MemorySourceRef) -> MemoryRecordId
extract(MemoryExtractionRequest) -> Vec<MemoryRecordCandidate>
index(MemoryRecordId, IndexPolicy) -> IndexResult
search(MemoryQuery, MemoryBudget) -> Vec<MemoryHit>
build_packet(TaskContext, MemoryBudget) -> MemoryPacket
explain(MemoryHitId | MemoryPacketId) -> MemoryExplanation
invalidate(MemoryRecordId, Reason) -> InvalidationResult
promote(MemoryRecordId, ReviewDecision) -> PromotionResult
export(MemoryExportRequest) -> MemoryExportResult
rebuild(MemoryRebuildRequest) -> RebuildResult
```

Rules:

- `ingest` and `extract` must attach at least one `MemorySource`.
- `search` filters out invalidated, rejected, superseded, unauthorized, or redacted records unless the caller has explicit scope.
- `build_packet` stores a packet artifact plus an explanation of why items were included or excluded before the packet is attached or injected.
- `promote` changes review state; it does not rewrite source events or markdown.
- `rebuild` can recreate indexes and generated records from source ranges with idempotency keys.

## Fractional Memory Packets

A memory packet is the small, explainable fraction of memory sent to an agent for a task.

Packet sections:

1. Active task/workpad anchors.
2. Required architecture/spec files.
3. Recent session/run summary.
4. Reviewed decisions and repo conventions.
5. Relevant retrieved facts/snippets.
6. Warnings about invalidated or stale facts when relevant.
7. Evidence links and review requirements.

Selection constraints:

- Each section has a token budget.
- Reviewed memory outranks generated memory.
- Current workpad/task facts outrank broad/global facts.
- Facts with `valid_until`, `revoked_by`, or stale source hashes are excluded by default.
- Secret-bearing and raw voice transcript artifacts are excluded.
- Every included item carries source refs and a short inclusion reason.

## Operational State Relationship

State owns:

- session/run/turn lifecycle
- tool calls and permissions
- evidence/evaluations
- workpad observations
- artifacts and raw adapter/runtime events

Memory owns:

- reusable facts extracted from state/files
- retrieval indexes over state/files
- packet selection and explanations
- invalidation/supersession metadata for derived records

Boundaries:

- A `MemoryRecord` may reference an event/artifact, but it does not replace the event.
- A `MemoryPacket` attached to a run/turn is prompt-input evidence and must be exactly replayable through its packet artifact. It does not become factual authority; source events and workpads remain the source of truth for the facts it contains.
- `memory_refs` remains the compatibility/provenance projection linking state sources to memory records.
- Dashboard/voice can summarize memory, but they render source links and review state.

## Privacy And Capability Rules

Memory access uses `PermissionPolicy`.

Scopes:

- `memory:read:record`
- `memory:search:project`
- `memory:build_packet:session`
- `memory:write:generated`
- `memory:promote:reviewed`
- `memory:invalidate:record`
- `memory:export:project`
- `memory:sync:external`

Rules:

- Trusted local prototype may allow read/search/build packet for project memory, but still emits permission and grant-use events.
- External sync/export requires explicit scope and connector policy.
- Raw voice transcript artifact retention is separate from memory ingestion. Long-term memory may store reviewed/redacted transcript summaries only. Raw transcript memory ingestion is out of scope unless a future high-risk feature explicitly adds it with separate policy and review.
- Credential material and subscription session material are never valid memory sources.

## External Memory Adapters

Prototype stance:

- No external memory backend is required for v0.
- Tana, Capacities, Zep, Graphiti, mem0, Letta, vector DBs, and PKM tools are optional references or future adapters.
- Capo-owned events, artifacts, markdown, and memory records remain exportable without these systems.

Adapter rules:

- External adapters implement import/export/sync, not primary recovery.
- Every imported record gets `source_kind = external_import`.
- Every exported record carries source refs, redaction state, and review state.
- External sync is disabled by default and permission-gated.

Candidate progression:

1. Local SQLite FTS5 over memory records and selected artifacts.
2. Local semantic sidecar only after real dogfood traces show FTS is insufficient.
3. Temporal graph experiment with Graphiti-like validity windows only if Capo needs "what was true when?" queries beyond timestamped facts.

## State Model Additions

Add tables:

```text
memory_records(memory_record_id, project_id, scope, scope_owner_ref_json, subject_ref_json, sensitivity_classification, record_kind, subject, predicate, object, body, confidence, review_state, source_count, valid_from, valid_until, supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state, created_at, updated_at)
memory_sources(memory_source_id, memory_record_id, source_kind, source_event_id, source_artifact_id, source_path, source_anchor, source_content_hash, source_sequence, quote_artifact_id, observed_at)
memory_index_entries(memory_index_entry_id, memory_record_id, index_kind, index_version, indexed_text_hash, backend_ref, status, indexed_at)
memory_packets(memory_packet_id, project_id, task_id, agent_id, session_id, run_id, turn_id, purpose, budget_tokens, selection_policy, included_items_json, excluded_items_json, explanation_artifact_id, packet_artifact_id, created_at)
memory_jobs(memory_job_id, project_id, source_query_json, job_kind, status, started_at, completed_at, emitted_sequence_start, emitted_sequence_end, error)
```

Add events:

- `memory.job_requested`
- `memory.job_completed`
- `memory.record_ingested`
- `memory.record_promoted`
- `memory.record_invalidated`
- `memory.record_superseded`
- `memory.index_updated`
- `memory.packet_built`
- `memory.packet_attached`
- `memory.export_requested`
- `memory.export_completed`

## Read Model Additions

`MemoryRecordReadModel`:

- reviewed/generated/rejected/superseded status
- source refs and confidence
- validity window and redaction state

`MemoryPacketReadModel`:

- packet purpose, budget, included item counts, source refs, and explanation artifact

`SessionReadModel`:

- attached memory packet IDs and latest packet explanation

`TaskReadModel`:

- relevant reviewed decisions, warnings, and stale-memory alerts

## Prototype Scope

In scope:

- Memory schema and events.
- Markdown/workpad source pointers.
- Generated memory records with provenance and review state.
- Memory packet build for fake-agent e2e tests.
- Excluding invalidated/rejected/superseded records from packets.
- No raw voice transcript or credential material memory sources.

Deferred:

- Embeddings.
- Vector DB sidecar.
- Graph memory.
- External memory sync.
- Automatic markdown writeback.
- Human review UI for memory promotion.

## Test Strategy

Prototype tests should prove:

1. A memory record cannot be ingested without source provenance.
2. Rejected, invalidated, superseded, or unauthorized records are excluded from packets.
3. A memory packet includes source refs and inclusion reasons for every item.
4. An attached or injected memory packet always has a packet artifact and can be replayed exactly.
5. Rebuilding indexes from events/files produces the same searchable record IDs.
6. Raw voice transcripts and credential material are rejected as memory sources by default.
7. External sync/export is denied without explicit memory/export scope.
8. `FakeMemoryBackend` can build a packet attached to a fake run/turn for the e2e prototype.

## Recommendation

Implement v0 with markdown source pointers, SQLite memory records, and a fake packet builder before adding FTS. The first useful product behavior is not semantic search; it is proving that Capo can attach small, source-linked, review-aware context packets to agent runs while preserving operational truth in events and project truth in workpads.

Confidence: high for v0 local-first storage and packet boundaries. Confidence is medium for the v1 retrieval backend sequence until dogfood traces show actual retrieval failures and temporal query needs.
