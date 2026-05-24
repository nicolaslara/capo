# Setup A Workpad-Driven Project

This guide explains how to create a project scaffold like Capo: a file-backed workflow where agents can resolve the current phase, pick the next task, record decisions, verify work, and continue without relying on chat history.

Use this when starting a new project that should be operated by coding agents through `/next`-style task execution.

## 1. Capture The Project Prompt

Start from the user's source prompt. Do not over-normalize it away.

Write down:

- Project name and short identity.
- Main objective.
- Desired feature set.
- Known constraints and preferences.
- Technology direction.
- Initial phases.
- Any explicit workflow expectations.
- Any existing projects to imitate.

For Capo, the source prompt defined:

- Name: `Capo`
- Goal: controller and harness for managing coding LLM agents.
- Boundary concerns: input surfaces, controller, tunnel, execution runtime, providers, state, memory.
- Preferred stack: favor Rust, use Python when ecosystem leverage matters.
- Workflow: research, architecture, prototype, features, dogfood.

## 2. Create The File Tree

Recommended starting tree:

```text
.
├── AGENTS.md
├── README.md
├── SETUP_PROJECT.md
├── TASKS.md
├── WORKING.md
├── project.md
├── .cursor/
│   └── commands/
│       └── next.md
├── .opencode/
│   └── commands/
│       └── next.md
└── workpads/
    ├── README.md
    ├── WORKPADS.md
    ├── research/
    │   ├── tasks.md
    │   ├── knowledge.md
    │   └── references.md
    ├── architecture/
    │   ├── tasks.md
    │   ├── knowledge.md
    │   ├── references.md
    │   └── boundaries.md
    ├── prototype/
    │   ├── spec.md
    │   ├── tasks.md
    │   ├── knowledge.md
    │   └── references.md
    ├── features/
    │   ├── tasks.md
    │   ├── knowledge.md
    │   └── references.md
    └── dogfood/
        ├── tasks.md
        ├── knowledge.md
        └── references.md
```

Adjust workpad names to the project. Keep `research`, `architecture`, and one implementation workpad unless there is a strong reason not to.

## 3. Write `project.md`

`project.md` is the durable product charter.

Include:

- `# Project Name`
- `## Goal`
- `## Source Of Truth`
- `## Product Thesis` or equivalent framing.
- `## Desired Features`
- `## Boundary Model`
- `## Stack Direction`
- `## Phases`
- `## Workflow`
- `## Initial References`
- `## Global Backlog`

Rules:

- Preserve the user's original intent.
- Put feature desires in tables or bullets so they are easy to scan.
- Separate durable goals from implementation guesses.
- Record external references with URLs and short notes.
- If a claim depends on current upstream behavior, include the observation date.

## 4. Write `TASKS.md`

`TASKS.md` is the user-editable phase queue. Agents use it to resolve the active workpad.

Suggested shape:

```markdown
# Task Queue

## Active Now

- [ ] research
- [ ] architecture
- [ ] prototype
- [ ] features
- [ ] dogfood

## Notes

- The first unchecked item is active unless this section explicitly overrides it.
- Do not skip gates unless this file says a spike is authorized.
```

Rules:

- Keep this file short.
- Do not bury task details here; task details belong in `workpads/{name}/tasks.md`.
- Use Notes for temporary overrides.

## 5. Write `AGENTS.md`

`AGENTS.md` is the orchestration brain for future agents.

Include:

- Repository purpose.
- Source-of-truth file table.
- How to resolve the active workpad.
- Current phase.
- Mandatory workflow.
- Git rules.
- Research rules.
- Safety boundaries.
- Verification expectations.

Minimum active-workpad algorithm:

```text
1. Read TASKS.md.
2. The first unchecked workpad is active unless Notes override it.
3. Confirm the workpad in workpads/WORKPADS.md.
4. Load that workpad's tasks, knowledge, and references.
5. Check gates before doing work.
```

Rules:

- Be explicit about what agents may and may not do.
- State whether commits require user confirmation.
- State how secrets, credentials, private data, and generated artifacts are handled.
- Put project-specific safety constraints here, not only in task files.

## 6. Write `WORKING.md`

`WORKING.md` defines how work gets done.

Include:

- Purpose.
- General agent expectations.
- Workaround policy.
- Core `/next` loop.
- Verification table.
- Workpad gates.
- Confidence assessment.
- Review-subagent policy.
- How to act on review feedback.
- Documentation expectations.
- Phase focus.
- Dependency policy.
- Research vs implementation rules.

Core loop template:

```text
1. Read TASKS.md and resolve the active workpad.
2. Load AGENTS.md, project.md, WORKING.md, and workpads/WORKPADS.md.
3. Load the active workpad's tasks.md, knowledge.md, and references.md.
4. Load extra context files required by that workpad.
5. Select a pending task by dependencies, risk, and testability.
6. Mark it in_progress.
7. Complete acceptance criteria with the smallest correct change.
8. Verify.
9. Record findings and decisions.
10. Use review subagents when the work is substantial or confidence is not high.
11. Mark completed only after evidence and review requirements are satisfied.
12. Make an explicit commit decision before another /next pass.
```

Rules:

- Every task needs evidence.
- Workarounds must be disclosed and tracked.
- Review is required for architecture-changing, security-sensitive, or low-confidence work.

## 7. Write `workpads/WORKPADS.md`

`workpads/WORKPADS.md` is the workpad map.

For each workpad include:

- Status.
- Objective.
- Files to load.
- Quick navigation.
- Rules.
- Prerequisites or gates.

Example:

````markdown
## architecture

**Prerequisites:** Research gate passed, unless TASKS.md authorizes a spike.

**Objective:** Convert research into durable boundaries, contracts, data model, security model, and prototype plan.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/architecture/tasks.md
workpads/architecture/knowledge.md
workpads/architecture/references.md
workpads/architecture/boundaries.md
workpads/research/knowledge.md
```

**Rules:**

- Define interfaces before binding to implementations.
- Record user-sensitive decisions explicitly.
````

Important: every workpad needs an objective here and in its own `tasks.md`.

## 8. Write `workpads/README.md`

This is the human overview of the workpad system.

Include:

- What workpads are.
- Standard files.
- Task state vocabulary.
- How `/next` resolves work.
- Where gates are recorded.

Suggested task states:

```text
pending
in_progress
blocked
completed
deferred
```

Use plain ASCII states unless the project has already standardized on icons.

## 9. Create Each Workpad

Each workpad should contain at least:

- `tasks.md`
- `knowledge.md`
- `references.md`

Use extra files for important stable concepts:

- `architecture/boundaries.md`
- `prototype/spec.md`
- `desktop/branch-integration.md`
- `feature-catalog.md`

### `tasks.md`

Include:

- Objective.
- Gate status.
- Task list.
- Acceptance criteria per task.
- Evidence requirement per task.

Template:

```markdown
# Research Tasks

## Objective

Turn the project prompt into sourced recommendations for protocol, stack, prior art, state, memory, security, and prototype direction.

## Gate

Research gate passes when knowledge.md records enough findings to choose the prototype direction.

## Tasks

### R0 - Capture source prompt

**Status:** completed

**Acceptance criteria:**

- project.md preserves the user's core goal and constraints.
- Desired features are listed.
- Initial phases are defined.

**Evidence:**

- Links to changed files or validation command output.
```

### `knowledge.md`

Include:

- Objective.
- Gate status.
- Decisions.
- Findings.
- Open questions.
- Review notes.
- Date-sensitive observations.

Template:

```markdown
# Research Knowledge

## Objective

Record sourced findings and recommendations that unblock architecture.

## Gate Status

Not passed.

## Decisions

- None yet.

## Findings

- None yet.

## Open Questions

- Which stack best supports the prototype?
```

### `references.md`

Include:

- Objective.
- Primary sources.
- Local references.
- License notes.
- Date observed.
- Open follow-ups.

Template:

```markdown
# Research References

## Objective

Track source material used by this workpad.

| Resource | URL or path | Date observed | Notes |
| --- | --- | --- | --- |
| Example | https://example.com | 2026-05-24 | Primary docs |
```

## 10. Define Workpad Objectives

Objectives prevent vague task execution.

Good objective:

```text
Build the smallest e2e product that can register an agent, send work, inspect progress, interrupt execution, persist state, and record evidence.
```

Weak objective:

```text
Work on prototype stuff.
```

Objective checklist:

- Names the outcome.
- Names the boundary of the workpad.
- Says what decision or artifact it should produce.
- Is specific enough for a new agent to choose tasks.
- Does not include unrelated future work.

Put the objective in:

- `workpads/WORKPADS.md`
- `workpads/{name}/tasks.md`
- `workpads/{name}/knowledge.md`
- `workpads/{name}/references.md` when useful.
- Any major extra file such as `spec.md` or `boundaries.md`.

## 11. Define Gates

Gates decide when the project may move to the next phase.

Common gates:

- Research gate: enough sourced findings to choose direction.
- Architecture gate: boundaries, state model, security model, and prototype plan are defined.
- Prototype gate: a working smoke proves the core loop.
- Dogfood gate: the project can safely use its own tool without losing state or review quality.

Gate template:

```markdown
## Gate

This gate passes when:

- Required decision 1 is recorded in knowledge.md.
- Required artifact 2 exists.
- Verification command or manual smoke has passed.
- Review findings are resolved or explicitly accepted.
```

Rules:

- Gates live in `WORKING.md` and in the relevant `knowledge.md`.
- Do not mark a gate passed without evidence.
- If a gate is bypassed for a spike, record that override in `TASKS.md`.

## 12. Add `/next` Commands

Add both command files when relevant:

- `.cursor/commands/next.md`
- `.opencode/commands/next.md`

They can be identical.

Command template:

```markdown
# Next: Do Next Task

Follow the project workpads methodology to complete the next task.

## Step 1: Read State Files

Read these files first:

1. `TASKS.md`
2. `AGENTS.md`
3. `project.md`
4. `WORKING.md`
5. `workpads/WORKPADS.md`
6. `workpads/{active-workpad}/tasks.md`
7. `workpads/{active-workpad}/knowledge.md`
8. `workpads/{active-workpad}/references.md`
9. Any extra files listed for the active workpad.

## Step 2: Resolve Active Workpad

- The active workpad is the first unchecked item in `TASKS.md`, unless Notes override it.
- Confirm the active workpad's objective in `workpads/WORKPADS.md`.
- Do not skip gates just because a later task looks more concrete.

## Step 3: Gate Check

- If the required gate is not passed, stop unless `TASKS.md` explicitly authorizes a spike.

## Step 4: Select A Task

Choose a pending or unblocked task based on dependencies, current state, risk, and testability.

## Step 5: Execute

1. Mark the task `in_progress`.
2. Complete acceptance criteria with the smallest correct change.
3. Update `references.md`.
4. Update `knowledge.md`.
5. Update `tasks.md` with follow-ups.
6. Assess confidence.
7. Spawn focused review subagents when required.
8. Apply or record review feedback.
9. Mark completed only when acceptance criteria and review requirements are satisfied.
10. Make an explicit commit decision before another `/next` pass.

## Rules

- The project prompt captured in `project.md` remains the source of truth when docs conflict.
- Do not start broad implementation during research or architecture unless explicitly requested.
- Do not log secrets or credentials.
- Do not commit without explicit user confirmation.

Start now.
```

## 13. Add Ignore Rules

Create `.gitignore` for predictable local artifacts.

Typical entries:

```gitignore
# Research scratch
.research/
tmp/

# Runtime state
.state/
.capo/

# Secrets
.env
.env.*
!.env.example

# Rust
target/
Cargo.lock

# Python
.venv/
__pycache__/
.pytest_cache/
```

Adjust `Cargo.lock` policy by project type. Applications usually commit it; libraries often do not.

## 14. Validate The Scaffold

Run these checks before committing:

```bash
find . -maxdepth 3 -type f | sort
rg -n "Objective" workpads
rg -n "zodl|aget|template|TODO" .
git status --short --branch
```

Manual checklist:

- `project.md` captures the user prompt.
- `TASKS.md` has an ordered active queue.
- `AGENTS.md` explains how agents resolve work.
- `WORKING.md` defines core loop, gates, verification, and review.
- `workpads/WORKPADS.md` lists every workpad with objective and load list.
- Every workpad has `tasks.md`, `knowledge.md`, and `references.md`.
- Every workpad has an objective.
- `/next` exists in the command directories the team uses.
- Gates prevent premature implementation.
- Safety rules cover secrets and risky integrations.
- Validation finds no stale copied project names.

## 15. Make The Initial Commit

Before committing, inspect the tree:

```bash
git status --short --branch
git diff --stat
```

Then commit:

```bash
git add -A
git commit -m "Initial workpad scaffold"
```

If the repository has commit-message hooks, follow the local convention.

## Common Mistakes

- Missing objectives in workpads.
- Putting all tasks in `TASKS.md` instead of per-workpad `tasks.md`.
- Letting `/next` choose tasks without reading gates.
- Marking tasks complete before updating `knowledge.md`.
- Recording research claims without URLs, dates, or license notes.
- Mixing product decisions, research findings, and task state in one file.
- Copying stale project names from the template project.
- Starting implementation before the research or architecture gate passes.
- Treating subscription sessions like ordinary API keys.
- Forgetting to define verification before coding starts.
- Making workpads so broad that no task can be completed in one focused pass.
- Relying on chat context instead of updating files.

## Recommended Starting Sequence

1. Create the scaffold.
2. Fill `project.md` from the source prompt.
3. Define phase workpads and gates.
4. Add `/next`.
5. Validate for missing objectives and stale names.
6. Commit the scaffold.
7. Start with research task R0 or R1.
8. Use research findings to sharpen architecture before implementing the durable prototype.
