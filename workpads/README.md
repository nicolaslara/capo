# Workpads

Project-specific working documentation for AI assistants.

1. Read [`../TASKS.md`](../TASKS.md) for the active workpad.
2. Read [`WORKPADS.md`](./WORKPADS.md) for what to load.
3. For architecture/prototype/feature work, read [`architecture/boundaries.md`](./architecture/boundaries.md).
4. For prototype/dogfood work, read [`prototype/spec.md`](./prototype/spec.md).
5. Execute tasks in `workpads/{active}/tasks.md`.

The `/next` command prompt lives in [`../.cursor/commands/next.md`](../.cursor/commands/next.md) and [`../.opencode/commands/next.md`](../.opencode/commands/next.md).

## Structure

```text
workpads/
├── WORKPADS.md
├── research/
├── architecture/
│   └── boundaries.md
├── prototype/
│   └── spec.md
├── features/
└── dogfood/
```

## Standard Files

| File | Purpose |
| --- | --- |
| `knowledge.md` | Decisions, specs, lessons |
| `references.md` | External links and notes |
| `tasks.md` | Task list with acceptance criteria |

Every workpad should state its **Objective** near the top of `tasks.md` and `knowledge.md`. If the workpad has a special spec or boundary file, that file should also state its objective.

## Task States

```text
pending      - Not yet started
in_progress  - Currently working on
completed    - Finished and verified
blocked      - Cannot proceed
```

## Workflow

See [`../WORKING.md`](../WORKING.md).
