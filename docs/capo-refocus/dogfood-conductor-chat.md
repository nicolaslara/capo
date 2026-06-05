# Dogfood: conductor chat in a browser

Drive ONE conductor turn per message from a static chat page served by
`capo-web`. The user types a message; capo-web issues `RunConductorTurnLocal`
against a long-lived conductor session, forwarding the URL of capo's in-process
HTTP MCP server (hosted inside capo-web on a loopback ephemeral port). The
conductor uses capo's MCP tools (`start_agent`, `list_agents`, `review_agent`,
`steer_agent`, `set_mode`) to manage workers; the reply comes back over
`POST /api/chat`.

## What capo-web wires (Layer 1)

- Hosts capo's stateless HTTP MCP server (`acp_mcp_router`) on `127.0.0.1:0`,
  bearer-authed, for the process lifetime.
- Registers + starts ONE long-lived conductor ACP session
  (`session-conductor-web`) at boot, reused across every message.
- `POST /api/chat {message, mode?, agent_id?}` drives one conductor turn
  (forwarding the hosted MCP url + bearer) and returns:
  ```json
  { "ok": true, "sessionId": "...", "turnId": "...",
    "reply": "...", "stopReason": "end_turn",
    "toolCalls": [ { "name": "start_agent", "isError": false, "arguments": {...} } ],
    "mode": { "scope": "all", "agentId": null } }
  ```

### Reply text is a LABEL, not verbatim prose

capo content-hashes raw provider output (its redaction floor) and never
re-persists the verbatim assistant text. The thread item for an
`agent_message_chunk` carries a normalized one-line label (e.g.
`item_delta (streaming)`), so `reply` surfaces the conductor's output ITEMS read
back from the committed thread, not the literal words the model emitted. The
tool calls the conductor made are surfaced verbatim in `toolCalls` (from the MCP
server's invocation log). Reading back verbatim reply prose would require
relaxing capo's raw-output policy and is out of scope for this slice.

## The one/all toggle (Layer 2)

The chat page has an `All agents` / `One agent` toggle. The scope is owned by
capo-web (per-conductor `ChatMode`), sent with each message as `mode` (+
`agent_id` when `one`), and injected into the conductor's goal each turn:

- `all` -> the conductor may start/review/steer any worker.
- `one`  -> the goal instructs the conductor to steer/review ONLY `agent_id`.

The MCP `set_mode` tool remains available to the conductor too; this toggle is
the capo-web-owned, per-turn projection of the same scope.

## Run it (live conductor in a browser)

```sh
export CARGO_TARGET_DIR=/Users/nicolas/devel/capo/target
# Live gates: opt in to the real nested claude-code-acp bridge.
export CAPO_WEB_LIVE_ACP=1
export CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1
export CAPO_SERVER_RUN_ACP_LIVE=1
cargo run -p capo-web
# open http://127.0.0.1:4177/chat
```

Then type, e.g.: `Start an agent to create HELLO.txt containing capo-works in
the project, then tell me it's done.` The conductor delegates via `start_agent`;
the worker writes the file into `<state_root>/acp/workspace` (git-inited at boot;
default state root `.capo-dev`).

### Environment requirements (live)

- `HOME` must be set and contain `~/.claude` subscription credentials.
- `ANTHROPIC_API_KEY` must NOT be set (the runtime scrubs it and the nested
  `CLAUDECODE`/`CLAUDE_CODE_*` guards via `env_clear()` + allowlist).
- Live latency is HIGH: each chat message spawns the conductor `npx`
  `@zed-industries/claude-code-acp` session, which dials capo's MCP endpoint and
  may itself drive a nested worker `npx` session. Expect tens of seconds per
  turn; there is no axum-side timeout on `/api/chat`.

### Offline / deterministic dev

Point the conductor drive at a stub instead of the live bridge:

```sh
export CAPO_WEB_ACP_PROGRAM=/path/to/acp-stub.sh CAPO_WEB_ACP_ARGV=""
# leave CAPO_WEB_LIVE_ACP unset
```

(The deterministic backend round-trip is covered by
`chat_endpoint_drives_a_conductor_turn_and_returns_reply` in
`crates/capo-web/src/main.rs`, which uses a `/bin/sh` stub conductor and asserts
the turn drives + the reply reads back -- NO live bridge.)

## Config knobs

| Env var                              | Default                                   | Effect |
| ------------------------------------ | ----------------------------------------- | ------ |
| `CAPO_WEB_ADDR`                      | `127.0.0.1:4177`                          | bind addr |
| `CAPO_STATE_ROOT`                    | `.capo-dev`                               | state root (+ `<root>/acp/workspace`) |
| `CAPO_WEB_LIVE_ACP`                  | unset                                      | `1` opts into the live conductor turn |
| `CAPO_WEB_ACP_PROGRAM`               | `npx`                                      | conductor ACP program |
| `CAPO_WEB_ACP_ARGV`                  | `-y @zed-industries/claude-code-acp`       | conductor ACP args (whitespace-split) |
| `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`| unset                                      | live ACP env gate (required for live) |
| `CAPO_SERVER_RUN_ACP_LIVE`           | unset                                      | live ACP env gate (required for live) |

## Status

- Layer 1 (backend + deterministic test) and Layer 2 (static page + this doc)
  are implemented and green. See `chat_endpoint_drives_a_conductor_turn_and_returns_reply`.
- The live browser round-trip is exercised ONLY via the manual steps above; it
  is not covered by an automated green (live runs are slow and gated). The
  optional live smoke (`#[ignore]` + `CAPO_E2E_LIVE_ACP`) is not included in
  this slice.
