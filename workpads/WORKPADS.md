# Active Workpads

> Source of truth for workpad context. Which workpad to run is chosen in [`../TASKS.md`](../TASKS.md).

## Current Focus

| Workpad | Status | Description |
| --- | --- | --- |
| **research** | Complete | Gate passed 2026-05-25 — ACP, prior art, stack, memory, subscription, tunnel, local-model, and voice research |
| **architecture** | **Active** | Boundary definitions, data model, contracts, security model, and prototype plan |
| **prototype** | Planned | Minimal e2e Capo controller and agent harness |
| **features** | Planned | Feature-specific workpads after architecture/prototype |
| **dogfood** | Planned | Move Capo project execution into Capo |

## research

**Status:** Complete. Gate passed 2026-05-25. Use as architecture input unless `TASKS.md` reopens research.

**Objective:** Turn the Capo product prompt into sourced recommendations for ACP, prior art, stack, subscription connectors, local models, memory, runtime/tunnel, and input surfaces.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/research/tasks.md
workpads/research/knowledge.md
workpads/research/references.md
```

**Quick nav:**

- `tasks.md` R0-R8: source capture, ACP, prior art, stack, subscriptions, local models, memory, tunnel, voice
- `knowledge.md` Research gate section
- `references.md` primary-source links

**Rules:**

- Prefer upstream docs and repos.
- Record date, license, and maturity where relevant.
- Make recommendations explicit and confidence-scored.
- No broad implementation unless task explicitly authorizes a spike.

## architecture

**Prerequisites:** Research gate passed 2026-05-25.

**Objective:** Convert research into durable boundaries, state/event contracts, capability model, runtime/tunnel plan, protocol/provider plan, memory architecture, and prototype plan.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/architecture/tasks.md
workpads/architecture/knowledge.md
workpads/architecture/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/research/knowledge.md
```

**Quick nav:**

- `boundaries.md` system boundary map and initial contracts
- `state-model.md` operational entities, event log, read models, SQLite/files layout, and restart recovery
- `acp-replay-dedupe.md` ACP `session/load`, `session/resume`, streaming, and dedupe design
- `capability-permissions.md` capability profiles, scopes, grants, permissions, revocation, and ACP option mapping
- `tasks.md` A0-A8: event model, capability model, runtime, security, prototype plan
- `knowledge.md` Architecture gate section

**Rules:**

- Keep connectivity/tunnel, execution runtime, controller, provider, input, state, and memory separate.
- Define interfaces before binding to concrete implementations.
- Record explicit user decisions where product direction is needed.

## prototype

**Prerequisites:** Architecture gate passed unless authorized as a spike.

**Objective:** Build the smallest e2e Capo that can spawn or register an agent, send work, inspect progress, interrupt execution, persist state, and record evidence.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/prototype/spec.md
workpads/prototype/tasks.md
workpads/prototype/knowledge.md
workpads/prototype/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/knowledge.md
```

**Quick nav:**

- `spec.md` Prototype minimum and MVP v0
- `tasks.md` P0-P8 scaffold through dogfood readiness
- `knowledge.md` Prototype gate section

**Rules:**

- Build the smallest product that can really orchestrate an agent.
- Persist enough state to recover after restart.
- Prefer dogfood usefulness over showcase polish.

## features

**Prerequisites:** Prototype gate passed or specific feature spike authorized.

**Objective:** Split post-prototype product work into independently executable feature workpads with dependencies, evidence standards, and review gates.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/features/tasks.md
workpads/features/knowledge.md
workpads/features/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/prototype/knowledge.md
```

**Rules:**

- Split large features into separate workpads once architecture is stable.
- Each feature needs acceptance criteria, evidence, and review requirements.

## dogfood

**Prerequisites:** Prototype gate passed.

**Objective:** Move Capo's own project execution into Capo only after restart recovery, inspection, interruption, and markdown fallback are proven.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/dogfood/tasks.md
workpads/dogfood/knowledge.md
workpads/dogfood/references.md
workpads/prototype/spec.md
workpads/prototype/knowledge.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
```

**Rules:**

- Do not migrate project execution into Capo until restart recovery and state inspection are proven.
- Keep file workpads as the human-auditable fallback until Capo is demonstrably reliable.

## How To Switch Focus

1. Edit `../TASKS.md` Active Now and queue checkboxes.
2. Record why in this file or the target workpad's `knowledge.md` if the switch changes phase order.
3. Load the new workpad context before selecting a task.
