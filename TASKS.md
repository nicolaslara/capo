# Project Task Queue

**You edit this file.** It tells agents which workpad to load for `/next` and similar commands.

Read top to bottom. The **first unchecked** item is the active workpad unless Notes override it. Check items off when a phase is finished, not when pausing mid-phase.

## Active Now

**operator-control** - Build the human operator interaction loop for running Capo agents, starting with a no-planner command REPL over the server boundary.

## Workpad Queue

- [x] **research** - ACP, prior art, subscriptions, local models, memory systems, voice, tunnel/connectivity, and language stack (gate passed 2026-05-25)
- [x] **architecture** - System boundaries, module contracts, data model, security model, and technical plan (gate passed 2026-05-25)
- [x] **prototype** - Minimal e2e Capo that can spawn, track, and interact with at least one coding agent (gate passed with constraints 2026-05-25)
- [x] **features** - Product feature workpads derived from the architecture and prototype (feature gate passed 2026-05-26)
- [x] **dogfood** - Move Capo project execution into Capo itself once stable enough (dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback)
- [x] **scaffold** - Align the implemented scaffold with the intended product spine before more breadth: Capo server/control plane, ACP-tracked agents, simple DB-backed project memory, minimal CLI client, deterministic e2e tests (completed 2026-05-26)
- [x] **server** - Implement the server/control plane, CLI-through-server path, mocked-agent tests, and Codex-backed proof (completed 2026-05-27).
- [x] **harness-research** - Research spike on modern coding-agent harness practice and whether ACP is enough (completed 2026-05-28).
- [ ] **operator-control** - Create a human operator REPL/control surface for inspecting and steering running agents through the Capo server, with planner modes starting at `none`.

## Notes

- The source-of-truth product prompt is captured in `project.md` and `workpads/prototype/spec.md`.
- Research gate passed 2026-05-25. Use `workpads/research/knowledge.md` and `workpads/research/findings/` as architecture inputs.
- Research and architecture may run in parallel only when task boundaries are independent and findings are recorded before implementation decisions.
- Favor Rust for durable controller/core work unless research shows Python ecosystem leverage is materially better for a specific subsystem.
- Python is acceptable for adapters, experiments, local-model integrations, voice pipelines, or memory-system prototypes.
- Do not start dogfooding until the prototype can persist state, show active agent state, and recover from a restart without losing the task ledger.
- Architecture gate passed 2026-05-25. Use `workpads/architecture/gate-review.md` and `workpads/architecture/prototype-plan.md` as prototype inputs.
- Prototype gate passed with constraints 2026-05-25. The local scaffold is proven with fake agents; real Codex/Claude connector proof and workpad import/update safety remain feature/dogfood blockers.
- Feature gate passed 2026-05-26. Real Codex connector proof, workpad indexing/import/proposals, dashboard/query, permissions/tools, memory/eval, voice, remote runtime, and maintainability splits are complete enough to start the dogfood workpad.
- Dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback. Full unattended/source-writing dogfood remains future hardening.
- Current product correction: Capo should not expose `workpad` as a primary product concept. Existing workpad commands are transitional scaffolding for this repository's markdown planning files. The future-facing model is Capo server/controller plus clients, ACP-tracked agents, and DB-backed project/workpad/task memory records that point to markdown files and are exposed to agents through tools/context.
- Scaffold work should prefer the narrow e2e spine over breadth: inspect agents, send instructions, track state, expose requested context/tool activity, persist/recover, and export evidence. Voice, remote clients, rich dashboards, and graph/vector memory should remain planned or stubbed unless needed to prove that spine.
- Server work should make the product-spine real: a durable Capo process owns controller/state/query behavior; local CLI commands become clients of that process; agent interactions are tested deterministically with mocked agents before proving Codex behind the same boundary.
- Server milestone completed 2026-05-27: loopback server, CLI-through-server control, mocked-agent tests, mocked Codex live-run tests, and manual real Codex smoke through the running server are recorded in `workpads/server/tasks.md`.
- Harness research spike completed 2026-05-28: ACP remains the preferred agent/protocol boundary, but the best harnesses add controller-owned runtime, permission, tool instrumentation, checkpoint/recovery, context/memory, evaluation, observability, and multi-client/server layers around it. See `workpads/harness-research/knowledge.md`.
- Operator-control work should make the server usable by a human without memorizing low-level dispatch commands. Start with a no-planner REPL that composes existing server commands; later planner modes may use Codex, Capo, or local models to choose tools/actions.
