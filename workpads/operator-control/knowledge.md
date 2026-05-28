# Operator Control Knowledge

## Objective

Capture decisions and lessons for Capo's human operator REPL/control surface.

## Initial Scope

- The operator-control surface is an input/client boundary, not a controller.
- The first implementation should be `--planner none`: deterministic command handling that composes existing Capo server commands.
- Future planners may be `codex`, `capo`, or local small models, but they should call the same server/control tools a human can call.
- "Jump into" or "attach to" an agent initially means setting a current agent context for subsequent REPL commands. It does not yet mean attaching to a provider-native interactive TTY/session.
- The REPL should favor concise human-readable summaries while leaving detailed evidence available through existing dashboard/status/evidence commands.

## Decisions

- Create a separate `operator-control` workpad rather than reopening `server`. Server ownership is complete; this is a client/input-surface layer.
- Start with CLI REPL because it can be tested deterministically through stdin and does not require TUI/web dependencies.
- Use `capo control` as the first command name. It describes the product role better than a generic REPL while leaving room for later aliases.
- Keep `--planner none` deterministic and command-driven. Unsupported planners fail before opening a provider or connecting to a model.
- Add `SteerAgent` to the server request surface instead of letting control call the older local `capo session redirect` path. This keeps steering auditable at the server boundary.
- `attach`/`jump` currently selects the active agent context for subsequent control commands. It does not attach to a provider-native TTY.
- In a terminal, `capo control` streams prompts/responses. With piped stdin, it runs as a deterministic scripted control loop for tests and docs.
- Future planner modes must lower into the same operator action/tool surface as `none`; they should not gain a separate execution path.
- Planner execution and server request execution should remain separate so a future LLM planner can choose an action without bypassing `capo-server`.
- Bare `capo` should enter the no-planner control loop. `--help` remains the explicit way to show the full command reference.
- `capo control` should auto-start a local loopback server when the configured/default address is free. This makes the product feel like one entrypoint while preserving the server boundary internally.
- Treat this entrypoint behavior as a product requirement, not just a smoke-test convenience: future planner modes should layer on top of `capo` / `capo control` and keep `none` as the default when no planner is configured.

## OC1 Findings

- The minimum useful human loop is small: list agents, attach, inspect status/dashboard, send steering text, detach, help, quit.
- The control client owns only transient UI state (`attached_agent`). Persisted orchestration state remains in `capo-server` and controller/state projections.
- Server steer audit events record `goal_hash` and `raw_goal_policy=not_rendered`; the session redirect event still contains the raw current goal because it is the current mocked session state. A later privacy pass should decide whether live-provider steering needs a redacted current-goal policy.
- Manual use worked with normal commands against `127.0.0.1:7878` and `CAPO_STATE=/tmp/capo-control-manual`.

## OC2 Findings

- `none` is the baseline compatibility mode: deterministic, no model calls, and command output should stay human-readable.
- `codex` and `capo` should be planner modes, not controller modes. They can choose tools/actions, but mutations still go through typed server commands.
- A Capo-native planner mode should itself be tracked as a Capo session so its planning and tool use are inspectable.
- Local small-model planners should fail closed to help/status behavior when confidence is low or output is malformed.
- The next code refactor should extract a planner boundary from `operator_control.rs` before adding any LLM-backed planner.

## OC3 Findings

- "Jump into" means selecting the current Capo agent context for all adapter kinds. For mocked, ACP, Codex, and Claude adapters, the REPL still sends typed Capo server commands; it does not attach to a provider-native TTY.
- Richer read commands can initially use the server dashboard/session summaries: `recent`/`work`, `tools`, `evidence`, and `reviews` render concise status without exposing raw event dumps.
- `interrupt` and `stop` are typed server commands, not compatibility CLI calls. They clear the attached agent context when applied to the attached agent.
- The operator-control module is split into planner/parser, executor, renderer, and server-process concerns so future `codex`/`capo` planners can plug in without bypassing execution.

## OC4 Findings

- `--planner capo` starts as a deterministic mocked planner, not a live model or provider call.
- The first Capo planner agent is the durable `capo-operator` agent/session. This makes planner behavior inspectable through the same dashboard/status/recent-work surfaces as other agents.
- Planner decisions are currently audited by steering `capo-operator` with a redacted summary. This reuses existing server state and avoids adding a second audit channel before the event model needs it.
- Planner-triggered mutations use a safe deterministic policy: natural language may read state, while steering requires the explicit syntax `steer AGENT to MESSAGE`.
- Read commands without an attached agent now aggregate over tracked agents, which makes planner answers like `what is blocked?` useful before the operator attaches to a specific agent.

## OC5 Findings

- Attach was functionally working but visually weak: the selected agent context was not shown in later list output, so it felt like nothing changed.
- `status` and `send` should show recent-work state, not just the compact agent row. The compact row is good for scanning, but it does not answer "what did the agent do?"
- `result` / `state` should be discoverable aliases for recent work because humans naturally ask for the result after sending an instruction.
- Stripping a single surrounding quote pair from one-line `send` input matches normal shell habits without adding a full shell parser.

## OC6 Findings

- Attached mode should feel like talking to the agent directly, closer to an agent-native CLI. Requiring `send` after `attach` makes attachment feel decorative instead of modal.
- The direct-send fallback belongs in the control loop because the planner/parser should keep failing closed on malformed commands; only the attached UI state decides whether unknown free text is safe to forward.
- The fallback must not intercept known Capo command words. For example, malformed `attach` or `status` inputs should still return command errors instead of being sent to the agent.
- The prompt should carry context (`capo[agent]>`) so the operator can see when free text will be routed to an agent.

## OC7 Findings

- A Codex-backed session must not be treated like a fake session. If the attached session has `adapter_kind=codex_exec`, ordinary attached text needs to run through server live-provider dispatch or fail closed.
- Starting Codex from control should compose existing server commands instead of creating another launch path: register agent, start session, live preflight, live run, attach, inspect.
- Keep the existing safety gates visible in the REPL. `new codex ...` and Codex attached text require `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and `CAPO_SERVER_RUN_CODEX_LIVE=1` when control starts.
- Codex live results used to render redacted summary metadata rather than provider text. OC8 added a safe artifact-backed display path for the current control turn while leaving durable summaries conservative.

## OC8 Findings

- Normal control output should be an operator UI, not a transport debugger. Session ids, run ids, dispatch ids, provider flags, hash-only goals, and raw policy names belong behind `details`.
- The durable read model can remain conservative while the current CLI renders a live Codex reply from the scanned stdout artifact after dispatch. This gives the human a usable conversation without storing raw provider content in `latest_summary`.
- `details [AGENT]` is the release valve for debugging and auditability. It keeps the old metadata available without forcing every operator transcript to expose implementation details.
- `dashboard` should scan like a product view: agent count, active count, and readable agent rows. The project id is useful for debug paths later, but it is not the primary first-screen signal for the operator.
- Current Codex `exec --json` emits assistant text as `{"item":{"type":"agent_message","text":"..."}}`; the adapter parser must support that alongside the older `item.role` plus `item.content[]` shape.

## OC9 Findings

- Default output should optimize for repeated use, not first-run explanation. `attach`, `send`, and `new codex` should confirm routing and show the latest reply; `status`/`result` are the explicit fuller read commands.
- Static dispatch is enough for the display boundary in this slice. A generic `AgentRenderer` keeps call sites typed without introducing a renderer registry or boxed dynamic dispatch.
- Terminal history should be in-memory only for now. Persisting operator input under the state root would create a new retention surface for potentially sensitive instructions.
- `rustyline` is a pragmatic REPL dependency for arrows, editing, Ctrl-C, and Ctrl-D behavior. Scripted stdin must stay separate so tests and docs remain deterministic.

## OC10 Findings

- Immediate agent replies are a distinct display mode from `result`/`status`. They should have their own renderer boundary instead of being a free helper.
- `AgentResultRenderer` keeps the concise reply path statically dispatched while allowing future result styles, such as compact, verbose, or structured client renderers.
- Live Codex result rendering still reads dispatch artifacts directly. That should become a typed dispatch-result renderer once Capo has a first-class user-visible result view model for artifact-backed provider turns.

## OC11 Findings

- The odd `workspace context is loaded, and I'm standing by` reply was not generated by Capo's renderer; it was provider output shown by Capo. The current `/tmp/capo-demo` logs cannot recover that prior raw reply because live Codex turns reused the same `stdout.txt` artifact path.
- The durable event log for live provider output currently keeps dispatch metadata, event ids, content hashes, and raw-event hashes. It intentionally does not keep raw assistant text in the `events` payload under the current `bounded_redacted_artifacts`/`not_rendered` policies.
- For operator debugging, Capo needs per-turn artifact retention or a bounded redacted display snapshot keyed by turn. Otherwise `details` can prove a provider run happened, but cannot explain what an earlier overwritten reply said.
- The table rendering bug was in Capo's display layer. Codex emitted the table with newlines in JSONL, but `bounded_display_text` collapsed all whitespace before rendering it.
- Result rendering should preserve structured multiline output such as markdown tables, lists, and code blocks while keeping ordinary short prose concise.
- `pulldown-cmark` is a good first parser because it is small, fast, and event-based. Capo should still keep its own `AgentResultView`/`ResultBlock` boundary so parser and renderer choices remain swappable.
- Future alternatives are likely useful for different renderers: `comrak` for a richer GFM AST and transformations, `markdown-to-ansi` or `termimad` for styled terminal output, `syntect` for code highlighting, and `ratatui`/`tui-markdown` for a full TUI.

## OC12 Findings

- README status had lagged behind the active operator-control implementation. It still framed `capo control --planner none` as the main active slice and did not mention the deterministic `--planner capo` path, attached free-text, `new codex`, `details`, or Markdown-preserving result display.
- The README should keep WIP/product-spine language while showing the most humane current entrypoint: bare `capo` starts the control loop and auto-starts a local loopback server when needed.
- Live Codex usage belongs in README as a gated control-loop path (`new codex ...` with `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and `CAPO_SERVER_RUN_CODEX_LIVE=1`), not as the older low-level dispatch-plan copy/paste flow.

## Open Questions

- Which commands should require explicit confirmation inside planner-backed modes?
- Should `send` eventually support multiline input or an editor handoff for longer operator instructions?
- What should provider-native attach mean later for long-lived ACP/Codex/Claude sessions, and can it be represented as a Capo-controlled stream instead of a raw TTY handoff?
- For OC4, should `--planner capo` create a dedicated operator-assistant agent per control process, reuse a durable project-level Capo agent, or attach to a user-selected planner agent?
- How should `result` retrieve prior live-provider replies after process restart if the artifact path is not local to the current client?
- How should live provider artifacts be named and retained so repeated turns in one session do not overwrite the raw output needed for later debugging?
