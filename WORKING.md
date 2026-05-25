# Working Practices

Living project-wide agreement for how agents work on **Capo**. Complements `project.md` and the workpads. Update when the workflow changes.

## Purpose

Build a maintainable controller for orchestrating coding agents with explicit boundaries, review loops, and dogfooding pressure. Agents execute tasks, assess confidence, invite critique when needed, and ask the user on product-sensitive decisions.

## General

Whenever a task is too complex, spawn the strongest available analysis agents to compare options and produce reviewable findings. Use faster agents for well-defined file creation or mechanical expansion only after the target structure is clear.

## Workarounds

Prefer the right fix over a shortcut. Workarounds are only acceptable when necessary to unblock progress, time-box a spike, or isolate unknowns.

When using a workaround:

1. Notify the user in the same turn: what was done, why the proper fix was deferred, and what the proper fix looks like.
2. State confidence: whether this is likely the right long-term solution.
3. Add a review task in the active workpad's `tasks.md`, or `project.md` backlog if cross-cutting.
4. Explore when unsure: use review subagents for non-trivial tradeoffs in architecture, security, subscriptions, permissions, memory, or data model.

Do not silently ship workarounds or leave them undocumented.

## Core Loop

Use this loop for `/next`, `$next`, and similar task execution. The concrete command prompt lives in `.cursor/commands/next.md` and `.opencode/commands/next.md`; the Codex skill lives in `.agents/skills/next/SKILL.md`.

1. Read `TASKS.md` and resolve the active workpad.
2. Load `AGENTS.md`, `project.md`, `WORKING.md`, and `workpads/WORKPADS.md`.
3. Load the active workpad's `tasks.md`, `knowledge.md`, and `references.md`.
4. For architecture, prototype, features, or dogfood work, also load `workpads/architecture/boundaries.md`.
5. After A2 is complete, also load `workpads/architecture/state-model.md` for architecture, prototype, features, or dogfood work.
6. After A2a is complete, also load `workpads/architecture/acp-replay-dedupe.md` for ACP/protocol, state, prototype, features, or dogfood work.
7. After A3 is complete, also load `workpads/architecture/capability-permissions.md` for permission, runtime, tool, protocol, prototype, features, or dogfood work.
8. After A4 is complete, also load `workpads/architecture/runtime-tunnel.md` for runtime, tunnel, protocol, provider, prototype, features, or dogfood work.
9. After A5 is complete, also load `workpads/architecture/protocol-provider.md` for protocol, provider, adapter, prototype, features, or dogfood work.
10. After A5a is complete, also load `workpads/architecture/tool-exposure.md` for tool, ACP client capability, MCP, runtime wrapper, prototype, features, or dogfood work.
11. After A6 is complete, also load `workpads/architecture/memory-architecture.md` for memory, retrieval, prompt context, prototype, features, or dogfood work.
12. After A7 is complete, also load `workpads/architecture/prototype-plan.md` for architecture, prototype, features, or dogfood work.
13. For prototype or dogfood work, also load `workpads/prototype/spec.md`.
14. Select a task by dependencies, risk, and testability.
15. Mark the task `in_progress` before doing work.
16. Complete acceptance criteria with the smallest correct change.
17. Verify per the task's evidence standard.
18. Record findings, decisions, and open questions in the workpad or project docs.
19. Assess confidence and use review subagents per thresholds below.
20. Incorporate review feedback, record rejections, or ask the user when product-sensitive.
21. Mark `completed` only when acceptance criteria and review requirements are satisfied.
22. Before another `/next` pass: explicit commit decision - commit, or record why not.

## Verification

Every task needs evidence before marking complete. Match depth to scope:

| Change touches | Minimum verification |
| --- | --- |
| Research only | Primary/source links, dated notes, license notes where relevant, open questions |
| Architecture docs | Boundary review, failure modes, explicit assumptions, user-sensitive decisions called out |
| Rust code | `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` once a Cargo project exists |
| Python code | Formatter/linter/test commands chosen by scaffold, recorded before first implementation |
| Agent runtime | Spawn/stop/recover smoke with logs stripped of secrets |
| Dashboard/UI | Local browser smoke and screenshot/checklist for core interactions |
| Voice or subscription connectors | Secret-handling review, transcript/session storage decision, manual smoke with dummy data where possible |

Record skipped verification in the task or `knowledge.md` with a reason.

## Workpad Gates

1. Research gate: `workpads/research/knowledge.md` records enough prior-art, ACP, stack, connector, and memory findings to choose a prototype direction.
2. Architecture gate: `workpads/architecture/knowledge.md` records boundary definitions, data model, security model, and prototype plan.
3. Prototype gate: `workpads/prototype/knowledge.md` records a working e2e smoke: spawn, track, interact, persist, and inspect at least one agent.
4. Dogfood gate: `workpads/dogfood/knowledge.md` records criteria and migration plan for moving Capo project execution into Capo.

Unless `TASKS.md` Notes override, do not implement broad feature work before the architecture gate.

## Confidence Assessment

| Level | Meaning | Expected action |
| --- | --- | --- |
| High | Strong evidence; narrow, verified scope | Proceed; periodic review on important deliverables |
| Medium | Likely correct but assumptions or weak tests | Prefer focused review before completion |
| Low | Unclear requirements, fragile integration, weak sources | Review or user direction before calling complete |

Consider: acceptance criteria met, tests/smoke evidence, cohesive boundaries, permission/security implications, provider lock-in, recovery behavior, and unresolved product choices.

## Review Subagents

Spawn when work is substantial, architecture-changing, security-sensitive, provider/subscription-sensitive, memory-affecting, or confidence is below high.

Useful lenses:

- Protocol fit: ACP compatibility, adapter boundaries, capability negotiation
- Architecture: controller/runtime/connectivity/state separation
- Security/privacy: subscription sessions, tunnel auth, secrets, logs, transcripts
- Test adequacy: smoke proves the intended behavior and fails for the right reasons
- Prior art: what existing systems do well or poorly
- Product fit: dogfood path stays real, not theoretical
- Code quality: minimal, idiomatic implementation in the chosen language

## Acting On Feedback

- Fix clearly correct issues in scope.
- Record accepted decisions in `knowledge.md`.
- Record rejected feedback when it affects future work.
- Ask the user on product tradeoffs or scope changes.

## Documentation

Prefer clear names. Comment non-obvious invariants: permission boundaries, recovery behavior, session ownership, secret handling, and protocol compatibility assumptions.

## Project-Level Knowledge

| Doc | Use |
| --- | --- |
| `project.md` | Product goal, phases, global backlog |
| `TASKS.md` | User-controlled workpad queue |
| `WORKING.md` | This file |
| `workpads/*/knowledge.md` | Workpad-specific decisions |
| `workpads/architecture/boundaries.md` | Boundary contracts and architecture map |
| `workpads/architecture/state-model.md` | State entities, event log, read models, and restart recovery |
| `workpads/architecture/acp-replay-dedupe.md` | ACP replay and dedupe rules |
| `workpads/architecture/capability-permissions.md` | Capability and permission model |
| `workpads/architecture/runtime-tunnel.md` | Runtime and connectivity model |
| `workpads/architecture/protocol-provider.md` | Adapter and provider connector model |
| `workpads/architecture/tool-exposure.md` | Tool registry, wrappers, instrumentation, ACP/MCP tool mapping |
| `workpads/architecture/memory-architecture.md` | Memory records, provenance, indexes, packets, and external adapter path |
| `workpads/architecture/prototype-plan.md` | Ordered prototype sequence, e2e smoke path, and dogfood prerequisites |

## Phase Focus

| Phase | Workpad | `/next` reads |
| --- | --- | --- |
| Complete | `research` | `research/tasks.md`, `knowledge.md`, `references.md` |
| Complete | `architecture` | `architecture/tasks.md`, `knowledge.md`, `references.md`, `boundaries.md`, `state-model.md`, `acp-replay-dedupe.md`, `capability-permissions.md`, `runtime-tunnel.md`, `protocol-provider.md`, `tool-exposure.md`, `memory-architecture.md`, `prototype-plan.md`, `gate-review.md` |
| Now | `prototype` | `prototype/spec.md`, `tasks.md`, `knowledge.md`, `references.md`, architecture artifacts including `prototype-plan.md` and `gate-review.md` |
| Later | `features` | Feature-specific tasks after prototype |
| Later | `dogfood` | Migration of Capo project execution into Capo |

## Dependency Policy

When adding or bumping a dependency, check the current upstream release and license first. Record intentional pins and ecosystem constraints in the relevant workpad.

## Research Vs Implementation

- Research: reference only unless a task explicitly authorizes a spike.
- Architecture: boundary and contract definitions before broad implementation.
- Prototype: smallest useful end-to-end Capo, not a complete product.
- Feature work: starts after prototype evidence and workpad breakdown.
