# Capo refocus — summary & results

This branch (`slice-a-acp-wiring`) refocuses capo onto ONE dogfoodable loop and
**validates that the design is sound by proving the loop end-to-end, live, over the
Claude subscription.** Nothing here is merged to `main` or pushed.

## The objective

A simple web chat where the user talks to ONE fast conductor LLM. The conductor uses
**capo tools** to start / review / steer coding agents. Every LLM — conductor and
workers — runs **Claude Code via ACP on the subscription** (no API key); capo is the
ACP client that owns all sessions. Agents run in the project dir (default) or a
worktree. Two interaction modes (one agent / all). Depth discipline: the conductor
(L1) delegates real work to workers (L2); long work favours workflows (L3).

## What was built (all committed on this branch, all tests green)

| Commit | What |
| --- | --- |
| `10881f4` | **Slice A** — wired the previously test-stranded ACP path into the `capo-server` command surface (`RunAcpLiveTurnLocal`): register an `acp` agent, run a confined turn through the controller's `drive_acp_live_turn`, observed file change + event log. Fail-closed behind env gates. |
| `c143fa8` | **WF3 / live bridge** — reconciled capo's ACP wire client with `@zed-industries/claude-code-acp` 0.16.2 so a REAL Claude Code agent, over the subscription, makes an observed file change driven through capo. |
| `c67cfbb` | **WF4 conductor** — capo-hosted in-process STATELESS HTTP MCP server exposing the capo tools (start_agent/list_agents/review_agent/steer_agent/set_mode), forwarded to a conductor session via `session/new`; `RunConductorTurnLocal`. |
| `e2722d5` | **WF4** — the forwarded MCP entry needs a `name` field (the real bridge rejects `session/new` otherwise). |
| `ae148ac` | **WF4b** — live conductor E2E green; root-caused & fixed the worker-write (drive workers in DEFAULT permission mode + steer the prompt so the on-wire `fs/write_text_file` is used and capo's permission decider supervises it, instead of `bypassPermissions` letting the worker simulate the write in a Task sub-agent). |
| `cf5ab40` | **WF5 web chat** — `capo-web` `POST /api/chat` drives a conductor turn; static `/chat` page (no build step); one/all mode. |

## Validation (how we know it works)

**Deterministic (CI-able, every `cargo test`):**
- `acp_dispatch_smoke` — server surface → ACP path → observed file (local `/bin/sh` stub).
- `acp_mcp_http_smoke` — the in-process HTTP MCP server (stateless, bearer-authed; `start_agent` drives a worker → observed file).
- `conductor_turn_smoke` — `session/new` forwards the MCP entry + prompt composition + fail-closed.
- `chat_endpoint_drives_a_conductor_turn_and_returns_reply` — `POST /api/chat` round-trip.
- **Full `cargo test --workspace`: 0 failures across all 14 crates (800+ tests).**

**Live (gated by `CAPO_E2E_LIVE_ACP=1`, real subscription, no API key):**
- `acp_live_bridge_smoke` — a real `claude-code-acp` worker writes a file via capo.
- `conductor_live_e2e` — a real conductor calls `start_agent` over capo's MCP → a real worker writes the file. **3/3 green.**
- **Live web smoke (the whole product): `curl POST /api/chat` "start an agent to create HELLO.txt = capo-works" → the real conductor calls `start_agent` (and sometimes `review_agent`) → the worker writes the file on disk. Ran 3/3 PASS, ~30s each.**

## How to run it yourself (the dogfood)

```bash
cd /Users/nicolas/devel/capo-sliceA
export CARGO_TARGET_DIR=/Users/nicolas/devel/capo/target
export CAPO_WEB_LIVE_ACP=1 CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1
export CAPO_STATE_ROOT=$(mktemp -d)/state CAPO_WEB_ADDR=127.0.0.1:4177
cargo run -p capo-web
# open http://127.0.0.1:4177/chat  and type:
#   "Start an agent to create a file HELLO.txt containing capo-works"
# the conductor delegates via start_agent; the worker writes the file in
#   $CAPO_STATE_ROOT/acp/workspace/
```
Requires: `claude` logged into a Pro/Max subscription, `node`/`npx` (the bridge is
`npx -y @zed-industries/claude-code-acp`). No `ANTHROPIC_API_KEY` — it's scrubbed.

## Key design facts learned (verified)

- The whole stack is **subscription-only via ACP**; capo is the ACP client (the
  Zed-equivalent). No API key anywhere.
- Tool channel = capo-hosted **in-process localhost HTTP MCP** (must be STATELESS),
  forwarded via `session/new mcpServers {name,type:http,url,headers}`; every tool call
  also round-trips via `canUseTool` (capo's supervision hook). MCP was the chosen
  channel after an empirical 3-way test (vs CLI-via-terminal, vs native ACP tools).
- Workers must run in **default permission mode** (not `bypassPermissions`) so the
  Write is an on-wire `fs/write_text_file` capo supervises — otherwise the worker can
  delegate to a Task/Bash sub-agent whose filesystem ops are *simulated* and never
  land. The worker prompt is also steered to edit files directly.

## Honest caveats / open items

- **Conductor responsiveness**: `start_agent` now has an optional `detached` flag
  (commit `9b17208`) — when set, it returns immediately (`status:running`) and the
  worker runs on a background thread; the conductor polls via `review_agent`/
  `list_agents`. This is the depth-discipline responsiveness path, added additively
  and flag-gated (default stays synchronous = the validated path). NOTE: the live web
  path still calls `start_agent` synchronously by default (so `/api/chat` returns once
  the worker is done); switching the web path to `detached` (and polling for the
  result) is a small, deliberate follow-up — left as a choice so the validated
  synchronous behavior is preserved.
- **Cut sweep** (bring capo back under control by removing/gating peripheral crates &
  superseded provider paths) is provided as an analysis (`CUT-PLAN.md`), NOT executed
  — those crates weren't created in this effort, so the destructive step is left for
  your review.
- The two **modes** (one/all) are wired (`set_mode`, default `all`) but only the
  single-agent path is exercised live; multi-agent "all" steering isn't yet
  live-tested.
- Provider breadth was intentionally narrowed to the ACP/claude-code path; Codex and
  the one-shot `claude -p` paths are still present (candidates for the cut sweep).

## Doc index (this folder)
- `PLAN.md` — the refit plan. `VALIDATION.md` — the validation strategy.
- `EXPERIMENT-tool-channel.md` — the MCP-vs-CLI-vs-in-process experiment.
- `dogfood-conductor-chat.md` — manual browser demo steps.
- `CUT-PLAN.md` — staged plan to shrink capo (analysis only).
- `NIGHT_LOG.md` — step-by-step log of the overnight run.
