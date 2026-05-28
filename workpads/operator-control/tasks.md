# Operator Control Workpad Tasks

## Objective

Create a human operator loop for interacting with running Capo agents through the server boundary.

The first product slice is a no-planner command REPL: it should make it easy to list agents, inspect status, attach to an agent context, send instructions, and jump back out without requiring the operator to remember low-level server/dispatch commands. Later slices can add planner-backed modes such as `codex`, `capo`, or local small-model planners that choose the same tools/actions.

## OC0 - Workpad And Scope

Status: completed on 2026-05-28

Acceptance:

- Create the operator-control workpad and make it the active queue entry.
- Record the initial scope: REPL/client surface, `--planner none` first, planner modes later.
- Keep the REPL as an input surface that composes server commands instead of owning orchestration state.

Evidence:

- `TASKS.md`
- `workpads/WORKPADS.md`
- `AGENTS.md`
- `workpads/operator-control/tasks.md`
- `workpads/operator-control/knowledge.md`
- `workpads/operator-control/references.md`

## OC1 - No-Planner Server REPL

Status: completed on 2026-05-28

Acceptance:

- Add a user-facing command such as `capo repl --planner none` or `capo control --planner none`.
- Support direct commands over the running server boundary:
  - list agents;
  - show dashboard/status;
  - attach or jump into an agent context;
  - send a message/goal to the attached or named agent;
  - detach/back out;
  - help and quit.
- Use existing server commands and query/read-model output; do not add a second orchestration path.
- Render concise human-readable output rather than only raw key-value dumps.
- Work in non-interactive/scripted mode so tests can feed commands through stdin.
- Reject unsupported planners with a clear message and no LLM/provider launch.
- Add tests for scripted REPL interaction with mocked agents and for unsupported planner rejection.
- Manually run the documented commands directly against a running server.

Must not do:

- Do not add an LLM planner in this first slice.
- Do not execute live Codex/Claude provider CLIs from ordinary REPL commands unless existing explicit live-provider gates are used.
- Do not bypass the server/control-plane boundary for mutations.
- Do not make the REPL own persisted state.

Evidence:

- Added `capo control --planner none [--connect ADDR] [--state PATH]`.
- Added server-backed `SteerAgent` / `capo server agent steer --agent NAME --goal GOAL` so control sends mutations through `capo-server`.
- Supported scripted and interactive control commands: `agents`, `dashboard`, `status`, `attach`/`jump`, `send`, `detach`/`back`, `help`, `quit`.
- Added deterministic tests:
  - `cargo test -p capo-server steer`
  - `cargo test -p capo-cli operator_control::tests`
  - `cargo test -p capo-cli --test server_transport control_repl_lists_attaches_and_steers_mock_agent_over_server -- --nocapture`
- Ran full suite: `cargo test`.
- Manual direct run against a live server:
  - `CAPO_STATE=/tmp/capo-control-manual ./target/debug/capo server serve`
  - `CAPO_STATE=/tmp/capo-control-manual ./target/debug/capo server agent register --name demo`
  - `CAPO_STATE=/tmp/capo-control-manual ./target/debug/capo server task send --agent demo --goal "Inspect the project and summarize current state"`
  - `printf '%s\n' 'agents' 'attach demo' 'status' 'send Please report current status and wait for the next instruction' 'dashboard' 'quit' | CAPO_STATE=/tmp/capo-control-manual ./target/debug/capo control --planner none`
  - Observed `Agents (1)`, `attached: demo`, `sent to demo`, `Dashboard`, and `active sessions: 1`.

## OC2 - Planner Boundary Design

Status: completed on 2026-05-28

Acceptance:

- Define planner mode semantics for `none`, `codex`, `capo`, and future local model planners.
- Define the tool surface a planner may use to inspect/steer Capo.
- Define safety boundaries for planner-triggered mutations, approvals, and provider access.
- Decide how planner output is audited and displayed to the human.

Evidence:

- Added `workpads/operator-control/planner-boundary.md`.
- Defined mode semantics for `none`, future `codex`, future `capo`, and local small-model planners such as `gemma`.
- Defined the first read and mutation tool surface that planner modes should lower into.
- Recorded safety rules for explicit planner selection, fail-closed unsupported modes, gated live providers, scoped approvals, and redaction.
- Recorded audit/display requirements and the next code direction: extract a `Planner` boundary while keeping server request execution separate.

## OC3 - Richer Agent Interaction Commands

Status: completed on 2026-05-28

Acceptance:

- Make control discoverable: bare `capo` should enter `capo control`, `--planner` should default to `none`, and control should start a local loopback server when the default server is not running.
- Add commands for recent work, tool activity, evidence, review needs, and interruption/stop.
- Decide what "jump into a running agent" means for mocked, ACP, Codex, and Claude adapters.
- Preserve deterministic mocked-agent tests before adding provider-specific behavior.

Evidence:

- Bare `capo` now routes to `capo control --planner none`; `capo --help` still shows the full command reference.
- `capo control` auto-starts a local loopback server when the configured/default address is free.
- Added control commands:
  - `recent [AGENT]` / `work [AGENT]`;
  - `tools [AGENT]`;
  - `evidence [AGENT]`;
  - `reviews [AGENT]`;
  - `interrupt [--agent AGENT] REASON`;
  - `stop [--agent AGENT] REASON`.
- Added typed server commands for `InterruptAgent` and `StopAgent`.
- Split `operator_control` into planner/parser, executor, renderer, and server-process modules.
- Added deterministic tests:
  - `cargo test -p capo-cli operator_control`
  - `cargo test -p capo-cli --test server_transport control -- --nocapture`
- Manual dogfood/use path:
  - Bare `capo` autostarted a loopback server with `CAPO_SERVER_ADDR=<free-loopback-addr>` and scripted `dashboard`, `quit`; output included `server: ... (started)`, `Dashboard`, `agents: 0`, and `bye`.
  - Against a running server at `127.0.0.1:7878`, registered `demo-a` and `demo-b`, started tasks, then ran bare `capo` with scripted `attach demo-a`, `recent`, `tools`, `evidence`, `reviews`, `interrupt ...`, `attach demo-b`, `stop ...`, `dashboard`, `quit`.
  - Observed recent-work summary, tool counts, evidence refs, review counts, `interrupted demo-a`, `stopped demo-b`, and final dashboard with `active sessions: 0`.
