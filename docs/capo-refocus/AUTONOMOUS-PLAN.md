# Autonomous execution plan (owner away ~5–18h, 2026-06-06)

Goal: plan → execute → review each item below, with reasonable + documented decisions.
Working on `main` in `/Users/nicolas/devel/capo`. **Commit per verified item; do NOT push.**
Each item: a build workflow (plan → implement → review) → I verify (build + deterministic
tests + live where relevant) → commit. Progress logged in `EXECUTION-SUMMARY.md`.

## Scope (the quoted items)
**Follow-ups (small):**
- F1 — Tool calls show as feed lines with their NAME (`→ start_agent(…)`), not hidden. Needs the
  tool name in the event stream (adapter tool events already carry `tool_name`; MCP tool calls
  like `start_agent` aren't session events today → emit a capo event for them).
- F2 — Fix coalesced-prose doubling: use the `item_completed` full text instead of concatenating
  `item_delta` chunks.

**Phase A — "make it a console" (FE-only, no fork):**
- A1 — Per-agent action bar: steer / interrupt / stop buttons in the detail pane → `POST /api/commands`.
- A2 — Non-blocking conductor: use the existing `detached` start_agent flag appropriately.
- A3 — Reload-survival: replay `/api/events?from=0` (or a watermark) on load to rebuild state.
- A4 — Sidebar → conductor→worker TREE with real lifecycle status (L1→L2, fork-free).

**Then B1/B2 — make the buttons real:** an in-flight ACP-turn registry so steer→`session/prompt`
and interrupt/stop→`session/cancel` actually reach a LIVE worker (today they only record intent).

## Sequencing (smallest/safest first; commit-gated)
1. WF1 → F1 + F2 (small, low-risk; validates the autonomous loop).
2. WF2 → A1 + A3 + A4 (FE-only against existing endpoints).
3. WF3 → A2 + B1/B2 (backend: detached default + in-flight turn registry → live steer/interrupt).
Each gated by: build clean + deterministic suites green + (where live) a gated live check + no
regression to the validated conductor loop + lockdown. Out-of-scope items (report_result,
diversity injection) are deferred to the owner — see `REVIEW-WHEN-BACK.md`.

## Standing decisions (reasonable defaults; owner can veto)
- **D1.** A1's buttons may "record intent" only until B1/B2 lands; WF3 makes them drive a live
  worker. (So A1 ships first, then becomes truly live.)
- **D2.** A2: keep the validated synchronous-aggregation path working (conductor uses
  `collect_results`); make per-worker start_agent non-blocking so parallel fan-out stays fast —
  but only if it does NOT regress `conductor_live_e2e`. If detached-by-default risks the loop,
  gate it behind a flag and document.
- **D3.** B1/B2 is the riskiest. If full live steer/cancel can't be landed safely, implement the
  registry + wiring + a gated live test for what works, and DOCUMENT precisely what's deferred —
  never fake delivery.
- **D4.** Lockdown stays opt-in (default off) so the validated demo is byte-identical.
- **D5.** No pushing; everything on `main` locally, committed per item (revertable).
- **D6.** Honesty: workflows report real build/test/live results; I independently verify before
  committing. If an item can't be completed cleanly, I stop at the last green state + log it.

## Status
- [x] WF1 (F1, F2) — commit `a8f0f13`
- [x] WF2 (A1, A3, A4) — commit `97c89a0`
- [x] WF3 (A2 verified, B2 cooperative cancel; B1 honestly deferred) — commit `c23d1f9`
- [x] EXECUTION-SUMMARY.md final write-up
All committed on `main`, NOT pushed. B1 (live steer) is the one deferral — see REVIEW-WHEN-BACK §C D-WF3b.
