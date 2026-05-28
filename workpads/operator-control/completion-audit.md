# Operator Control Completion Audit - 2026-05-28

## Objective Audited

Create a human operator loop for interacting with running Capo agents through
the server boundary.

The first product slice is a no-planner command REPL that makes it easy to list
agents, inspect status, attach to an agent context, send instructions, and jump
back out without remembering low-level server/dispatch commands. Planner-backed
modes may choose the same tools/actions later.

## Current Verdict

Complete.

Operator-control is stable enough to close and hand off to
`goal-orchestration`. The remaining questions are future scope:
provider-native attach semantics, live-provider artifact retention,
multiline/editor input, richer renderer backends, and deeper goal/story
orchestration.

## Requirement Evidence

| Requirement | Evidence | Status |
| --- | --- | --- |
| Human REPL/control loop exists | `capo` aliases to `capo control`; `capo control` supports interactive and scripted stdin paths in `crates/capo-cli/src/operator_control.rs`. | Complete |
| Uses server boundary for mutations | Control sends typed `ServerCommand` requests via `send_tcp`; agent steering, start, interrupt, and stop use server commands. | Complete |
| Default no-planner command mode | Missing `--planner` defaults to `none`; bare `capo` enters control. | Complete |
| Server discovery/autostart | `server_process.rs` resolves loopback address and starts `capo server serve` when needed. | Complete |
| List/status/dashboard/attach/send/detach/help/quit | Parser and tests cover `agents`, `dashboard`, `status`, `attach`, `send`, `detach`, `help`, and `quit`. | Complete |
| Attached context feels conversational | Attached prompts show `capo[AGENT]>`; unknown non-command text sends to the attached agent; detach disables that routing. | Complete |
| Rich inspection commands | `recent`/`result`, `details`, `tools`, `evidence`, `reviews`, `interrupt`, and `stop` are implemented and tested. | Complete |
| Real Codex can be started from control | `new codex AGENT GOAL` and attached Codex text go through live-provider dispatch only with explicit gates. | Complete with gated live-provider policy |
| Fake Codex output is not silently substituted | Control refuses attached Codex chat without live opt-in instead of using the fake steer path. | Complete |
| User-facing rendering is readable | Default rows hide debug ids; `details` keeps audit metadata; Markdown tables/code preserve structure. | Complete |
| Result rendering has a typed boundary | `AgentResultRenderer`, `ConciseResultRenderer`, `AgentResultView`, and `ResultBlock` keep static-dispatch rendering swappable. | Complete |
| Planner-backed path exists | `--planner capo` creates/reuses tracked `capo-operator`, uses Codex for free-form operator input, validates JSON action output, executes only supported server-backed actions, and audits decisions. | Complete as Codex-backed first slice |
| Unknown control flags fail loudly | `reject_unknown_control_flags` rejects typos such as `--planer` before planner defaulting. | Complete |
| README matches current behavior | README documents WIP status, server/control usage, operator-agent mode, live Codex gates, Markdown rendering, and project-memory guidance. | Complete |
| Workpad evidence is recorded | OC0-OC14 are marked complete with evidence in `tasks.md`; findings are in `knowledge.md`. | Complete |

## Verification Commands

Required for closure:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
git diff --check
```

Focused control checks also used during the final pass:

```sh
cargo test -p capo-cli operator_control -- --nocapture
cargo test -p capo-cli --test server_transport control -- --nocapture
```

Manual live planner proof:

```sh
cargo build -p capo-cli
printf '%s\n' "what's up?" 'quit' \
  | ./target/debug/capo control --planner capo --state /tmp/capo-planner-live-manual
```

Observed output included:

```text
Capo
server started
Type `help` for commands.
Dashboard
agents: 1
active: 0
- capo-operator - finished (1 evidence)
bye
```

Artifact proof:

```sh
rg -n "action|dashboard|provider_cli_executed|item.completed" \
  /tmp/capo-planner-live-manual -S
```

The retained Codex stdout artifact included a validated planner action:
`{"action":"dashboard","summary":"Show the dashboard overview."}`.

## Remaining Future Work

- Live provider artifacts should be retained per turn or store bounded redacted
  display snapshots keyed by turn.
- Provider-native attach for ACP/Codex/Claude needs a stream/session model
  rather than raw TTY handoff.
- Faster/local planner backends such as Gemma can replace Codex behind
  `CAPO_CONTROL_PLANNER_PROVIDER` once a local model runner is selected.
- Richer renderers can plug in later: Comrak, ANSI terminal renderers, Syntect,
  or Ratatui/TUI views.
- `goal-orchestration` owns durable goals, reporting, evidence ledgers,
  continuation scheduling, validation, and historical reports.
- `dashboard-webclient` owns browser UI design, implementation, and screenshot
  iteration after the controller/query surfaces are ready.
