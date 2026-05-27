# Capo

Capo is a work-in-progress local-first controller for coding-agent sessions.

The intended product shape is a durable Capo server/control plane with clients. The local `capo` CLI is one client for inspecting controller state, sending instructions, dispatching tracked agents, and exporting evidence. Future clients may include a remote CLI, dashboard/app, and voice surface.

Tracked agents are represented through protocol boundaries, with ACP-compatible interaction as the target direction. Project memory starts simple: Capo indexes markdown-backed project/task records into its local state and exposes relevant context to agents through governed tools and context packets.

This repository is still early. The current implementation is a Rust scaffold/prototype with deterministic fake/scripted-agent paths, bounded real local connector proof, and transitional compatibility surfaces. It is not yet a polished end-user product or a full live ACP server.

## Current Status

- Scaffold alignment is complete.
- There is no active workpad selected in `TASKS.md`.
- `capo project memory ...` is the preferred markdown-backed project-memory surface.
- `capo workpad ...` still exists only as compatibility for older local scripts and repo migration.
- Real Codex/Claude execution paths are opt-in and gated; normal repeatable tests use fake/scripted agents.

See:

- [`project.md`](./project.md) for product direction
- [`TASKS.md`](./TASKS.md) for current phase/workpad state
- [`WORKING.md`](./WORKING.md) for the agent workflow
- [`workpads/WORKPADS.md`](./workpads/WORKPADS.md) for workpad load lists
- [`workpads/scaffold/completion-audit.md`](./workpads/scaffold/completion-audit.md) for the latest alignment audit

## Try The CLI

Run commands from the repository root:

```sh
cargo run -p capo-cli -- --help
cargo run -p capo-cli -- init
```

Use a separate state directory while experimenting:

```sh
export CAPO_STATE=.capo-dev/readme-demo
```

### Start A Fake Agent

```sh
cargo run -p capo-cli -- agent register \
  --name demo \
  --adapter fake \
  --runtime fake \
  --state "$CAPO_STATE"

cargo run -p capo-cli -- task send \
  --agent demo \
  --goal "Inspect the project and summarize the current state" \
  --state "$CAPO_STATE"

cargo run -p capo-cli -- dashboard --state "$CAPO_STATE"
cargo run -p capo-cli -- session status --agent demo --state "$CAPO_STATE"
```

### Index Project Memory

```sh
cargo run -p capo-cli -- project memory index \
  --root . \
  --state "$CAPO_STATE"

cargo run -p capo-cli -- project memory next \
  --state "$CAPO_STATE"
```

Start the next indexed source task for the fake agent:

```sh
cargo run -p capo-cli -- project memory start-next \
  --agent demo \
  --state "$CAPO_STATE"
```

### Read Project Memory Through A Governed Wrapper

```sh
mkdir -p .capo-dev/readme-artifacts

cargo run -p capo-cli -- tool run-wrapper \
  --tool project_memory_read \
  --workspace . \
  --artifacts .capo-dev/readme-artifacts \
  --path project.md
```

## Development Checks

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The broad workspace test suite is the normal local verification gate. Real local provider smoke tests are ignored unless explicitly opted in with their environment gates.

## Compatibility Notes

Prefer product-language commands and fields:

- `capo project memory ...`
- `--source-task`
- `--source-path`
- `--source-status`
- `project_memory_read`

Compatibility surfaces still exist:

- `capo workpad ...`
- `--workpad-task`
- `--workpad-path`
- `--workpad-status`
- `workpad_read`

Those are retained for local scripts, historical tests, and migration safety. New examples and new code should use project/task/memory/context language.
