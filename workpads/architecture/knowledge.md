# Architecture Knowledge

## Objective

Record the architectural decisions that make Capo modular: each boundary should be explicit enough to implement, test, replace, and review independently.

## Status

Architecture gate not passed.

## Initial Direction

- Keep the controller, agent runtime, connectivity/tunnel, provider connector, state store, memory layer, and input surfaces separate.
- Start with local execution before remote/cloud execution.
- Start with simple durable state and human-readable workpads before advanced memory.
- Build for ACP compatibility, but verify exact protocol fit during research before making it the only agent boundary.

## Research Gate Input

Research gate passed 2026-05-25. Use `workpads/research/knowledge.md` and `workpads/research/findings/` as architecture inputs.

Key research decisions to carry forward:

- ACP should be an adapter boundary, not Capo's core domain model.
- Capo should be a Rust-first hybrid system: Rust controller, SQLite event log, markdown workpads, Python sidecars only where ecosystem leverage warrants it.
- First runtime should be local process execution with explicit capability profiles; do not claim hard sandboxing until OS/container enforcement exists.
- Subscription-backed connectors should use vendor-supported local CLIs/SDKs first; reject web scraping and private endpoint reuse.
- Memory v0 should be markdown plus SQLite; semantic/graph memory is a rebuildable v1 layer.
- Input implementation sequence should be CLI, local dashboard, mobile/PWA, then voice; voice architecture should still be first-class conversational interaction with Capo.
- Source-code architecture inspection favors controller-owned events/read models, raw adapter events mapped into normalized Capo events, local process runtime first, durable permission events, and adapter boundaries for Codex/Claude/ACP.

## User Decisions - 2026-05-25

- First target adapters: Claude Code and Codex. Architecture should treat both as first-class initial targets rather than generic examples.
- Capo should expose tools to agents. Start with easy tools, but design the tool-exposure boundary to grow.
- Capo-exposed tools should wrap existing agent tools where possible so Capo can track, instrument, audit, and eventually enforce policy around tool use.
- ACP streaming replay and restart recovery deduplication needs more research before locking the event model. Track this as an architecture task rather than hand-waving it in A2.
- Initial permissions should be simple and permissive: everything allowed for the early local prototype.
- Permission decisions still need a modular policy architecture so later versions can route decisions through static policy, user approval, or a fast security agent.
- Capo should not expose itself as an ACP agent right now. Capo should be the user's entrypoint and remain primarily a controller/client for the prototype.
- Voice should be a conversational interface to Capo for asking what agents have done, checking status/blockers, discussing next steps, and steering agents. It is not just speech-to-text input.

## A0 - Research Ingestion

Status: completed on 2026-05-25.

Architecture inputs ingested:

- Research gate summary: `workpads/research/knowledge.md`
- ACP protocol mapping: `workpads/research/findings/R1-acp.md`
- Prior-art product comparison: `workpads/research/findings/R2-prior-art.md`
- Prior-art source-code architecture: `workpads/research/findings/R2-code-architecture.md`
- Subscription connector security boundary: `workpads/research/findings/R3-subscriptions.md`
- Stack/runtime/tunnel recommendation: `workpads/research/findings/R4-R6-stack-runtime.md`
- Memory recommendation: `workpads/research/findings/R5-memory.md`
- Input and conversational voice recommendation: `workpads/research/findings/R7-input-surfaces.md`

Architecture direction:

- Use controller-owned event/state IDs and store external adapter IDs separately.
- Persist raw adapter events separately from normalized Capo events.
- Project CLI/dashboard/voice state from Capo read models, not live agent process memory.
- Implement Claude Code and Codex adapters first, with ACP as an adapter boundary rather than the Capo domain model.
- Use Rust for controller, state, runtime supervision, command handling, and audit.
- Use SQLite for operational truth and markdown workpads for human-auditable project state.
- Use local process runtime first; remote, Tailscale, SSH, container, and stronger sandboxing are later adapters.
- Start permissions as all-allowed for trusted local dogfooding while still routing all decisions through a modular policy boundary.
- Make Capo-exposed tools instrumented wrappers so tool calls become durable, auditable events.
- Treat conversational voice as a Capo-facing control surface over the same read models and command envelopes as CLI/dashboard.

Architecture risks:

- **Event identity and replay:** ACP `session/load`, Codex JSONL streams, Claude Code output, and Capo restart recovery can duplicate partial updates unless A2/A2a defines stable idempotency rules.
- **Adapter drift:** Codex/Claude CLI output schemas and subscription semantics can change. Adapter contracts need raw event capture, version metadata, and golden transcript tests.
- **Permission over-simplification:** All-allowed v0 can hide missing policy boundaries. Every allow decision still needs a durable decision source, scope, and audit event.
- **Tool observability gaps:** If Capo only wraps top-level CLI processes, provider-native tools may remain opaque. A5a must define what can be instrumented in v0 and where visibility is deferred.
- **Runtime safety claims:** Local process execution is controllable but not a sandbox. Documentation and UI must not imply stronger isolation than exists.
- **State/source split:** Markdown workpads and SQLite event state can diverge unless architecture defines which store is authoritative for each class of fact.
- **Voice privacy:** Conversational voice can expose sensitive status, code, and credentials through transcripts. Retention/redaction rules must be explicit before implementation.
- **UI ownership:** Dashboard or voice surfaces must not become the owner of orchestration state; they submit commands and render read models only.
- **Naming drift:** Terms like session, run, turn, task, event, checkpoint, agent, adapter, runtime, and tool must be defined before implementation to keep modules readable.

Resolved open questions:

- First concrete agent connectors: Claude Code and Codex.
- Capo modes: rejected as a Capo product model; modes belong to adapters/subagents if present.
- Capo as ACP agent/editor backend: deferred; Capo remains the entrypoint.
- Voice role: first-class conversational interface to Capo, not generic dictation.

## Architecture Gate

Status: not passed.

Required evidence:

- Boundary contracts.
- State/event model.
- Capability model.
- Runtime/tunnel plan.
- Protocol/provider plan.
- Memory architecture.
- Prototype task plan.

## Open Questions

- Should the core process be a long-running server from day one, or a CLI that later grows a daemon?
- Should the first UI be TUI, web dashboard, or both?
- How should partial streaming updates be persisted and replayed without duplicate UI state across ACP `session/load` and Capo restart recovery?
- What is the exact vocabulary for `project`, `agent`, `adapter`, `runtime`, `session`, `run`, `turn`, `task`, `event`, `item`, `tool_call`, `artifact`, and `checkpoint`?
- Which data belongs only in SQLite, which belongs in markdown workpads, and which is mirrored between them?
