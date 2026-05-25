# Next: Do Next Task

Follow the Capo workpads methodology to complete the next task.

## Step 1: Read State Files

Read these files first:

1. `TASKS.md` - active workpad queue and phase notes
2. `AGENTS.md`
3. `project.md`
4. `WORKING.md`
5. `workpads/WORKPADS.md`
6. `workpads/{active-workpad}/tasks.md`
7. `workpads/{active-workpad}/knowledge.md`
8. `workpads/{active-workpad}/references.md`
9. `workpads/architecture/boundaries.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood`
10. `workpads/architecture/state-model.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A2 is complete
11. `workpads/architecture/acp-replay-dedupe.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A2a is complete
12. `workpads/architecture/capability-permissions.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A3 is complete
13. `workpads/architecture/runtime-tunnel.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A4 is complete
14. `workpads/architecture/protocol-provider.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A5 is complete
15. `workpads/architecture/tool-exposure.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A5a is complete
16. `workpads/architecture/memory-architecture.md` - when active workpad is `architecture`, `prototype`, `features`, or `dogfood` after A6 is complete
17. `workpads/prototype/spec.md` - when active workpad is `prototype` or `dogfood`
18. `workpads/research/knowledge.md` - when active workpad is `architecture`
19. `workpads/architecture/knowledge.md` - when active workpad is `prototype`
20. `workpads/prototype/knowledge.md` - when active workpad is `features` or `dogfood`

## Step 2: Resolve Active Workpad

- The active workpad is the first unchecked item in `TASKS.md`, unless `TASKS.md` Notes override it.
- Confirm the active workpad's objective in `workpads/WORKPADS.md` and the active workpad's `tasks.md`.
- Do not skip gates just because a later task looks more concrete.

## Step 3: Gate Check

- If active workpad is `architecture` and the research gate is not passed, stop unless `TASKS.md` explicitly authorizes architecture discovery in parallel.
- If active workpad is `prototype` and the architecture gate is not passed, stop unless `TASKS.md` explicitly authorizes a spike.
- If active workpad is `features` and the prototype gate is not passed, stop unless `TASKS.md` explicitly authorizes a feature spike.
- If active workpad is `dogfood` and the prototype gate is not passed, stop.

## Step 4: Select A Task

Choose a pending or unblocked task based on:

- Dependencies
- Current state
- Risk
- Testability
- Whether it unblocks architecture or dogfooding decisions
- Whether it improves the next e2e prototype path

Prefer tasks that produce durable evidence over broad brainstorming.

## Step 5: Execute

1. Mark the task `in_progress`.
2. Complete the task's acceptance criteria with the smallest correct change.
3. Update `references.md` with primary sources, local paths, dates observed, and license notes where relevant.
4. Update `knowledge.md` with decisions, findings, confidence, rejected options, and open questions.
5. Update `tasks.md` with follow-ups discovered during the work.
6. Assess confidence per `WORKING.md`.
7. Spawn focused review subagents when work is substantial, architecture-changing, security-sensitive, provider/subscription-sensitive, memory-affecting, or confidence is below high.
8. Apply review feedback, record rejected feedback, or ask the user when direction is needed.
9. Mark the task `completed` only when acceptance criteria and review requirements are satisfied.
10. Make an explicit commit decision before another `/next` pass.

## Rules

- The initial product prompt captured in `project.md` remains the source of truth when docs conflict.
- Do not start broad implementation during research or architecture unless explicitly requested.
- Keep controller, protocol adapter, runtime, tunnel, provider, capability, state, memory, evaluation, and input surfaces separate.
- Favor Rust for durable controller/core work unless research shows Python materially reduces risk for a specific subsystem.
- Treat subscription-backed connectors as privileged integrations with explicit security review.
- Do not log secrets, subscription sessions, OAuth tokens, cookies, voice transcripts containing secrets, or provider credentials.
- Do not commit without explicit user confirmation.
- If evidence is weak, record uncertainty instead of guessing.

Start now.
