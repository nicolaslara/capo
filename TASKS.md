# Project Task Queue

**You edit this file.** It tells agents which workpad to load for `/next` and similar commands.

Read top to bottom. The **first unchecked** item is the active workpad unless Notes override it. Check items off when a phase is finished, not when pausing mid-phase.

## Active Now

**server** - Build the Capo server/control-plane runtime and route client/CLI interactions with agents through it.

## Workpad Queue

- [x] **research** - ACP, prior art, subscriptions, local models, memory systems, voice, tunnel/connectivity, and language stack (gate passed 2026-05-25)
- [x] **architecture** - System boundaries, module contracts, data model, security model, and technical plan (gate passed 2026-05-25)
- [x] **prototype** - Minimal e2e Capo that can spawn, track, and interact with at least one coding agent (gate passed with constraints 2026-05-25)
- [x] **features** - Product feature workpads derived from the architecture and prototype (feature gate passed 2026-05-26)
- [x] **dogfood** - Move Capo project execution into Capo itself once stable enough (dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback)
- [x] **scaffold** - Align the implemented scaffold with the intended product spine before more breadth: Capo server/control plane, ACP-tracked agents, simple DB-backed project memory, minimal CLI client, deterministic e2e tests (completed 2026-05-26)
- [ ] **server** - Implement the server/control plane, CLI-through-server path, mocked-agent tests, and Codex-backed proof.

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
