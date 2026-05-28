# Active Workpads

> Source of truth for workpad context. Which workpad to run is chosen in [`../TASKS.md`](../TASKS.md).

## Current Focus

| Workpad | Status | Description |
| --- | --- | --- |
| **research** | Complete | Gate passed 2026-05-25 — ACP, prior art, stack, memory, subscription, tunnel, local-model, and voice research |
| **architecture** | Complete | Gate passed 2026-05-25 — boundary definitions, data model, contracts, security model, and prototype plan |
| **prototype** | Complete | Gate passed with constraints 2026-05-25 — local scaffold, fake-agent e2e, dashboard, voice contract, evidence export |
| **features** | Complete | Feature gate passed 2026-05-26 — real connectors, dogfood bridge, dashboard/query, permissions/tools, memory/eval, voice, remote runtime, and maintainability splits |
| **dogfood** | Complete | Dogfood gate passed 2026-05-26 — Capo-assisted development with markdown/git fallback |
| **scaffold** | Complete | Completed 2026-05-26 — product-spine alignment, project-memory aliases, deterministic scaffold proofs |
| **server** | Complete | Completed 2026-05-27 - Capo server/control plane, CLI-through-server path, mocked-agent tests, mocked Codex path, and manual real Codex smoke |
| **harness-research** | Complete | Completed 2026-05-28 - best practices for coding-agent harnesses and ACP's role as adapter boundary |
| **operator-control** | **Active** | Build a human operator REPL/control surface for inspecting and steering running agents through the server |

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

**Status:** Complete. Architecture gate passed 2026-05-25.

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
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/research/knowledge.md
```

**Quick nav:**

- `boundaries.md` system boundary map and initial contracts
- `state-model.md` operational entities, event log, read models, SQLite/files layout, and restart recovery
- `acp-replay-dedupe.md` ACP `session/load`, `session/resume`, streaming, and dedupe design
- `capability-permissions.md` capability profiles, scopes, grants, permissions, revocation, and ACP option mapping
- `runtime-tunnel.md` runtime runners, local process lifecycle, remote runtime abstraction, connectivity/tunnel boundary, and exposure policy
- `protocol-provider.md` Codex, Claude Code, ACP adapter shape, provider connectors, credential scopes, and subscription policy
- `tool-exposure.md` Capo tool registry, wrapper tools, instrumentation, ACP/MCP tool mapping, and observed-only native tools
- `memory-architecture.md` memory records, provenance, indexing, memory packets, and external adapter path
- `prototype-plan.md` ordered implementation sequence, e2e smoke path, and dogfood prerequisites
- `gate-review.md` architecture gate result, user-sensitive decisions, and residual prototype risks
- `tasks.md` A0-A8: event model, capability model, runtime, security, prototype plan
- `knowledge.md` Architecture gate section

**Rules:**

- Keep connectivity/tunnel, execution runtime, controller, provider, input, state, and memory separate.
- Define interfaces before binding to concrete implementations.
- Record explicit user decisions where product direction is needed.

## prototype

**Status:** Complete. Gate passed with constraints 2026-05-25.

**Prerequisites:** Architecture gate passed 2026-05-25 unless explicitly reopened.

**Objective:** Build the smallest e2e Capo that can spawn or register an agent, send work, inspect progress, interrupt execution, persist state, and record evidence.

**Gate result:** The local scaffold is proven with fake agents, SQLite state/recovery, text dashboard, voice contract, Capo tools, memory packet refs, and markdown evidence export. Real subscription-backed connector proof and workpad import/update safety move to feature/dogfood work.

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
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/architecture/knowledge.md
```

**Quick nav:**

- `spec.md` Prototype minimum and MVP v0
- `tasks.md` P0-P15 scaffold through dogfood readiness
- `knowledge.md` Prototype gate section

**Rules:**

- Build the smallest product that can really orchestrate an agent.
- Persist enough state to recover after restart.
- Prefer dogfood usefulness over showcase polish.

## features

**Status:** Complete. Feature gate passed 2026-05-26.

**Prerequisites:** Prototype gate passed with constraints on 2026-05-25.

**Objective:** Split post-prototype product work into independently executable feature workpads with dependencies, evidence standards, and review gates.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/features/tasks.md
workpads/features/knowledge.md
workpads/features/references.md
workpads/features/agent-connectors.md
workpads/features/dogfood-bridge.md
workpads/features/dashboard.md
workpads/features/permissions-tools.md
workpads/features/memory-eval.md
workpads/features/voice.md
workpads/features/remote-runtime.md
workpads/features/state-store.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/prototype/knowledge.md
```

**Rules:**

- Split large features into separate workpads once architecture is stable.
- Each feature needs acceptance criteria, evidence, and review requirements.
- Start with real local agent connector proof or the dogfood bridge, depending on whether the next pass prioritizes actual agent execution or importing Capo's own workpads.

## dogfood

**Status:** Complete. Dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback.

**Prerequisites:** Prototype gate passed and feature gate passed.

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
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Rules:**

- Do not migrate project execution into Capo until restart recovery and state inspection are proven.
- Keep file workpads as the human-auditable fallback until Capo is demonstrably reliable.

## scaffold

**Status:** Complete. Scaffold alignment completed 2026-05-26.

**Prerequisites:** Architecture, prototype, feature, and dogfood gates are recorded as passed. The scaffold product-direction correction is complete.

**Objective:** Align the implemented scaffold with the intended product spine: Capo is a server/control plane with clients; tracked agents are represented through ACP-compatible protocol boundaries; project/workpad/task memory is data in Capo's DB pointing to markdown source files; the local CLI is one client for inspecting and steering tracked agents, not the domain model itself.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/scaffold/tasks.md
workpads/scaffold/knowledge.md
workpads/scaffold/references.md
workpads/prototype/spec.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/features/agent-connectors.md
workpads/features/dogfood-bridge.md
workpads/features/tasks.md
workpads/dogfood/knowledge.md
```

**Rules:**

- Prefer product-language surfaces: project, task, memory, context, agent, session, dispatch, evidence.
- Treat `capo workpad ...` as transitional development scaffolding for this repo's markdown files, not the long-term user-facing API.
- Keep voice, mobile, remote clients, rich dashboards, remote runtime adapters, graph/vector memory, and source-writing dogfood deferred or stubbed unless a task proves they are needed for the core spine.
- The next e2e proof should show a client talking to Capo, Capo tracking agents through protocol-shaped events, DB-backed project memory/context exposed to agents, persisted state/recovery, and evidence export.
- Use static dispatch where it keeps boundaries readable; allow a simpler alternative only when it improves naming, testability, and modularity without coupling controller, protocol, runtime, tools, and memory.

## server

**Status:** Complete. Milestone completed 2026-05-27.

**Prerequisites:** Scaffold alignment completed 2026-05-26.

**Objective:** Make Capo run as a server/control plane that owns agent tracking, state, and query behavior while local CLI/client surfaces interact with agents through that server boundary.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/server/tasks.md
workpads/server/knowledge.md
workpads/server/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/scaffold/knowledge.md
```

**Rules:**

- The server/control plane owns controller, state, query, and recovery behavior.
- The CLI is a client of the server boundary, not the owner of agent orchestration.
- Test with deterministic mocked agents first, then prove Codex through the same boundary.
- Keep tunnel/connectivity, runtime execution, protocol adapters, memory, and input surfaces modular.

## harness-research

**Status:** Complete. Research spike completed 2026-05-28.

**Objective:** Document best-known practices for building coding-agent harnesses,
compare modern harnesses and agent products, and answer whether ACP is enough
for Capo's harness architecture.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/harness-research/tasks.md
workpads/harness-research/knowledge.md
workpads/harness-research/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Quick nav:**

- `knowledge.md` executive answer, ACP coverage/gaps, harness practices,
  comparison matrix, and Capo recommendations
- `references.md` dated source links and notes

**Rules:**

- Treat ACP as an adapter/protocol boundary, not the whole harness.
- Do not inspect or rely on leaked proprietary source.
- Prefer primary sources and official docs; label closed-product observations as
  lower confidence.

## operator-control

**Status:** Active.

**Prerequisites:** Server/control-plane milestone completed 2026-05-27.

**Objective:** Give humans an ergonomic operator loop for inspecting and steering running Capo agents through the server boundary, starting with a no-planner command REPL and preserving a path to planner-backed modes.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/operator-control/tasks.md
workpads/operator-control/knowledge.md
workpads/operator-control/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/server/knowledge.md
```

**Rules:**

- The REPL is an input surface/client; it must not own orchestration state.
- `--planner none` is command-driven and deterministic. Future planner modes may call tools/LLMs, but must use the same server/control boundaries.
- Prefer human-readable summaries inspired by operator CLIs; keep machine-ish evidence available through existing commands.
- Do not bypass Capo server commands to mutate agent/session/runtime state.
- Keep live provider execution behind existing explicit opt-in gates.

## How To Switch Focus

1. Edit `../TASKS.md` Active Now and queue checkboxes.
2. Record why in this file or the target workpad's `knowledge.md` if the switch changes phase order.
3. Load the new workpad context before selecting a task.
