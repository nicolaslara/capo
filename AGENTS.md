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
| `workpads/operator-control/tasks.md` | Human operator control loop tasks: REPL, planner modes, attach/jump agent context, command rendering |
| `workpads/goal-orchestration/tasks.md` | Capo-owned goal lifecycle, agent reporting, evidence/story projections, continuation, validation, and historical reports |
| `workpads/dashboard-webclient/tasks.md` | Browser dashboard/web client tasks: design, review, acceptance, implementation, screenshot review, and iteration |
| `workpads/harness-research/daily-driver-review.md` | Systematic per-dimension daily-driver review and phased roadmap that motivates the harness track |
| `workpads/real-turn-loop/tasks.md` | Real controller turn loop, provider-neutral `AgentAdapter` trait, one real Codex workspace-write adapter, and the minimal safety floor (active) |
| `workpads/streaming-transport/tasks.md` | Streaming runtime, JSON-RPC framing, event-tail subscribe, multi-turn thread, interrupt, and the server-side SSE/HTTP contract |
| `workpads/tools-aci/tasks.md` | Real tool path, typed narrow tool I/O, edit/patch/search/test ACI quality, instrumentation, and the GO2 reporting/evidence tools |
| `workpads/safety-gates/tasks.md` | Permission enforcement in the loop, grant read-back/revoke, real verification runner, checkpoint/rollback, and liveness recovery |
| `workpads/goal-autonomy/tasks.md` | Implementation of the goal-orchestration design: goal/evidence model, continuation scheduler, evidence-gated auditor, reattach-after-compaction |
| `workpads/depth/tasks.md` | Live ACP/Claude adapters, real memory packet/FTS5 retrieval, OS sandbox/worktrees, and optional OTel observability |
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
8. If active workpad is `operator-control`, confirm the server milestone is complete and keep work focused on human input/client surfaces that use the server boundary.
9. If active workpad is `goal-orchestration`, confirm operator-control is stable enough to inspect goal/report/story state, and keep work focused on server/controller-owned objectives rather than provider-native goal state.
10. If active workpad is `dashboard-webclient`, confirm server/query contracts are stable enough for the selected slice and keep work focused on browser client UX, visual QA, and screenshot-reviewed iteration.

## Current Phase

**real-turn-loop** is active as of 2026-05-29, the first workpad of the **daily-driver harness track** (server/CLI): real-turn-loop -> (streaming-transport || tools-aci) -> safety-gates -> goal-autonomy -> depth. The track comes from the systematic daily-driver review in `workpads/harness-research/daily-driver-review.md` (verdict: disciplined bones, unbuilt loop) and was decomposed and adversarially reviewed on 2026-05-29. The critical path is a real controller turn loop plus one real workspace-write adapter, because the controller is currently fake-only and no provider can edit code end-to-end yet.

**goal-orchestration** is now the canonical goal-loop DESIGN source (GO0-GO14); its implementation is realized by `goal-autonomy` (and the GO2 reporting tools in `tools-aci`) on the real-turn-loop substrate, and it closes as "design realized" after goal-autonomy. Operator-control remains the human CLI/input surface for inspecting and steering running agents through the server boundary.

The web UI (`web/app`, `web/dashboard`) is owned by a separate agent and is out of scope for the harness track; those workpads deliver only the server-side streaming contract (evolving `crates/capo-web`).

Dashboard-webclient first slice is complete as of 2026-05-28. It is a
dependency-free static webclient under `web/dashboard/` with fixture-backed
read models, a mocked server-command API, and browser screenshot evidence.

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
15. For operator-control work, load `workpads/server/knowledge.md` for the current server command evidence
16. For goal-orchestration work, load `workpads/harness-research/knowledge.md`, `workpads/operator-control/knowledge.md`, and `workpads/server/knowledge.md`
17. For dashboard-webclient work, load `workpads/features/dashboard.md`, `workpads/operator-control/knowledge.md`, and `workpads/goal-orchestration/knowledge.md`
18. Pick a pending task and mark it `in_progress`
19. Complete the acceptance criteria with the smallest correct change
20. Record findings in `knowledge.md` and source links in `references.md`
21. Review per `WORKING.md`
22. Mark complete only after evidence is recorded

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
