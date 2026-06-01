# CLAUDE.md

Claude should use `AGENTS.md` as the main entrypoint for this repository.

## Required Startup

1. Read `AGENTS.md`.
2. Resolve the active workpad from `TASKS.md`.
3. Load the required files listed in `AGENTS.md` and `workpads/WORKPADS.md`.
4. Follow the mandatory workflow, git rules, safety boundary, and verification
   rules from `AGENTS.md`.

## Source Of Truth

- `AGENTS.md` is the orchestration brain and primary instruction surface.
- `TASKS.md` determines the active workpad.
- `WORKING.md` defines the execution loop and evidence expectations.
- `workpads/WORKPADS.md` defines per-workpad context.
- Active workpad files define the task acceptance criteria and evidence.

Do not invent a separate Claude-specific workflow. If this file conflicts with
`AGENTS.md`, follow `AGENTS.md` and update this file later only if needed.
