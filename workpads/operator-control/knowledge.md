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

## Open Questions

- What should "jump into a running Codex/Claude agent" mean once provider-native sessions are long-lived rather than one-shot dispatches?
- Which commands should require explicit confirmation inside planner-backed modes?
- Should `send` eventually support multiline input or an editor handoff for longer operator instructions?
