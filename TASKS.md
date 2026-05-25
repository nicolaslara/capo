# Project Task Queue

**You edit this file.** It tells agents which workpad to load for `/next` and similar commands.

Read top to bottom. The **first unchecked** item is the active workpad unless Notes override it. Check items off when a phase is finished, not when pausing mid-phase.

## Active Now

`prototype` -> build the minimal e2e Capo scaffold, starting with workspace/toolchain setup.

## Workpad Queue

- [x] **research** - ACP, prior art, subscriptions, local models, memory systems, voice, tunnel/connectivity, and language stack (gate passed 2026-05-25)
- [x] **architecture** - System boundaries, module contracts, data model, security model, and technical plan (gate passed 2026-05-25)
- [ ] **prototype** - Minimal e2e Capo that can spawn, track, and interact with at least one coding agent
- [ ] **features** - Product feature workpads derived from the architecture and prototype
- [ ] **dogfood** - Move Capo project execution into Capo itself once stable enough

## Notes

- The source-of-truth product prompt is captured in `project.md` and `workpads/prototype/spec.md`.
- Research gate passed 2026-05-25. Use `workpads/research/knowledge.md` and `workpads/research/findings/` as architecture inputs.
- Research and architecture may run in parallel only when task boundaries are independent and findings are recorded before implementation decisions.
- Favor Rust for durable controller/core work unless research shows Python ecosystem leverage is materially better for a specific subsystem.
- Python is acceptable for adapters, experiments, local-model integrations, voice pipelines, or memory-system prototypes.
- Do not start dogfooding until the prototype can persist state, show active agent state, and recover from a restart without losing the task ledger.
- Architecture gate passed 2026-05-25. Use `workpads/architecture/gate-review.md` and `workpads/architecture/prototype-plan.md` as prototype inputs.
