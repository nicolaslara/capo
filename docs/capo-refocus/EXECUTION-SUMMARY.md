# Autonomous execution summary (owner away ~5–18h, 2026-06-06)

Working on `main`, committed per verified item, **not pushed**. Plan: `AUTONOMOUS-PLAN.md`.
Open questions for the owner: `REVIEW-WHEN-BACK.md`.

## Done
### WF1 — F1 + F2 ✅ (commit `a8f0f13`)
- **F1 (tool calls as legible feed lines):** the conductor's MCP tool calls now emit a capo
  session event (`tool.call_requested`, actor `agent-conductor`, conductor session) carrying
  `tool_name` + an **allowlisted, clipped** arg summary (`task/goal/path/query/command/name/
  agent_id/mode/detached/worktree`; other args incl. `capo_write` `content` are DROPPED for
  safety). The feed renders `→ start_agent(task=…)`. New public `CapoServer::append_event` seam;
  `McpState.with_conductor_identity`. Best-effort emit. New test
  `tools_call_emits_conductor_tool_event_with_name`. Adapter (worker) tool names are surfaced
  where the source provides them; not fabricated when absent (documented).
- **F2 (prose doubling):** on `item_completed`, the coalesced prose line is REPLACED with the full
  text instead of appending deltas.
- Verified: capo-adapters 80, capo-web 6, capo-server lib 158, acp_mcp_http_smoke 4 — all green; clippy clean.

### WF2 — A1 + A3 + A4 ✅ (commit `97c89a0`)
- **A1 (per-agent action bar):** detail-pane Steer/Interrupt/Stop buttons → `POST /api/commands`
  (`steer_agent`/`interrupt_agent`/`stop_agent`, agent = name). Muted note: these record intent
  server-side and become live once B1/B2 lands — no faked delivery.
- **A3 (reload-survival):** on load, replay `/api/events?from=0` once (streamed via ReadableStream
  because the SSE tail never closes; idle-drain + cancel) into a rebuilt activity feed; `bumpSeq`
  per event so the live tail resumes strictly after the backlog (no double-render). `send` disabled
  until replay settles.
- **A4 (conductor→worker tree):** sidebar groups conductor (`/conductor/i`) as root, others indented
  as workers; flat fallback when no conductor.
- FE-only, no Rust touched. Verified: JS parses (`new Function`), capo-web 6/6 green, build clean;
  review (3-agent plan→implement→review) returned all-PASS, I re-confirmed wirings against the real
  endpoint contracts in `main.rs`.

## In progress
- WF3 — A2 (non-blocking conductor) + B1/B2 (in-flight ACP-turn registry → live steer/interrupt). Next up.

## Decisions made during WF2
- D-WF2a (A4): the dashboard read model has NO parent field, so the conductor→worker tree is a
  fork-free FE heuristic — conductor = id/name matches `/conductor/i` (root), all other agents are
  workers (children). One level. Documented in code. (Mirrored to REVIEW-WHEN-BACK §C.)
- D-WF2b (A1): steer/interrupt/stop buttons POST the real `/api/commands` kinds, but those only
  RECORD INTENT server-side today; the action bar says so explicitly. Live delivery lands in WF3
  (B1/B2). No faked delivery (per D1/D3).

## Decisions made (also mirrored into REVIEW-WHEN-BACK.md §C)
- D-WF1a: reused `EventKind::ToolCallRequested` for the conductor tool event (no new codec variant).
- D-WF1b: arg allowlist + 120-char clip for the emitted tool event (no secret/large args leak).
