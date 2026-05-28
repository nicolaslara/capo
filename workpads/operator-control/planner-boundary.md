# Operator Control Planner Boundary

## Objective

Define how `capo control --planner none|codex|capo|...` can evolve from a deterministic command loop into an agent-assisted operator without moving orchestration state out of the Capo server.

## Boundary

`capo control` is an input surface. It may parse text, ask a planner to choose actions, and render results for a human, but it must not own durable orchestration state.

Durable state belongs to:

- `capo-server` for typed command handling, transport, request identity, and audit events.
- `capo-controller` for session, task, run, tool, memory, evidence, and recovery projections.
- `capo-state` for the event log and read models.

The control client may keep only transient UI state, such as the currently attached agent and short-lived planner scratch output.

## Planner Modes

### `none`

`none` is deterministic and must not call an LLM or provider CLI.

Input is parsed into an `OperatorAction` such as:

- list agents;
- show dashboard;
- show status;
- attach or detach agent context;
- send/steer an agent;
- help;
- quit.

This is the baseline compatibility contract. Every future planner mode should lower its choice to the same action family before touching the server.

### `codex`

`codex` is a future planner mode where Codex can decide which Capo control tools to call.

Constraints:

- It may inspect Capo state through read-only tools by default.
- It must use explicit server commands for mutations.
- It must not run live provider dispatch, shell tools, or subscription-backed sessions unless the existing Capo gates allow that action.
- It should produce a short explanation before any mutation that needs human approval.

### `capo`

`capo` is a future planner mode where a Capo-managed agent acts as the operator assistant.

Constraints:

- It should run as a tracked Capo session, so its own planning, tool calls, approvals, and outcomes are visible in Capo state.
- It may delegate complex reasoning to stronger models through explicit provider connectors, not through hidden client-local calls.
- It should be able to summarize what other agents are doing, propose steering instructions, and execute approved steering actions.

### Local Planner Modes

Future local modes, such as `gemma`, should be treated as planner implementations behind the same `Planner` boundary.

Constraints:

- They may choose actions, not bypass server commands.
- They should be optimized for common operator intents: status, recent work, stalled agents, next instruction, and risk summary.
- They should fail closed to `none`-style command help when confidence is low or output is malformed.

## Tool Surface

Planner modes should use a small typed tool surface that mirrors server commands and read models.

Read tools:

- `list_agents`
- `agent_status`
- `dashboard`
- `recent_work`
- `tool_activity`
- `evidence_summary`
- `review_needs`

Mutation tools:

- `attach_agent_context`
- `detach_agent_context`
- `steer_agent`
- `interrupt_agent`
- `stop_agent`
- `start_session`

Mutation tools must call `capo-server` commands. If a needed action does not have a typed server command yet, add the server command first instead of letting the planner call controller internals or compatibility CLI commands.

## Safety Rules

- Planner mode selection is explicit with `--planner`.
- Unsupported planners fail before connecting to a model or provider.
- Planner-backed mutations should have policy metadata: planner mode, actor, selected tool, target agent/session/run, confidence if available, and approval state.
- Destructive or high-impact actions require confirmation unless the user has granted a scoped durable permission.
- Live provider execution remains behind existing opt-in gates.
- Raw prompts, transcripts, provider outputs, and subscription/session material must follow the same redaction and bounded-evidence policy as server dispatch.

## Audit And Display

Every planner action should be explainable to the human and auditable in Capo state.

For read actions, display concise summaries and avoid flooding the operator with raw event dumps.

For mutations, display:

- what changed;
- which agent/session was targeted;
- whether the action was executed, queued for approval, rejected, or failed;
- where detailed evidence can be inspected.

For planner-backed actions, record:

- planner mode;
- planner action id;
- selected operator action/tool;
- target;
- approval decision if any;
- redacted rationale or rationale hash;
- server request id produced by the mutation.

## Implementation Direction

The next code step should introduce an explicit internal `Planner` trait or enum-owned dispatcher in `crates/capo-cli/src/operator_control.rs`.

The current `none` parser can become `NonePlanner`. Future planners should return the same `OperatorAction` type or a small superset with approval metadata. Server request execution should stay in a separate executor so planner implementations cannot bypass it.
