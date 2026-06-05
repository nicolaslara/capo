# Capo Refocus Plan — one loop, dogfooded

> Status: **design, for review.** No code written yet. This plan refocuses capo on a
> single dogfoodable loop to validate that the design is sound, and lists what to cut
> to bring the codebase back under control.

## The objective (north star)

A simple web chat where the user talks to ONE fast, pluggable **conductor** LLM. The
conductor uses **capo tools** to start / review / steer coding agents on a project.
Agents run **Claude Code via ACP using the subscription** (no API key), in the
project's existing dir by default or in a git worktree on request. Two interaction
modes — **one specific agent** or **all agents** — set explicitly by the user or chosen
smartly by capo. Depth discipline: the conductor stays responsive and pushes non-trivial
work to a subagent; subagents push long work down again; only the **third level** does
long-running work and **strongly favours workflows**.

## Ground truth (verified)

External (verified via web):
- Bridge: **`@zed-industries/claude-code-acp`**, launched **`npx @zed-industries/claude-code-acp`**.
  Speaks **ACP = JSON-RPC 2.0 over NDJSON stdio** — the same wire capo's `AcpWireClient` implements.
- Auth: the bridge **owns its own auth/billing**; **subscription** via `/login` OAuth using
  `~/.claude` credentials. API key not required and not auto-used. → Pass `HOME`, omit
  `ANTHROPIC_API_KEY` to force subscription.
- Tools/MCP: MCP servers can be forwarded over ACP, and the agent reads its own native MCP
  config → capo exposes agent-management tools via an MCP server the session loads.

In-tree (verified by code reads):
- **The ACP path is built but stranded in tests.** `AcpLiveAdapter` + `drive_acp_live_turn`
  live on `FakeBoundaryController` (which is actually the *production* controller despite the
  name) and are called only from `dp10`/`dp11` smoke modules. `capo-server` / `capo-cli` have
  **zero** references to it.
- The server's live path `RunLiveProviderLocal` spawns **Codex** (`CAPO_CODEX_BIN`) or
  **`claude_code`** one-shot (`claude -p --output-format stream-json`, via
  `ClaudeCodeAdapter::local_workspace_write_launch_plan`) and **explicitly rejects `acp`**
  ("live provider preflight supports codex or claude, not acp").
- `dp11 live_acp_smoke` *is* essentially "start an ACP agent that writes a file" — but it uses
  a local **stub** (`write_acp_agent_stub`), lives in a test, and is unreachable from server/web.
- `AcpLiveAdapter` takes `program` + `argv` + transport (`PipedProcessTransport` live /
  `ScriptedAcpTransport` replay), gated by `ACP_LIVE_RUN_OPT_IN_ENV`. Its env allowlist passes
  `HOME`, not `ANTHROPIC_API_KEY` — already correct for subscription.
- `WorktreeManager` (`crates/capo-runtime/src/worktree.rs`): `git worktree add` + lifecycle
  events; only re-exported / test callers today.
- `capo-web` (axum): `/api/dashboard`, `/api/commands`, `/api/thread`, `/api/events` (SSE).
  `web/app` is React+Vite+Tailwind — a **dashboard, not a chat**.
- `capo-tools`: registry of capo-owned tools (`capo.agent_status`, `capo.session_summary`, …)
  + wrapper tools, LLM-callable via `authorize_and_invoke`.
- No Anthropic API client. Parent/child + continuation scaffolding exists but is unwired.

**Conclusion: the design is mostly there. The slice is mostly wiring + de-scoping, not new
architecture.** The single highest-value, highest-risk move is to lift the stranded ACP path
into the server loop pointed at the real bridge.

## The minimal vertical slice (two stages)

### Slice A — the load-bearing gap: real Claude-Code-over-ACP agent, reachable from the server
Promote `dp11` from *stub + test* to *real bridge + server command path*:
1. Make `RunDispatchTurn` / the live-provider path accept an **`acp`** adapter binding instead
   of rejecting it; route it to the existing `drive_acp_live_turn` on the production controller.
2. Launch `npx @zed-industries/claude-code-acp` via `AcpLiveAdapter` (`program`+`argv`),
   confined to the project dir, env = allowlist + `HOME` (subscription), **no** `ANTHROPIC_API_KEY`.
   Make the bridge command an override env (e.g. `CAPO_ACP_AGENT_CMD`, default
   `npx @zed-industries/claude-code-acp`) so it stays pluggable.
3. Worktree option: default same-folder; when requested, call the existing `WorktreeManager`
   from the running loop and confine the agent to the worktree path.
4. Result: a registered `acp` agent runs a turn, the bridge edits a file, capo records
   **observed** evidence (fs change + event log).

This stage IS the headless E2E gate (see VALIDATION below).

### Slice B — the conductor + web chat
1. capo hosts an **in-process localhost HTTP MCP endpoint** (NOT a separate binary) exposing
   agent-management tools: `start_agent(project, task, worktree?)`, `list_agents()`,
   `review_agent(id)`, `steer_agent(id, msg)`, `set_mode(scope: one|all, agent_id?)`,
   `get_agent_output(id)`. These wrap existing server commands (`RegisterAgent`,
   `SendTask`/`RunDispatchTurn`, `SteerAgent`, `Dashboard`, `/api/thread`). The endpoint is
   forwarded to each `claude-code-acp` session via the `session/new` `mcpServers` array
   (`{type, url, headers}`; the agent dials the localhost URL directly). Every tool call also
   round-trips to capo via the ACP **`canUseTool`** callback — capo's universal
   supervision/gate hook on the wire. (DECIDED empirically — see
   `EXPERIMENT-tool-channel.md`. CLI-via-terminal works too and is kept as a thin zero-dep
   fallback; a separate-process stdio MCP server is rejected.) The HTTP MCP transport must be
   **stateless** or `claude mcp` reports a connect failure.
2. The **conductor** is a long-lived `claude-code-acp` ACP session owned by capo-server,
   launched with that MCP server in its config (model pluggable via the bridge's config).
3. capo-web gains a **chat surface**: a `POST /api/chat` (turn → conductor session) and reuse
   the existing SSE `/api/events` tail for streaming; a thin chat view in `web/app` plus a
   one-agent/all-agents mode toggle. Smart routing = a cheap decision the conductor LLM makes,
   or it calls `set_mode`.

### Depth discipline mapping
- **L1 conductor** (`claude-code-acp` session behind chat): responsive; for any non-trivial ask
  it calls `start_agent` rather than working inline.
- **L2 worker** (each `start_agent` → another `claude-code-acp` ACP agent / capo session): does
  the work; for long sub-tasks favours spawning its own subagents.
- **L3** (inside a worker): long-running tasks **favour workflows**. Maps onto the existing
  parent/child + continuation scaffolding (to be wired in a later workflow). The slice only
  needs L1→L2 to prove the model; L3-as-workflows is the next layer.

## Cut list (ruthless — bring capo back under control)

"Cut" = remove from the **default running path** (feature-gate / delete-later), get tests green
without it. Not all deletion happens at once; the point is the slice's runtime is small.

Cut from the running path:
- `capo-voice` (voice intake), `capo-eval` (stub), `capo-workpads` (stub) — out.
- `capo-memory` beyond a minimal context packet — disable extraction/staleness/FTS jobs.
- **Codex** live adapter path and the **one-shot `claude -p`** (`claude_live.rs`) path — superseded
  by ACP; gate off.
- Advanced **recovery / orphan-reaping / checkpoint / shadow-git**, **continuation scheduler** —
  out of the slice (revisit continuation when wiring L3 workflows).
- `parent_child` stays *dormant* until L3 depth work; do not delete (it's the L2/L3 seam).

Keep (the slice's spine): `capo-core`, `capo-state` (event log), `capo-server` (command surface),
`capo-controller` (the one real turn loop + `drive_acp_live_turn`), `capo-adapters` (acp_* only),
`capo-runtime` (LocalProcessRunner + worktree), `capo-tools` (minimal agent-management tools for
the conductor MCP), `capo-query` (read models), `capo-web` (+ chat surface).

## Open questions (human decisions)

1. ~~**Conductor tool exposure**~~ — **DECIDED** (empirically, `EXPERIMENT-tool-channel.md`):
   capo-hosted **in-process localhost HTTP MCP**, forwarded via `session/new` `mcpServers`,
   with **`canUseTool`** as the supervision/gate hook; CLI-via-terminal kept as a thin
   fallback. Caveat: the agent dials the localhost MCP URL directly (not tunneled through the
   ACP pipe), so "in-process" = capo *hosts the socket*; capo still services every call and
   `canUseTool` still round-trips for supervision. Transport must be stateless.
2. **Model pluggability:** the model is chosen inside the `claude-code-acp`/Claude Code config,
   not by capo directly. Is "set the bridge's model via config/env" sufficient, or do you want
   capo to control it per-session? *Recommendation: bridge config for the slice.*
3. **Cut vs delete:** for this slice, is feature-gating the cut list acceptable (delete later),
   or do you want crates physically removed now? *Recommendation: gate now, delete in WF5.*
4. **One-vs-all "smart" routing:** ship explicit toggle only for the slice, add LLM auto-routing
   in Slice B? *Recommendation: yes.*
5. Confirm exact bridge package/version (`@zed-industries/claude-code-acp` vs the renamed
   `claude-agent-acp`) at implementation time.

## Recommended next workflows

- **WF2 — build Slice A.** Wire `acp` into the server dispatch path + launch real bridge +
  worktree option. Worktree-isolated implementing agents; adversarial review of the wiring.
- **WF3 — validation harness.** Build the two-tier E2E (replay + live), record the replay
  fixture from one live run. (Can interleave with WF2.)
- **WF4 — build Slice B.** capo MCP server + conductor session + web chat + mode toggle.
- **WF5 — cut/de-scope sweep.** Feature-gate the cut list, get tests green, report LoC reduction.
