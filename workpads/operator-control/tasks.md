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

## OC3a - Discoverable Capo Entrypoint

Status: completed on 2026-05-28

Acceptance:

- Treat bare `capo` as an alias for `capo control`.
- Default the planner to `none` when `--planner` is not set.
- If the configured/default local loopback server is not running, start it before entering control.
- Keep explicit command help discoverable through `capo --help`.
- Preserve the server boundary: autostart should launch `capo server serve`, not create a client-local controller.
- Add this requirement to the operator-control task sequence so future planner modes do not regress the intuitive entrypoint.

Evidence:

- `crates/capo-cli/src/main.rs` routes empty args to `operator_control(&parsed, &[])`, while `--help` still renders the command reference.
- `crates/capo-cli/src/operator_control.rs` defaults missing `--planner` to `none`.
- `crates/capo-cli/src/operator_control/server_process.rs` resolves `--connect` / `CAPO_SERVER_ADDR` / default address, requires loopback, detects whether the port is bound, and starts `capo server serve` when needed.
- `crates/capo-cli/tests/server_transport/basic.rs` covers bare `capo` autostarting control and reporting the started server.

## OC4 - First Capo Planner Mode

Status: completed on 2026-05-28

Acceptance:

- Add `--planner capo` as the first planner-backed mode.
- Make the planner run as, or create, a tracked Capo-managed agent/session so its planning and tool use are inspectable in Capo state.
- Keep planner output lowering into the same `OperatorAction`/server executor path as `none`.
- Start with a deterministic mocked Capo planner before any live provider or local-model calls.
- Allow the planner to answer simple operator intents such as "what happened?", "what is blocked?", and "steer AGENT to ...".
- Require confirmation or a safe deterministic policy before planner-triggered mutations.
- Add tests proving:
  - unsupported planners still fail closed;
  - `--planner capo` does not bypass the server boundary;
  - planner decisions are audited as Capo state;
  - mocked planner interaction can list/status/steer without launching live Codex or Claude.

Evidence:

- Added deterministic `CapoPlanner` mode in `crates/capo-cli/src/operator_control/planner.rs`.
- `--planner capo` maps direct commands plus simple operator intents:
  - `what happened?` -> dashboard overview;
  - `what is blocked?` -> review-needs query;
  - `status of AGENT` -> agent status;
  - `steer AGENT to MESSAGE` -> explicit server-backed steering.
- `crates/capo-cli/src/operator_control.rs` now prepares a tracked `capo-operator` agent/session through the server before `capo` planner use.
- Planner decisions are audited by steering `capo-operator` with a redacted decision summary containing an input hash, action label, target agent, mutation flag, and short summary.
- Planner output still lowers into the same `OperatorAction` executor used by `none`; all mutations use typed server commands.
- Unsupported planners still fail closed.
- Added deterministic tests:
  - `cargo test -p capo-cli operator_control -- --nocapture`;
  - `cargo test -p capo-cli --test server_transport capo_planner_tracks_decisions_as_server_state_and_steers_mock_agent -- --nocapture`;
  - `cargo test -p capo-cli --test server_transport -- --nocapture`.
- Manual direct run:
  - Started `./target/debug/capo server serve --addr 127.0.0.1:0 --max-requests 14`.
  - Registered `demo-planner` and sent `Manual Capo planner check`.
  - Ran `./target/debug/capo control --planner capo --connect <addr>` with `what happened?`, `status of demo-planner`, `steer demo-planner to Please continue and summarize status`, `recent capo-operator`, `quit`.
  - Observed `planner: capo`, `capo-operator` tracked as a running session, `sent to demo-planner`, and `Recent work` for `capo-operator` showing `capo planner decision input_hash=...`.

## OC5 - Attached Context And Result Visibility

Status: completed on 2026-05-28

Acceptance:

- Make attached agent context visible after `attach`, especially in subsequent `agents` / `ls` output.
- Make `status` show enough state to answer "what is this agent doing and what happened last?"
- Make `send` immediately show the resulting latest work summary instead of only repeating the terse agent line.
- Add an obvious command alias for seeing the last result/state after a send.
- Keep all reads and mutations going through the server boundary.
- Preserve scripted test coverage and manually verify the user-reported transcript shape.

Evidence:

- `agents` / `ls` now marks the selected agent with `(attached)`.
- `attach AGENT` and `status [AGENT]` render both the one-line state and the recent-work summary.
- `send MESSAGE` renders `Recent work` immediately after the server-backed `SteerAgent` response.
- Added `state [AGENT]` and `result [AGENT]` aliases for `recent [AGENT]`.
- Quoted one-line sends such as `send "print something"` strip the surrounding quotes before steering.
- Added deterministic coverage in `crates/capo-cli/tests/server_transport/basic.rs` for attached marker visibility and immediate send-result rendering.
- Manual direct run:
  - Started `./target/debug/capo server serve --addr 127.0.0.1:0 --max-requests 10`.
  - Registered `demo`, started `Manual attach/result test`, then ran scripted control with `agents`, `ls`, `attach demo`, `ls`, `status`, `tools`, `send "print something"`, `result`, `quit`.
  - Observed `- demo [running] (attached) ...` after attach and `goal: print something` / `summary: Fake adapter processed goal for demo: print something` after send.

## OC6 - Direct Attached Agent Interaction

Status: completed on 2026-05-28

Acceptance:

- Once attached to an agent, allow ordinary text to be sent directly to that agent without requiring `send`.
- Preserve Capo control commands while attached: `status`, `result`, `tools`, `detach`, `quit`, etc. should still behave as commands.
- Make the interactive prompt show the attached context.
- After `detach`, ordinary unknown text should no longer be sent to the previous agent.
- Verify with a scripted attached-mode transcript.

Evidence:

- `ControlRepl::run_line` now falls back to direct `send` only when an agent is attached and the input does not look like a Capo control command.
- Interactive prompts now render as `capo[AGENT]>` while attached and `capo>` otherwise.
- `help` now states: "When attached, ordinary text is sent directly to the attached agent."
- Updated `crates/capo-cli/tests/server_transport/basic.rs` so attached-mode free text sends to the selected mocked agent.
- Manual direct run:
  - Started `./target/debug/capo server serve --addr 127.0.0.1:0`.
  - Registered `demo`, started `Manual attached chat test`, then ran scripted control with `attach demo`, `print something`, `result`, `detach`, `print after detach`, `quit`.
  - Observed `print something` was sent to `demo` and produced a recent-work summary; after `detach`, `print after detach` returned `error: unknown command`.

## OC7 - Start Codex From Control

Status: completed on 2026-05-28

Acceptance:

- Allow an operator inside `capo control` to start a new Codex-backed agent without leaving the REPL.
- Do not silently route Codex attached-chat messages through the fake `SteerAgent` path.
- Require the existing explicit Codex live-provider opt-ins before launching Codex.
- After starting Codex, attach to the new agent and make `result` / direct attached text use Codex live-provider dispatch.
- Preserve deterministic tests that prove Codex without opt-in fails closed instead of producing fake output.

Evidence:

- Added `new codex AGENT GOAL` / `start codex AGENT GOAL` control commands.
- Starting a Codex agent from control registers the agent if needed, starts a Codex adapter session, runs live preflight, runs live Codex dispatch, attaches to the agent, and renders recent work.
- Attached text for an existing `codex_exec` session now goes through live Codex dispatch when `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and `CAPO_SERVER_RUN_CODEX_LIVE=1` are set.
- Attached text for `codex_exec` without those env vars fails closed with an explicit error and does not render fake adapter output.
- Added deterministic coverage in `crates/capo-cli/tests/server_transport/basic.rs`: `control_repl_refuses_to_fake_codex_attached_chat_without_live_opt_in`.
- Manual no-opt-in run:
  - Registered `codex-demo`, started a `codex` session, attached in control, typed `say hi`.
  - Observed `Codex live execution from control requires...` and no `Fake adapter processed goal...` output.
- Manual real Codex run:
  - Ran control with `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1`.
  - Scripted `new codex codex-repl Say CAPO_REPL_CODEX_OK and nothing else`, `result`, `quit`.
  - Observed `provider_cli_executed: true`, `status: exited`, attached `codex-repl`, and recent work with `adapter codex_exec` summary metadata.

## OC8 - User-Facing Control UI

Status: completed on 2026-05-28

Acceptance:

- Make the default control transcript readable to an operator who does not know Capo internals.
- Hide session ids, run ids, dispatch ids, hash-only goals, provider execution flags, and raw policy details from normal `agents`, `status`, `result`, `send`, and `dashboard` output.
- Preserve a discoverable `details [AGENT]` command for debugging and audit metadata.
- For live Codex turns, render the latest assistant message from the scanned stdout artifact when available, without changing the durable state summary policy.
- Keep deterministic tests for parser/render behavior, mocked control, no-opt-in Codex failure, and the Codex artifact reply renderer.
- Manually run the user-facing CLI path and verify the transcript no longer reads like debug output.

Evidence:

- `agents` now renders compact rows such as `- demo-ui [running] - running (1 tools, 1 memories, 1 evidence)`.
- `status`, `result`, and `send` render conversation-shaped fields: status, goal, reply, and activity.
- Added `details [AGENT]` / `debug [AGENT]` for session ids, run ids, dispatch ids, CLI execution flags, and raw output policy metadata.
- `new codex ...` and attached Codex text now render `Codex finished.` plus `Reply: ...` from the scanned live stdout artifact when a display message exists.
- Added focused deterministic coverage:
  - `cargo test -p capo-adapters codex_exec_agent_message_text_maps_to_assistant_content -- --nocapture`
  - `cargo test -p capo-cli operator_control -- --nocapture`
  - `cargo test -p capo-cli --test server_transport control -- --nocapture`
- Manual direct run:
  - Started `./target/debug/capo server serve --addr 127.0.0.1:17880`.
  - Registered `demo-ui`, sent `Initial UI test`, then ran control with `agents`, `attach demo-ui`, attached free text, `result`, and `details`.
  - Observed the normal transcript showing human-readable status/reply/activity; `details` remained the debug path.
  - Ran real Codex with `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 ./target/debug/capo --state /tmp/capo-ui-live-codex2`, scripted `new codex ui-live Reply with exactly CAPO_UI_LIVE_OK and nothing else`, `quit`.
  - Observed `Codex finished.` and `Reply: CAPO_UI_LIVE_OK` in the control transcript.

## OC9 - Concise Display And Interactive REPL Ergonomics

Status: completed on 2026-05-28

Acceptance:

- Make the default UI less verbose than OC8: avoid repeating full status/goal/activity blocks after every attach/start/send.
- Keep detailed state available through explicit inspection commands rather than default output.
- Represent display rendering as a typed static-dispatch boundary so future views can be added without passing loose render function pointers.
- Add real terminal line editing/history behavior so arrow keys work in interactive `capo control`.
- Do not persist operator input history by default.
- Preserve scripted stdin behavior for deterministic tests.
- Commit and push after verification.

Evidence:

- The control banner now renders as `Capo` plus a short `server started` line only when autostart occurs.
- `agents` rows now omit debug-like status brackets and adapter/session metadata.
- `attach`, `new codex`, and `send` now render concise reply lines such as `demo-ui: ...` instead of repeating the full state block.
- `status` / `result` still render status, goal, reply, and activity; `details` remains the session/run/dispatch metadata path.
- Added `AgentRenderer` plus zero-sized concrete renderers in `operator_control/render.rs`, and `read_agent_or_all` now uses generic static dispatch.
- Added `rustyline` for interactive terminal input; scripted stdin still uses the deterministic non-interactive path.
- Manual mock run showed concise default output for `agents`, `attach`, attached free text, `result`, and `tools`.
- Manual TTY run used `rustyline`: typed `help`, pressed up-arrow, reran `help`, then `quit`; no `control-history.txt` was written under the state root.
- Manual live Codex run with `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1` showed concise output: `ui-live: CAPO_UI_CONCISE_OK` followed by `Attached to ui-live.`
- Full verification passed:
  - `cargo fmt`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test`
  - `git diff --check`
- Subagents:
  - xhigh planner reviewed the approach and recommended static dispatch, no persisted history, and removing session ids from normal tool/evidence/review output.
  - medium executor verified the `rustyline` path and focused tests without additional edits.

## OC10 - Result Renderer Boundary

Status: completed on 2026-05-28

Acceptance:

- Move the immediate agent result/reply display behind a typed renderer abstraction.
- Preserve static dispatch for the result renderer.
- Keep live Codex artifact rendering separate unless/until dispatch-artifact results get their own typed view model.
- Preserve existing concise `attach`, `send`, and non-live `start` output.

Evidence:

- Added `AgentResultRenderer` and `ConciseResultRenderer` in `crates/capo-cli/src/operator_control/render.rs`.
- `ControlRepl` now renders immediate agent results through `render_agent_result<R: AgentResultRenderer>(...)`.
- Existing `RecentWorkRenderer`, `DetailsRenderer`, `ToolActivityRenderer`, `EvidenceRenderer`, and `ReviewNeedsRenderer` remain the inspection/static-dispatch path.

## OC11 - Markdown Reply Rendering And Live Artifact Logs

Status: completed on 2026-05-28

Acceptance:

- Investigate whether the `/tmp/capo-demo` state root explains the odd `workspace context is loaded, and I'm standing by` reply.
- Preserve line breaks for structured replies such as markdown tables in immediate agent result rendering.
- Apply the same structured display behavior to live Codex artifact replies.
- Record the current live artifact retention gap as a follow-up finding.
- Add deterministic tests for markdown table rendering.
- Add a first-pass markdown parser dependency without committing the control surface to one terminal renderer.

Evidence:

- Inspected `/tmp/capo-demo/capo.sqlite` `events` and `adapter_dispatch_executions`; the database recorded dispatch/preflight events, content hashes, and repeated artifact ids, but not the older raw assistant text.
- Searched `/tmp/capo-demo` for `workspace context`, `standing by`, and table text. Only the latest table reply remained in `control-live-artifacts/run-session-ui-demo-8b7e52f4b93ea487/stdout.txt`, with newlines preserved in the Codex JSONL artifact.
- Confirmed `adapter_dispatch_executions` reused `artifact-runtime-run-session-ui-demo-8b7e52f4b93ea487-stdout` and the same `stdout.txt` path across multiple turns, so prior turn raw output was overwritten.
- Added shared display helpers in `crates/capo-cli/src/operator_control/render.rs` to keep concise prose compact while preserving markdown-shaped multiline output.
- Added `pulldown-cmark` as the first parser for `AgentResultView` classification, including markdown and fenced code detection.
- Left code comments naming future renderer/parser alternatives: `comrak`, `markdown-to-ansi`, `termimad`, `syntect`, and `ratatui`/`tui-markdown`.
- Updated live Codex rendering in `crates/capo-cli/src/operator_control.rs` to use the same result-body formatting and structured display helper.
- Added deterministic coverage:
  - `cargo fmt --check`
  - `cargo test -p capo-cli operator_control -- --nocapture`
  - `cargo test -p capo-cli --test server_transport control -- --nocapture`
  - `git diff --check`
