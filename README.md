# Capo

Capo is a work-in-progress local-first controller for coding-agent sessions.

The intended product shape is a durable Capo server/control plane with clients. The local `capo` CLI is one client for inspecting controller state, sending instructions, dispatching tracked agents, and exporting evidence. Future clients may include a remote CLI, dashboard/app, and voice surface.

Tracked agents are represented through protocol boundaries, with ACP-compatible interaction as the target direction. Project memory starts simple: Capo indexes markdown-backed project/task records into its local state and exposes relevant context to agents through governed tools and context packets.

This repository is still early. The current implementation is a Rust scaffold/prototype with deterministic fake/scripted-agent paths, server-owned dispatch, a human control loop, bounded real local Codex proof, and transitional compatibility surfaces. It is not yet a polished end-user product, unattended goal orchestrator, or full live ACP server.

## Current Status

- Scaffold alignment is complete.
- The server/control-plane milestone is complete: `capo server ...` can drive mocked agents, server-native sessions, dispatch plans/gates, deterministic local runs, and Codex-shaped runs through the server boundary.
- Operator control is active and usable: bare `capo` opens `capo control`, defaults to `--planner none`, starts a local loopback server when needed, and can list, attach to, inspect, steer, interrupt, and stop agents through that server.
- `--planner capo` exists as a deterministic, tracked Capo planner mode. It does not call a live LLM; it audits planner choices through the same Capo state.
- Control can start and continue Codex-backed sessions only behind explicit live-provider gates. Normal repeatable tests use fake/scripted agents or mocked Codex output, and Claude live execution is still blocked.
- Result rendering now keeps structured Markdown output readable in the terminal. Durable live-provider raw-output retention remains conservative and artifact-backed.
- `capo project memory ...` is the preferred markdown-backed project-memory surface.
- `capo workpad ...` still exists only as compatibility for older local scripts and repo migration.

See:

- [`project.md`](./project.md) for product direction
- [`TASKS.md`](./TASKS.md) for current phase/workpad state
- [`WORKING.md`](./WORKING.md) for the agent workflow
- [`workpads/WORKPADS.md`](./workpads/WORKPADS.md) for workpad load lists
- [`workpads/operator-control/tasks.md`](./workpads/operator-control/tasks.md) for current control-loop evidence
- [`workpads/scaffold/completion-audit.md`](./workpads/scaffold/completion-audit.md) for the completed scaffold alignment audit

## Try The CLI

Run commands from the repository root:

```sh
cargo run -p capo-cli --bin capo -- --help
cargo run -p capo-cli --bin capo -- init
```

Use a separate state directory while experimenting:

```sh
export CAPO_STATE=.capo-dev/readme-demo
```

### Use The Server

Terminal 1 starts the local Capo server. By default it listens only on loopback at `127.0.0.1:7878`.

```sh
cargo run -p capo-cli --bin capo -- server serve
```

Terminal 2 uses normal `capo server ...` commands. When the default local server is running, these commands talk to it automatically.

```sh
cargo run -p capo-cli --bin capo -- server agent register --name demo

cargo run -p capo-cli --bin capo -- server task send \
  --agent demo \
  --goal "Inspect the project and summarize the current state"

cargo run -p capo-cli --bin capo -- server dashboard
```

### Use The Operator Control Loop

Bare `capo` enters the default control loop. It starts a local loopback server when one is not already running and then sends commands through the server boundary.

```sh
cargo run -p capo-cli --bin capo --
```

This aliases to `capo control --planner none`. The default planner is deterministic: it only runs the commands you type. After `attach`, ordinary text is sent directly to the attached agent; Capo commands such as `status`, `result`, `details`, `tools`, `detach`, and `quit` still behave as commands.

Then type:

```txt
agents
attach demo
Please report current status and wait for the next instruction
result
status
tools
evidence
reviews
details
dashboard
quit
```

The same control loop can be scripted:

```sh
printf '%s\n' \
  'agents' \
  'attach demo' \
  'Please report current status and wait for the next instruction' \
  'result' \
  'status' \
  'tools' \
  'evidence' \
  'reviews' \
  'details' \
  'dashboard' \
  'quit' \
  | cargo run -p capo-cli --bin capo --
```

The deterministic Capo planner mode can map a small set of natural-language operator intents to the same server-backed actions while recording its decisions as a tracked `capo-operator` session:

```sh
printf '%s\n' \
  'what happened?' \
  'what is blocked?' \
  'steer demo to Please summarize the latest state' \
  'recent capo-operator' \
  'quit' \
  | cargo run -p capo-cli --bin capo -- control --planner capo
```

For real Codex from control, start the REPL with both live-provider gates enabled and use `new codex`:

```sh
printf '%s\n' \
  'new codex codex-demo Say CAPO_REAL_CODEX_OK and nothing else' \
  'result' \
  'details' \
  'quit' \
  | CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 \
    cargo run -p capo-cli --bin capo --
```

The real Codex path should render the latest assistant reply when a scanned stdout artifact contains one. Attached text for an existing Codex session also requires the same live-provider gates; otherwise control fails closed instead of pretending Codex output came from a fake adapter. Claude live execution is intentionally still blocked.

### Start A Fake Agent

```sh
cargo run -p capo-cli --bin capo -- agent register \
  --name demo \
  --adapter fake \
  --runtime fake \
  --state "$CAPO_STATE"

cargo run -p capo-cli --bin capo -- task send \
  --agent demo \
  --goal "Inspect the project and summarize the current state" \
  --state "$CAPO_STATE"

cargo run -p capo-cli --bin capo -- dashboard --state "$CAPO_STATE"
cargo run -p capo-cli --bin capo -- session status --agent demo --state "$CAPO_STATE"
```

### Index Project Memory

```sh
cargo run -p capo-cli --bin capo -- project memory index \
  --root . \
  --state "$CAPO_STATE"

cargo run -p capo-cli --bin capo -- project memory next \
  --state "$CAPO_STATE"
```

Start the next indexed source task for the fake agent:

```sh
cargo run -p capo-cli --bin capo -- project memory start-next \
  --agent demo \
  --state "$CAPO_STATE"
```

### Read Project Memory Through A Governed Wrapper

```sh
mkdir -p .capo-dev/readme-artifacts

cargo run -p capo-cli --bin capo -- tool run-wrapper \
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
