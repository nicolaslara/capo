# R5 - Memory Systems

Observed: 2026-05-25

## Recommendation

Capo should treat memory as layered, auditable project state, not as one opaque "agent memory" product.

Shortlist:

- **v0:** Markdown/file-backed workpads plus a local SQLite event log. This gives Capo a human-readable source of truth and a durable operational ledger before adding embeddings or graph extraction.
- **v1:** Add a memory adapter interface with two optional backends: local semantic search over Capo-owned records, and a temporal graph experiment using Graphiti or a Graphiti-like model if temporal fact invalidation becomes important.
- **Do not use as source of truth:** Tana, Capacities, Zep Cloud, mem0 Cloud, or Letta Cloud. They are useful references or optional sync/export targets, but Capo's durable state should remain local and exportable by default.

Confidence: **medium-high** for v0, **medium** for v1. The v0 choice is supported by Capo's current workpad model and SQLite's durability/portability. The v1 backend choice should wait until Capo has real dogfood traces to evaluate retrieval quality and maintenance cost.

## Capo Memory Requirements

Capo needs several kinds of memory:

- **Operational memory:** sessions, runs, agent lifecycle events, tool calls, interrupts, reviews, failures, approvals, and recovery checkpoints.
- **Project memory:** workpads, task state, decisions, architecture boundaries, research findings, review notes, and source links.
- **Reusable agent memory:** user preferences, repo conventions, recurring fixes, agent skill notes, and lessons from prior runs.
- **Derived semantic memory:** embeddings, graph facts, summaries, and retrieval hints derived from the first three layers.

Only the first two should be authoritative in v0. Derived memory must be rebuildable from primary records.

## v0 Baselines

### Markdown/File-Backed Baseline

Use Markdown files as the human-auditable memory layer:

- Keep current workpads as canonical project memory.
- Add frontmatter only where it materially helps indexing, for example `scope`, `status`, `updated`, `confidence`, `sources`, and `supersedes`.
- Keep files git-friendly and readable without Capo.
- Use append-only sections for dated decisions and lessons where churn matters.
- Generate summaries from files, but do not make generated summaries the only copy of a decision.

Benefits:

- Best data ownership story: copy the repo and the memory moves with it.
- Works with existing developer tools, review, diffs, branches, and search.
- Fits Capo dogfooding: if Capo breaks, humans and other agents can still work from the files.

Risks:

- No transactional concurrency across many writers.
- Search quality is limited without an index.
- Agents can create messy, duplicated, or stale notes unless Capo enforces structure and review.

### SQLite/Event-Log Baseline

Use SQLite as the local operational state store and event log:

- Store append-only events for sessions, runs, agent messages, tool calls, permission decisions, task transitions, evidence, review outcomes, and memory extraction jobs.
- Store projections for current state, but keep them rebuildable from events.
- Store source pointers into Markdown files rather than duplicating all project truth.
- Add FTS5 tables for local full-text search once the event volume justifies it.
- Export JSONL and Markdown reports from SQLite for backup, audit, and migration.

SQLite is a strong fit because it is single-file, serverless, zero-configuration, transactional, public-domain software, and its database file format is portable across platforms. FTS5 is part of SQLite's amalgamation and supports local full-text indexing.

Risks:

- SQLite is excellent for a local controller, but not a multi-writer remote database across network filesystems.
- WAL/backups need correct handling; Capo should use SQLite backup APIs or controlled export, not naive file copies.
- Event schema discipline matters. If events are too unstructured, memory extraction becomes a replay of messy logs.

Primary sources:

- SQLite overview: https://www.sqlite.org/about.html
- SQLite FTS5: https://www.sqlite.org/fts5.html

## External And Adjacent Systems

| System | What To Learn | Ownership/Export Story | License / Maturity | Capo Fit |
| --- | --- | --- | --- | --- |
| **Tana** | Node graph, supertags, fields, fast capture, structured notes as user-managed knowledge. | Tana documents workspace export as Markdown or JSON, and larger node downloads as Markdown or Tana Paste. Its Input API is write-only today; read access is "not yet available." | Proprietary SaaS. | Useful optional personal-PKM export/sync target, not a Capo backend. Write-only API and proprietary cloud make it unsuitable as operational memory. |
| **Capacities** | Object-first notes, typed properties, collections, Markdown/frontmatter export, local-link graph export. | Full export is documented as Markdown with frontmatter, CSV collections, media folders, and local links. Automated export can run daily/weekly/monthly if the app is open. API is beta and token access reaches all data. Capacities says it is not end-to-end encrypted. | Proprietary SaaS. | Better export story than many PKM tools, but still an optional human notebook. Not a Capo source of truth. |
| **Zep / Graphiti** | Temporal knowledge graph, provenance, fact invalidation, hybrid retrieval across semantic/keyword/graph traversal. | Graphiti is self-hosted OSS. Zep is managed or private-cloud platform with users, sessions, message storage, dashboards, SDKs, and graph infrastructure. Zep sessions integrate messages into a user-level graph rather than isolating all memory by session. | Graphiti: Apache-2.0. Python 3.10+. Requires graph/search backends such as Neo4j, FalkorDB, Kuzu, or Amazon Neptune/OpenSearch, plus LLM/embedder providers. | Best v1 research candidate for temporal/project fact memory if Capo needs "what was true when?" queries. Too operationally heavy for v0. |
| **mem0** | Simple add/search/update/delete memory API, user/agent/session memory separation, vector plus graph/reranker options, cloud/OSS split. | Platform exports can download JSON, including structured exports filtered by user/agent/run/session, but export jobs expire after 7 days. OSS can be self-hosted with configurable vector store/LLM/embedder/reranker. | Apache-2.0 repo. Python 3.10+ OSS stack. Common vector stores include Qdrant and Postgres + pgvector. | Good v1 semantic-memory adapter candidate. Avoid making it the canonical store; use Capo-owned raw events and exportable facts underneath. |
| **Letta** | Stateful agents, memory blocks, message persistence, runs/steps, and git-backed memory files in Letta Code/MemFS. | Letta API stores messages and memory state; Letta Code local mode stores agent state locally, and MemFS exports agent memory to a local directory. MemFS is git-backed Markdown, with `system/` files pinned into context and non-system files progressively disclosed. | Apache-2.0 repo. Letta Code and Letta API are more of an agent platform than a library. | Valuable design reference, especially git-backed context repositories. As a dependency it overlaps too much with Capo's controller goal. |
| **Obsidian-style local Markdown** | Local-first note ownership and Markdown vault ergonomics. | Files are already Markdown on disk. | App is proprietary; data format is open enough for Capo's purposes. | Reinforces v0 file-backed memory. Capo should not depend on Obsidian, but Markdown vault compatibility is useful. |
| **Chroma / Qdrant / pgvector** | Local or self-hosted vector retrieval primitives. | Data ownership depends on deployment. pgvector keeps vectors with Postgres data; Chroma and Qdrant can run locally or managed. | Chroma docs state Apache-2.0. Qdrant is open source. pgvector is open-source for Postgres. | Candidate implementation details under a Capo semantic-memory adapter, not memory systems by themselves. For v1, prefer pgvector only if Capo has already moved to Postgres; otherwise local SQLite plus a small vector sidecar is simpler. |

Primary sources:

- Tana Input API: https://outliner.tana.inc/learn/features/input-api
- Tana export: https://outliner.tana.inc/learn/features/copy-paste-and-export
- Tana terms/privacy/security: https://tana.inc/pages/terms-privacy-security
- Capacities API: https://docs.capacities.io/developer/api
- Capacities export: https://docs.capacities.io/reference/export
- Capacities import/bulk import: https://docs.capacities.io/reference/import and https://docs.capacities.io/reference/bulk-import
- Capacities E2EE note: https://docs.capacities.io/more/end-to-end-encryption
- Graphiti repo: https://github.com/getzep/graphiti
- Zep sessions: https://help.getzep.com/v2/sessions
- Zep graph concepts: https://help.getzep.com/v2/understanding-the-graph
- Zep vs Graphiti: https://help.getzep.com/docs/faq/zep-vs-graphiti
- Zep paper: https://arxiv.org/abs/2501.13956
- mem0 repo: https://github.com/mem0ai/mem0
- mem0 platform overview: https://docs.mem0.ai/platform/overview
- mem0 OSS configuration: https://docs.mem0.ai/open-source/configuration
- mem0 exports: https://docs.mem0.ai/cookbooks/essentials/exporting-memories
- Letta repo: https://github.com/letta-ai/letta
- Letta stateful agents: https://docs.letta.com/guides/core-concepts/stateful-agents
- Letta Code memory: https://docs.letta.com/letta-code/memory/
- Letta MemFS: https://docs.letta.com/letta-code/memfs/
- Letta local mode: https://docs.letta.com/letta-code/local-mode
- Letta context repositories: https://www.letta.com/blog/context-repositories
- Chroma OSS: https://docs.trychroma.com/docs/overview/oss
- pgvector: https://github.com/pgvector/pgvector

## Data Ownership And Export Story

Capo should make the ownership hierarchy explicit:

1. **Authoritative local records:** SQLite event log and Markdown workpads. These are Capo-owned, local-first, inspectable, and exportable.
2. **Rebuildable indexes:** FTS, embeddings, graph facts, summaries, and retrieval caches. These can be deleted and rebuilt from authoritative records.
3. **Optional external mirrors:** Tana, Capacities, mem0 Cloud, Zep Cloud, Letta Cloud, or user PKM tools. These may improve UX, but cannot be required for recovery.

Minimum export formats:

- `events.jsonl` for the operational event log.
- `state.sqlite` plus documented schema migrations for local restore.
- Markdown workpad archive with relative links intact.
- `memory-facts.jsonl` for extracted facts with source pointers, timestamps, confidence, scope, and invalidation metadata.
- Optional `graph.jsonl` or GraphML/RDF export if Capo adopts graph memory later.

Data ownership rule: every derived memory shown to an agent must include provenance back to a local event, local file, or explicitly user-imported external record.

## Layered / Fractional Memory Model

Recommended model:

1. **Raw event memory:** append-only events in SQLite. This is the legal/audit/recovery ledger.
2. **Human project memory:** Markdown files in workpads. This is the shared human-agent planning surface.
3. **Curated fact memory:** compact assertions extracted from events and files, for example repo conventions, user preferences, architectural decisions, and "avoid doing X" lessons.
4. **Retrieval memory:** FTS, embeddings, rerankers, and graph traversal over raw and curated records.
5. **Prompt memory:** the small fraction injected into an agent's context for a specific task.

"Fractional memory" means Capo should never hand the agent a monolithic memory dump. It should compose a task-specific memory packet:

- current task and active workpad,
- relevant boundary/spec files,
- recent run summary,
- top curated facts with source links,
- top retrieved snippets,
- explicit stale/invalidated facts to avoid if relevant.

Every memory packet should be explainable: why each item was included, what source supports it, and whether it is pinned, retrieved, or inferred.

## Operational Risks

- **Memory poisoning:** Agents can write plausible but false summaries. Mitigation: extracted facts require provenance, confidence, scope, and review state; important decisions stay in Markdown.
- **Stale facts:** User preferences, repo structure, and provider behavior change. Mitigation: include `valid_from`, optional `valid_until`, `supersedes`, and `revoked_by` fields.
- **Prompt bloat:** Memory can consume the context window and degrade work. Mitigation: token budgets per memory layer and a visible "why included" report.
- **Provider leakage:** Cloud memory tools may send project content, secrets, or transcripts to third parties. Mitigation: local-first default; explicit connector permissions; redaction before external sync.
- **Backend lock-in:** Managed memory services optimize for their APIs and dashboards. Mitigation: Capo-owned raw events and exportable memory facts.
- **Extraction drift:** LLM extractors may invent facts, over-generalize one run, or merge unrelated identities. Mitigation: source-linked extraction, local review, and periodic rebuilds from raw logs.
- **Graph complexity:** Temporal graphs are powerful but operationally expensive. Mitigation: defer Graphiti/Zep-style memory until Capo has real temporal queries that files/SQLite cannot answer.
- **Concurrent writes:** Multiple agents can update files and memory at once. Mitigation: SQLite transactions for events; git branches/worktrees or file locks for Markdown; review before merging generated memory.

## v0 Design Sketch

Tables/projections to define in architecture:

- `events(id, ts, actor, kind, run_id, session_id, task_id, payload_json, redaction_state)`
- `artifacts(id, event_id, path, content_hash, media_type, summary)`
- `memory_facts(id, scope, subject, predicate, object, source_event_id, source_path, confidence, valid_from, valid_until, revoked_by, review_state)`
- `memory_packets(id, run_id, generated_at, budget_tokens, inputs_json, included_items_json)`

Markdown conventions:

- `workpads/*/knowledge.md` remains human-facing decision memory.
- `workpads/research/findings/*.md` remains sourced research memory.
- Future generated memory should live outside human-authored workpads unless reviewed, for example `memory/generated/` or a Capo-owned `.capo/memory/` directory decided in architecture.

## v1 Design Sketch

Add a `MemoryBackend` adapter:

- `ingest(record)`
- `search(query, scope, filters, budget)`
- `explain(result_id)`
- `invalidate(record_id, reason, source)`
- `export(format)`
- `rebuild(source_range)`

Initial backend candidates:

1. **Local FTS backend:** SQLite FTS5 over events, artifacts, and facts.
2. **Local semantic backend:** embeddings over curated facts and selected artifacts; implementation can use a sidecar vector store or a later Postgres/pgvector path.
3. **Temporal graph backend:** Graphiti experiment for entities/facts with validity windows and provenance.

Do not expose external backends directly to agents. Capo should mediate retrieval, redaction, provenance, and token budgets.

## Open Questions

- Should Capo keep memory under the project repo, a global Capo home directory, or both?
- What is the first privacy boundary: per-project, per-user, per-agent, per-runtime, or per-provider?
- Should generated memory require human review before it can become prompt-pinned?
- How much of the event log can be retained without leaking secrets or proprietary code into future tasks?
- Does Capo need temporal graph queries in v1, or is timestamped fact invalidation enough?
- Which vector path best matches the eventual stack: SQLite extension/sidecar, Qdrant, Chroma, or Postgres + pgvector?

