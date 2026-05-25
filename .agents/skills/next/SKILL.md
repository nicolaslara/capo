---
name: next
description: Run Capo's next-task workflow from the repository workpads. Use when the user says next, /next, $next, continue, continue with your goal, do the next task, or asks Codex to select and execute the next Capo workpad task.
---

# Capo Next Task

Follow Capo's workpad methodology to select, execute, verify, and record the next task.

## Load State

Read these files first:

1. `TASKS.md` - active workpad queue and phase notes
2. `AGENTS.md`
3. `project.md`
4. `WORKING.md`
5. `workpads/WORKPADS.md`
6. `workpads/{active-workpad}/tasks.md`
7. `workpads/{active-workpad}/knowledge.md`
8. `workpads/{active-workpad}/references.md`

Load these conditionally:

- `workpads/architecture/boundaries.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood`
- `workpads/architecture/state-model.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A2 is complete
- `workpads/architecture/acp-replay-dedupe.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A2a is complete
- `workpads/architecture/capability-permissions.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A3 is complete
- `workpads/architecture/runtime-tunnel.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A4 is complete
- `workpads/architecture/protocol-provider.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A5 is complete
- `workpads/architecture/tool-exposure.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A5a is complete
- `workpads/architecture/memory-architecture.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A6 is complete
- `workpads/architecture/prototype-plan.md` when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A7 is complete
- `workpads/prototype/spec.md` when active workpad is `prototype` or `dogfood`
- `workpads/research/knowledge.md` when active workpad is `architecture`
- `workpads/architecture/knowledge.md` when active workpad is `prototype`
- `workpads/prototype/knowledge.md` when active workpad is `features` or `dogfood`
- The feature source file named by the selected task in `workpads/features/tasks.md` when active workpad is `features`

## Resolve And Gate

- The active workpad is the first unchecked item in `TASKS.md`, unless `TASKS.md` Notes override it.
- Confirm the workpad objective and load list in `workpads/WORKPADS.md`.
- Do not skip gates because a later task looks more concrete.
- If active workpad is `architecture` and the research gate is not passed, stop unless `TASKS.md` explicitly authorizes architecture discovery in parallel.
- If active workpad is `prototype` and the architecture gate is not passed, stop unless `TASKS.md` explicitly authorizes a spike.
- If active workpad is `features` and the prototype gate is not passed, stop unless `TASKS.md` explicitly authorizes a feature spike.
- If active workpad is `dogfood` and the prototype gate is not passed, stop.

## Select Task

Choose a pending or unblocked task based on dependencies, current state, risk, testability, whether it unblocks architecture or dogfooding decisions, and whether it improves the next end-to-end prototype path.

Prefer tasks that produce durable evidence over broad brainstorming.

## Execute

1. Mark the chosen task `in_progress`.
2. Complete the task's acceptance criteria with the smallest correct change.
3. Update `references.md` with primary sources, local paths, dates observed, and license notes where relevant.
4. Update `knowledge.md` with decisions, findings, confidence, rejected options, and open questions.
5. Update `tasks.md` with follow-ups discovered during the work.
6. Assess confidence per `WORKING.md`.
7. Spawn focused review subagents when work is substantial, architecture-changing, security-sensitive, provider/subscription-sensitive, memory-affecting, or confidence is below high.
8. Apply review feedback, record rejected feedback, or ask the user when direction is needed.
9. Mark the task `completed` only when acceptance criteria and review requirements are satisfied.
10. Make an explicit commit decision before another `$next` pass.

## Rules

- The initial product prompt captured in `project.md` remains the source of truth when docs conflict.
- Do not start broad implementation during research or architecture unless explicitly requested.
- Keep controller, protocol adapter, runtime, tunnel, provider, capability, state, memory, evaluation, and input surfaces separate.
- Favor Rust for durable controller/core work unless research shows Python materially reduces risk for a specific subsystem.
- Treat subscription-backed connectors as privileged integrations with explicit security review.
- Do not log secrets, subscription sessions, OAuth tokens, cookies, voice transcripts containing secrets, or provider credentials.
- Do not commit without explicit user confirmation.
- If evidence is weak, record uncertainty instead of guessing.
