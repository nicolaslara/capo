# AGENTS.md

Repository for **Capo**, a modular controller and harness for managing coding LLM agents. Progress persists in files and git, not conversation context.

## Source Of Truth

| File | Role |
| --- | --- |
| `TASKS.md` | User-edited workpad queue: which phase to work in |
| `project.md` | Product goal, feature set, phases, global backlog |
| `WORKING.md` | Agent loop, review thresholds, gates, verification |
| `workpads/WORKPADS.md` | Per-workpad load lists and commands |
| `workpads/prototype/spec.md` | Prototype and MVP feature tiers |
| `workpads/{workpad}/tasks.md` | Executable tasks |
| `workpads/{workpad}/knowledge.md` | Decisions and lessons |
| `workpads/{workpad}/references.md` | External research |
| `workpads/architecture/boundaries.md` | System boundaries and adapter contracts |
| `workpads/architecture/state-model.md` | State entities, event log, read models, and restart recovery |
| `workpads/architecture/acp-replay-dedupe.md` | ACP streaming replay, `session/load`, and dedupe design |
| `workpads/architecture/capability-permissions.md` | Capability profiles, scopes, grants, permissions, and ACP option mapping |
| `workpads/architecture/runtime-tunnel.md` | Runtime runners, process lifecycle, tunnels, connectivity, and exposure policy |
| `workpads/architecture/protocol-provider.md` | Codex, Claude Code, ACP adapter shape, provider connectors, and subscription policy |
| `workpads/architecture/tool-exposure.md` | Capo tool registry, wrapper tools, instrumentation, ACP/MCP tool mapping |
| `workpads/architecture/memory-architecture.md` | Memory records, provenance, indexes, packets, and external adapter path |
| `workpads/architecture/prototype-plan.md` | Ordered prototype implementation sequence, e2e smoke path, and dogfood prerequisites |
| `workpads/architecture/gate-review.md` | Architecture gate decision, user-sensitive decisions, and prototype residual risks |
| `workpads/scaffold/tasks.md` | Scaffold alignment tasks: product spine, command naming, memory hierarchy, e2e gate |
| `workpads/server/tasks.md` | Server/control-plane tasks: server-owned agent tracking, CLI client path, mocked-agent and Codex proofs |
| `.cursor/commands/next.md` / `.opencode/commands/next.md` | `/next` task-execution command |
| `.agents/skills/next/SKILL.md` | Codex `$next` task-execution skill |

## Resolve Active Workpad

1. Read `TASKS.md`; the first unchecked workpad in the queue is active unless Notes override it.
2. Confirm status and load list in `workpads/WORKPADS.md`.
3. If active workpad is `architecture`, confirm the research gate has passed or that `TASKS.md` explicitly authorizes architecture discovery in parallel.
4. If active workpad is `prototype`, confirm the architecture gate has passed or that `TASKS.md` explicitly authorizes a spike.
5. If active workpad is `dogfood`, confirm the prototype gate has passed.
6. If active workpad is `scaffold`, confirm the architecture/prototype/feature/dogfood history is loaded and treat this as an alignment pass before new breadth.
7. If active workpad is `server`, confirm scaffold alignment is complete and keep work focused on server-owned orchestration before richer clients.

## Current Phase

**Server/control-plane implementation** is active after scaffold alignment completed. The durable target is to make Capo run as a server that owns controller/state/query behavior while CLI and future clients steer tracked agents through that server boundary. Mocked-agent tests come first; Codex must then be proven through the same path.

## Mandatory Workflow

Before task work:

1. `TASKS.md` -> active workpad
2. `project.md`, `WORKING.md`, `workpads/WORKPADS.md`
3. Active workpad `tasks.md`, `knowledge.md`, `references.md`
4. `workpads/architecture/boundaries.md` for architecture, prototype, features, dogfood, and scaffold work
5. `workpads/architecture/state-model.md` for architecture, prototype, features, dogfood, and scaffold work once A2 is complete
6. `workpads/architecture/acp-replay-dedupe.md` for ACP/protocol, state, prototype, features, dogfood, and scaffold work once A2a is complete
7. `workpads/architecture/capability-permissions.md` for permission, runtime, tool, protocol, prototype, features, dogfood, and scaffold work once A3 is complete
8. `workpads/architecture/runtime-tunnel.md` for runtime, tunnel, protocol, provider, prototype, features, dogfood, and scaffold work once A4 is complete
9. `workpads/architecture/protocol-provider.md` for protocol, provider, adapter, prototype, features, dogfood, and scaffold work once A5 is complete
10. `workpads/architecture/tool-exposure.md` for tool, ACP client capability, MCP, runtime wrapper, prototype, features, dogfood, and scaffold work once A5a is complete
11. `workpads/architecture/memory-architecture.md` for memory, retrieval, prompt context, prototype, features, dogfood, and scaffold work once A6 is complete
12. `workpads/architecture/prototype-plan.md` for architecture, prototype, features, dogfood, and scaffold work once A7 is complete
13. `workpads/prototype/spec.md` for prototype, dogfood, and scaffold work
14. For features work, load the feature source file named by the selected task in `workpads/features/tasks.md`
15. Pick a pending task and mark it `in_progress`
16. Complete the acceptance criteria with the smallest correct change
17. Record findings in `knowledge.md` and source links in `references.md`
18. Review per `WORKING.md`
19. Mark complete only after evidence is recorded

## Git Rules

- Do not commit or push without explicit user confirmation.
- If asked to commit, show files and message first.
- No destructive git commands unless explicitly requested.
- Keep generated research clones and scratch artifacts under gitignored paths.

## Research Rules

- Prefer primary sources: upstream repos, docs, protocol schemas, SDK examples, licenses.
- Record dated claims. Agent tooling, ACP, and memory systems change quickly.
- Separate proven facts from recommendations and assumptions.
- Study prior art, but keep Capo's boundaries explicit instead of inheriting another project's architecture by accident.

## Safety Boundary

- Never log API keys, subscription tokens, OAuth tokens, cookies, session files, or voice transcripts containing secrets.
- Treat subscription-backed agent access as a privileged connector, not as an ordinary API key.
- Keep tunnel/connectivity concerns separate from agent execution and controller state.
- Make remote-control capabilities auditable and revocable.

## Verification

**Research:** cited URLs or local paths, license notes, open questions, and recommendation confidence.

**Architecture:** boundary definitions, failure modes, acceptance criteria, and review notes.

**Implementation:** language-specific format/lint/test commands once scaffolded, plus a manual smoke path for spawning and steering at least one local agent.
