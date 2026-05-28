# Capo

Capo is a work-in-progress local-first controller for coding-agent sessions.

The intended product shape is a durable Capo server/control plane with clients. The local `capo` CLI is one client for inspecting controller state, sending instructions, dispatching tracked agents, and exporting evidence. Future clients may include a remote CLI, dashboard/app, and voice surface.

Tracked agents are represented through protocol boundaries, with ACP-compatible interaction as the target direction. Project memory starts simple: Capo indexes markdown-backed project/task records into its local state and exposes relevant context to agents through governed tools and context packets.

This repository is still early. The current implementation is a Rust scaffold/prototype with deterministic fake/scripted-agent paths, bounded real local connector proof, and transitional compatibility surfaces. It is not yet a polished end-user product or a full live ACP server.

## Current Status

- Scaffold alignment is complete.
- The server/control-plane milestone is complete: `capo server ...` can drive mocked agents and Codex-shaped runs through the server boundary.
- Operator control is active: `capo control --planner none` can list, attach to, inspect, and steer a running mocked agent through a running Capo server.
- `capo project memory ...` is the preferred markdown-backed project-memory surface.
- `capo workpad ...` still exists only as compatibility for older local scripts and repo migration.
- Real Codex execution is opt-in and gated; normal repeatable tests use fake/scripted agents or mocked Codex output. Claude live execution is still blocked.

See:

- [`project.md`](./project.md) for product direction
- [`TASKS.md`](./TASKS.md) for current phase/workpad state
- [`WORKING.md`](./WORKING.md) for the agent workflow
- [`workpads/WORKPADS.md`](./workpads/WORKPADS.md) for workpad load lists
- [`workpads/scaffold/completion-audit.md`](./workpads/scaffold/completion-audit.md) for the latest alignment audit

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

With the server still running from Terminal 1, Terminal 2 can enter a simple command loop. The first version has no LLM planner; it only runs the commands you type against the server.

```sh
cargo run -p capo-cli --bin capo --
```

Bare `capo` aliases to `capo control --planner none`. If the default loopback server is not running, control starts it for the current command.

Then type:

```txt
agents
attach demo
status
recent
tools
evidence
reviews
send Please report current status and wait for the next instruction
dashboard
quit
```

The same control loop can be scripted:

```sh
printf '%s\n' \
  'agents' \
  'attach demo' \
  'status' \
  'recent' \
  'tools' \
  'evidence' \
  'reviews' \
  'send Please report current status and wait for the next instruction' \
  'dashboard' \
  'quit' \
  | cargo run -p capo-cli --bin capo --
```

For real Codex, start a Codex-backed session and preflight the live provider gate:

```sh
cargo run -p capo-cli --bin capo -- server agent register --name codex-demo

cargo run -p capo-cli --bin capo -- server session start \
  --agent codex-demo \
  --adapter codex \
  --goal "Say CAPO_REAL_CODEX_OK and nothing else" \
  --session codex-demo-session \
  --run codex-demo-run

CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 cargo run -p capo-cli --bin capo -- server dispatch live-preflight \
  --agent codex-demo \
  --adapter codex \
  --goal "Say CAPO_REAL_CODEX_OK and nothing else" \
  --session codex-demo-session \
  --run codex-demo-run \
  --turn codex-demo-turn
```

Copy the printed `dispatch_plan_id`, then run Codex explicitly:

```sh
CAPO_SERVER_RUN_CODEX_LIVE=1 cargo run -p capo-cli --bin capo -- server dispatch live-run-local \
  --dispatch-plan DISPATCH_PLAN_ID_FROM_PREFLIGHT \
  --goal "Say CAPO_REAL_CODEX_OK and nothing else"
```

The real Codex run should report `provider_cli_executed=true`. Claude live execution is intentionally still blocked.

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
