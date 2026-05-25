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
