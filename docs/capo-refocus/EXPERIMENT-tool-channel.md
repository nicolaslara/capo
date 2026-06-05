# EXPERIMENT: How to expose "capo tools" to a claude-code-acp session

Date: 2026-06-05. Time-boxed throwaway experiment. No capo source modified, no git touched.
All artifacts built in a `mktemp -d` sandbox. This doc is the only output.

## TL;DR

All three channels were empirically validated end-to-end (the model actually invoked
the tools in each). The decisive architectural finding for "option 3":

- **claude-code-acp's `session/new` DOES accept a client-forwarded `mcpServers` array, and
  it supports `http` and `sse` transports (not just stdio).** It advertises
  `mcpCapabilities: { http: true, sse: true }` at `initialize`.
- BUT client-forwarded http/sse/stdio MCP servers are handed straight to the Claude Agent
  SDK subprocess, which connects to them **directly** — the MCP tool traffic does **NOT**
  route back through the ACP client. So capo "intercepts" those calls only by virtue of
  *hosting* the endpoint (e.g. a localhost HTTP server in capo's own process), not because
  ACP tunnels them.
- There is a separate, stronger in-process mechanism that claude-code-acp itself uses: a
  `type: "sdk"` MCP server with a live in-process `McpServer` **instance**, plus a
  `canUseTool` permission callback that fires for **every** tool call and round-trips to
  the ACP client. These are how the built-in ACP tools (Read/Edit/Bash) call *back into the
  client process*. They are SDK/agent-internal, **not** exposed to ACP-forwarded servers.

Net: capo CAN achieve "agent has the tools in its list + capo's own process services them"
without a separate binary, by hosting an **in-process localhost HTTP (or SSE) MCP server**
and forwarding its URL via `session/new`'s `mcpServers`. This is in-process in the sense
that *capo hosts it in-process*; the transport hop is a localhost socket, and the routing
is agent→localhost, not agent→ACP-client→capo.

## Environment

| Item | Value |
|------|-------|
| `claude --version` | 2.1.165 (Claude Code) |
| Headless subscription | WORKS — `claude -p "say hi"` returned `Hi!`, exit 0, no login prompt |
| node | v24.10.0 |
| npx / npm | 11.6.1 |
| python3 | 3.14.2 |
| `@zed-industries/claude-code-acp` | 0.16.2 |
| `@anthropic-ai/claude-agent-sdk` (its dep) | 0.2.44 |

No Anthropic API key was used; the headless `claude` runs on the local subscription, matching
the capo/ACP architecture. The MCP SDK (`@modelcontextprotocol/sdk`) was npm-installed into the
sandbox to build the test servers.

## What I built (sandbox throwaway artifacts)

1. **Fake `capo` CLI** (python): subcommands `start-agent / list-agents / review / steer`,
   `--json` output, appends each invocation to a jsonl log.
2. **stdio MCP server** (node, `@modelcontextprotocol/sdk` + `StdioServerTransport`): tools
   `start_agent / list_agents / review_agent / steer_agent`, logs each call to jsonl.
3. **http MCP server** (node, `StreamableHTTPServerTransport` + express): same four tools,
   long-running localhost process, logs each call. (First attempt used a per-request
   session-id generator and `claude mcp list` reported "Failed to connect"; switching to a
   **stateless** transport — `sessionIdGenerator: undefined`, 405 on GET — made it
   `✓ Connected`. Worth noting as a real integration gotcha.)

## OBSERVED results per channel (with invocation evidence)

All runs used `claude -p ... --allowedTools ...` (headless), low timeouts, fresh logs.

### Channel 1 — CLI via terminal (instructions in CLAUDE.md, model uses Bash)
3 runs, 3 different phrasings. **Model invoked the CLI every time with correct args.**

- Run 1 ("start an agent to add a hello() fn, then list"): `start-agent --task "add a hello() function" --json`, then `list-agents --json`. (2 invocations)
- Run 2 (indirect: "I need a worker to refactor the auth module. Kick it off and confirm it's running."): `start-agent --task "Refactor the auth module"`, `list-agents`, then it noticed the listing and ran `review --agent agent-001`. (3 invocations)
- Run 3 ("Check on agent-001's progress and tell it to also add tests."): `review --agent agent-001`, then `steer --agent agent-001 --instruction "Also add tests..."`. (2 invocations)

Reliability: 3/3. Args structured and sensible. All four commands exercised across runs.
Caveat: works only because CLAUDE.md told it the exact commands; no tool schema enforces args.

### Channel 2 — MCP (stdio) external server
Registered via project `.mcp.json`. CLAUDE.md mentioned the tools by name only.
**Model invoked the MCP tools.** (`--allowedTools mcp__capo__*` to bypass the approval prompt.)

- Run ("start an agent ... then list"): `start_agent` with a rich structured `task` arg, then `list_agents` with `{}`. (2 invocations, logged in the server's jsonl)

Reliability: 1/1 in this run; tool list + schemas present, so args were well-formed.
Note: `claude mcp list` initially showed "Pending approval" — a fresh stdio server needs a
one-time trust approval; `--allowedTools` sidesteps it in headless mode.

### Channel 3 — MCP (http) server hosted by a long-running local process (the option-3 proxy)
Registered via `claude mcp add --transport http capohttp http://127.0.0.1:7399/mcp`.
Server is a standalone localhost node process (stands in for "capo hosting the endpoint
in-process"). **Model invoked the tools and they routed to my process over HTTP.**

- Connection: `claude mcp list` → `capohttp: ✓ Connected`. Request log shows Claude doing
  `initialize`, `notifications/initialized`, a GET (SSE probe), `tools/list`.
- Run ("start an agent ... then list"): `start_agent {"task":"Add a hello() function"}`,
  then `list_agents {}` — both landed in the http server's invocation log. (2 invocations)

Reliability: 1/1. This is the concrete proof that Claude Code (subscription, headless) will
call tools served by an http endpoint that *you* host in your own process.

## Decisive option-3 findings from the source/spec

Evidence from `@zed-industries/claude-code-acp@0.16.2` (`dist/acp-agent.js`,
`dist/mcp-server.js`) and `@anthropic-ai/claude-agent-sdk@0.2.44` (`sdk.d.ts`):

- **`initialize` advertises** (`acp-agent.js:64-72`):
  `agentCapabilities.mcpCapabilities = { http: true, sse: true }`.
  → claude-code-acp tells ACP clients it can accept client-forwarded http/sse MCP servers.

- **`session/new` consumes a client `mcpServers` array** (`acp-agent.js:723-746`). For each
  entry: if it has a `type` field → treated as a remote server `{ type, url, headers }`
  (i.e. **http/sse**); otherwise → `{ type: "stdio", command, args, env }`. So a CLIENT
  (capo) can pass either a stdio command OR an http/sse URL when creating the session.

- **Forwarded servers go straight to the Claude Agent SDK subprocess** (`acp-agent.js:784`):
  `mcpServers: { ...userProvidedOptions.mcpServers, ...mcpServers }`. The SDK process opens
  the connection to those servers. **There is no ACP-level tunnel for this MCP traffic** —
  for an http/sse server the agent dials the URL directly. capo "intercepts" only because it
  is the one hosting that URL.

- **The built-in `acp` server is the real in-process pattern** (`acp-agent.js:748-754`):
  `createMcpServer(...)` builds a live `McpServer` and registers it as
  `{ type: "sdk", name: "acp", instance: server }`. `sdk.d.ts:302-318` confirms
  `McpSdkServerConfigWithInstance = { type:'sdk', name, instance: McpServer }` is a first-class
  member of the SDK's `McpServerConfig` union (alongside stdio/http/sse). Its tool handlers
  (`mcp-server.js`: Read/Edit/Write/Bash) call **back into the ACP client** —
  `agent.client.createTerminal(...)`, `agent.readTextFile(...)`, `agent.client.sessionUpdate(...)`.
  THIS is "agent calls tool → client's own process handles it," in-process, no socket, no child.
  **But it is internal to claude-code-acp; ACP-forwarded `mcpServers` cannot supply an `sdk`
  instance over the wire** (`sdk.d.ts:320`: the process/serializable transport union excludes
  the `WithInstance` variant — a live object can't be JSON-forwarded across the ACP boundary).

- **`canUseTool` is a universal interception hook** (`acp-agent.js:574-633`, wired at
  `:789`). It runs inside the claude-code-acp process for **every** tool call (built-in,
  stdio-MCP, http-MCP alike) and round-trips to the ACP client via
  `this.client.requestPermission(...)`. capo, as the ACP client, therefore *sees and can
  allow/deny/modify* every tool call regardless of channel — but this is a permission gate,
  not a tool *implementation* hop.

- **Claude Code natively supports http/sse transport MCP** — confirmed by
  `claude mcp add --transport http <url>` (and `sse`), and exercised live in Channel 3.

## Comparison table

| Dimension | CLI (terminal) | External MCP (stdio) | In-process / forwarded MCP (http/sse hosted by capo) |
|---|---|---|---|
| Model invokes it reliably? | Yes, 3/3 (depends on CLAUDE.md) | Yes (tools in list) | Yes (tools in list) |
| Tools appear in model's tool list? | No (just Bash) | Yes | Yes |
| Args structured/validated? | No (free-form CLI flags) | Yes (JSON schema) | Yes (JSON schema) |
| Latency | process spawn per call | process already running (stdio pipe) | localhost socket, process already running |
| Setup complexity for capo | low (a CLI + CLAUDE.md) | medium (separate process + `.mcp.json`/`mcp add`) | medium (capo hosts an HTTP server thread + forwards URL via `session/new`) |
| Separate process/binary? | yes (the CLI per call) | yes (the MCP server process) | **No** — capo hosts the endpoint inside its own process |
| Does capo natively intercept in its own process? | No (it mediates the terminal, but the CLI binary does the work) | No (separate server process does the work) | **Partial-yes** — the handler runs in capo's process; the agent reaches it via localhost, not via ACP |
| ACP tunnels the calls back through the client? | n/a | No | No (agent dials the URL directly) |
| Failure modes | model forgets/misspells commands; arg drift; no schema | server not trusted ("pending approval"); extra process lifecycle | http transport config gotchas (stateless vs session-id → "Failed to connect"); port mgmt; same trust/approval surface |

Two "in-process" flavors exist and must not be conflated:
- **(3a) capo-hosted localhost http/sse MCP** — works today, validated. In-process *hosting*,
  localhost transport. The agent connects directly to capo's port.
- **(3b) true ACP `sdk`-instance interception** (agent tool → ACP client object, no socket) —
  exists and is exactly what claude-code-acp uses internally, but is NOT reachable through the
  client-forwarded `mcpServers` ACP API (live instances can't cross the JSON-RPC boundary). To
  get 3b, capo would have to BE the agent/SDK host (i.e. embed the Claude Agent SDK itself),
  not drive claude-code-acp as an external ACP agent. That contradicts the fixed architecture
  ("capo is the ACP client; the conductor is a claude-code-acp session").

## What I could NOT test (and why)

- I did not run a full custom ACP client that spawns `claude-code-acp` and passes a forwarded
  `mcpServers` over `session/new`. I validated the *agent side* from source (the param exists,
  http/sse accepted) and the *Claude-Code side* by registering an http MCP server through
  `claude` directly. Connecting those two — capo as ACP client forwarding a localhost URL — is
  the one integration hop inferred, not executed end-to-end. Confidence is high because both
  endpoints were independently verified, but flagging it honestly.
- I did not measure real latency numbers; the "latency" column is qualitative.
- Single successful run for the two MCP channels (3 runs only for CLI). Reliability of MCP
  channels at scale is inferred from "tools-in-list + schema," not stress-tested.
- I did not exercise the `canUseTool` permission round-trip live (no full ACP client); its
  behavior is read from source.

## RECOMMENDATION

**Primary: in-process-forwarded-MCP via a capo-hosted localhost HTTP MCP endpoint (option 3a),
with `canUseTool` as the universal supervision hook.** It is the only channel that satisfies
the stated goal — "the agent KNOWS the tools (in its tool list, schema-validated) AND capo's
own process services the call without a separate binary." capo runs the MCP server inside
its own process (a thread/bound port), and forwards `http://127.0.0.1:<port>/mcp` to each
claude-code-acp session via `session/new`'s `mcpServers` (transport advertised and accepted:
`http`/`sse`). No extra binary, no per-call shell-out. Validated: Claude Code connects to and
calls such a server.

Be precise internally about what "in-process" buys you: capo hosts and implements the tools
in-process, but the agent reaches them over a localhost socket, and the calls do NOT tunnel
back through the ACP client connection. The "purest" ACP in-process interception (the
`type:"sdk"` instance + `client.*` callbacks that claude-code-acp uses for Read/Edit/Bash) is
**not** available to an external ACP client over the forwarded-`mcpServers` API; obtaining it
would require capo to embed the Claude Agent SDK itself rather than drive claude-code-acp,
which the fixed architecture rules out.

**Secondary / hybrid:** keep the **CLI** channel as a zero-dependency fallback and for
human/debug use (it worked 3/3 and needs only CLAUDE.md). Avoid an **external stdio MCP server
as a separate process** — it has all the schema benefits of 3a but reintroduces the very
"separate process + config" weirdness the project wants to avoid, with no upside over the
capo-hosted http endpoint.

So: **in-process-forwarded-MCP (http, capo-hosted) as the main channel; CLI as a thin fallback.**
Use `canUseTool` (always present in claude-code-acp) for capo to observe/gate every tool call.
