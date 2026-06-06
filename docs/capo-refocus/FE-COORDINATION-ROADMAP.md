# FE Coordination Roadmap — from validated loop to a usable daily driver

> Status: **synthesis, for review.** Builds on REFOCUS-SUMMARY (validated loop on `main`),
> CONTROL-PLANE-RESEARCH (Path-1 lockdown proven, Path-2 needs the v0.33.x bridge fork),
> PLAN/VALIDATION, and the two analyst gap maps (Analyst A: current-state + gap map;
> Analyst B: requirements + biggest wins). All `file:line` anchors are in `crates/...`.

---

## 0. The defining fact (frame everything around this)

**capo's backend control surface is ~80% there; the FE only consumes the read-only slice of it.**

- The web command surface `/api/commands` (`capo-web/src/main.rs:739-776`) already accepts
  `steer_agent`, `interrupt_agent`, `stop_agent`, `send_task` — and **no UI element calls any
  of them.** `chat.html` only ever hits `/api/chat`, `/api/dashboard`, `/api/thread`,
  `/api/events`. The agent-detail pane (`chat.html:470-548`) is strictly read-only.
- Consequence: **the human's only actuator is a chat box to ONE conductor LLM.** Every act of
  coordination (start, steer, review, cancel) must be phrased in English to the conductor, who
  *may* call a tool. There is no direct manipulation of a worker.
- One deeper backend caveat the FE work must respect: `steer_agent`/`interrupt_agent`/
  `stop_agent` today **record events / mutate read-model state but do NOT reach a live ACP
  worker process** (`lib.rs:533/593/650`; `controller/lib.rs:271-286`). Steering is recorded,
  not delivered. So a chunk of the value (real mid-flight steer/cancel) needs a backend
  primitive — an **in-flight ACP turn registry** — that does not exist yet.

The roadmap therefore splits cleanly: **most wins are FE wiring against existing endpoints
(no bridge fork);** a smaller set needs the in-flight-turn registry (BE, fork-free); and only
the deepest layer (below-L2 workflow/sub-agent visibility) needs the **`claude-agent-acp`
v0.33.x bridge fork** described in CONTROL-PLANE-RESEARCH.

---

## 1. Target — what "usable daily driver to coordinate bg agents + workflows" means

A single web console where, across a normal working day, the user can:

1. **Launch** background coding agents on a chosen project/worktree from the UI (not only by
   asking the conductor in prose), and keep the conductor responsive while they run.
2. **Watch many at once** — a live tree of conductor (L1) → workers (L2) → (where observable)
   workflow/sub-activity (L3) — with real per-agent status (running/queued/failed/done +
   timing), not string-matched guesses.
3. **Drill into one** — see a specific worker's live transcript and what it is doing.
4. **Steer one directly** — type to a single worker and have it actually reach the live
   session; **interrupt/cancel** a worker going down the wrong path, mid-flight.
5. **Review results** — browse the artifacts/files a worker changed and see a real `git diff`,
   not an English summary.
6. **Trust the control plane** — see and act on capo's `canUseTool` permission asks
   (allow / deny-with-guidance) in the UI.
7. **Survive reality** — reload the page or reconnect mid-turn without losing the transcript
   or the in-flight activity; juggle more than one project.

"Usable" = the user reaches for this console instead of a terminal to run their day's agent
work, and trusts it enough to leave agents running in the background. The minimum bar that
flips it from demo to driver is **direct per-agent action + a responsive non-blocking
conductor + a real results/diff view** (Wins 1–3 below).

---

## 2. The biggest wins (top 5)

Ordered by impact-per-effort. The top three are predominantly **FE work against capabilities
that already exist** — that is where capo is leaving the most value on the table.

### WIN 1 — Direct per-agent action bar: steer + interrupt + stop (+ permission asks)
- **Impact: very high.** Converts capo from "narrate your intent to one LLM and wait" into a
  real direct-manipulation console where the human can grab *any* worker. This is the single
  biggest demo→daily-driver gap.
- **Effort: low for the FE shell; medium overall** (the FE buttons are a few dozen lines of
  HTML/JS posting to `/api/commands`, which already accepts these actions). The agent-detail
  pane already knows the selected agent's id/sessionId.
- **Dependencies:** No bridge fork. **BUT** the backend actions currently only record
  events — they do **not** reach a live worker (`lib.rs:533/593/650`). So WIN 1's *visible*
  shell ships immediately, but its *real effect* depends on the **in-flight ACP turn registry**
  (see WIN 2 / Phase B). Sequence the FE buttons first (instant legibility + the "send_task"/
  recorded-steer path works), then wire them to the live registry.
- **Why it's a win:** highest ROI; unblocks the core interaction model; mostly already built
  on the backend.

### WIN 2 — Non-blocking conductor + live reconnect (responsiveness + durability)
- **Impact: very high.** Today `/api/chat` holds a `turn_lock` and blocks synchronously for
  the whole worker turn (`main.rs:379-456`, no timeout); the UI is dead for the duration and a
  reload loses everything. These are the two things that most break the "all day" illusion.
- **Effort: low–moderate.** Two coupled fixes: (a) switch the web path's `start_agent` to the
  already-existing **`detached`** flag (commit `9b17208`; REFOCUS-SUMMARY caveat #1 calls this
  "a small, deliberate follow-up") so the conductor returns immediately and workers run in the
  background; (b) on load, **replay `/api/events?from=0`** (the event log is durable in
  capo-state; the `lastSeq`/`from=N` watermark plumbing already exists) to rehydrate in-flight
  agents + transcript, then resume the SSE tail.
- **Dependencies:** No bridge fork. Needs a coherent **turn-lifecycle model** (WIN 4's status
  model) to be fully clean, and the verbatim-reply redaction policy decision (`main.rs:529-539`
  returns a redacted label, not raw prose) for transcript rehydration.
- **Why it's a win:** makes capo *feel* like a tool you leave running; "async start_agent" is
  the headline item from the known-gaps list and it is mostly already-built primitives.

### WIN 3 — Results / diff pane per worker (close the review loop, on worktrees)
- **Impact: high.** Reviewing output is the *point*; English summaries aren't reviewable. Add a
  "Changes" tab to the detail pane showing the worker's `git diff` / `git status` for its run
  dir or worktree, plus the files it wrote.
- **Effort: moderate.** A small read endpoint (`/api/diff?session=` → `git diff` in the agent's
  cwd) feeds an FE pane. `WorktreeManager` and the confined run dir already exist.
- **Dependencies:** No bridge fork. Pairs naturally with worktree-per-worker (WIN 4 area), and
  forces a results-collection rethink: today the shared single workspace is what makes
  `collect_results` work; per-worker worktrees need a results path that survives isolation.
- **Why it's a win:** completes launch → watch → **review the actual code**; worktree isolation
  makes parallel fan-out safe to review.

### WIN 4 — Agent/workflow tree view with real lifecycle status (L1→L2, fork-free)
- **Impact: high.** The sidebar is a flat 2s-poll list (`chat.html:428-449`) and status is
  inferred by string-matching `run_status` (`main.rs:1121`, `statusClass()`). Group it as
  **conductor → its workers** using the spawn relationship capo already has (each `start_agent`
  worker is a distinct capo session the conductor created), and back it with a real
  turn/run lifecycle read model (running/queued/failed/done + timing). Surface detached-worker
  failures as events (today they only `eprintln!` to stderr, `acp_mcp_http.rs:455`).
- **Effort: moderate (FE) + small backend** (a lifecycle projection over capo-state; enrich
  `/api/dashboard`).
- **Dependencies:** No bridge fork for the **L1→L2** layer. The **L3 sub-tree (a worker's
  internal workflow / native sub-agents) is the part that needs the fork** — defer to WIN 5.
- **Why it's a win:** delivers the "watch many at once + drill into one" affordance — the core
  console metaphor — without the heavy dependency.

### WIN 5 — L3 workflow / native sub-agent visualization (needs the v0.33.x bridge fork)
- **Impact: high but deferrable.** A worker's internal fan-out is opaque; native sub-agents
  inside a worker are invisible/unsteerable (the whole point of CONTROL-PLANE-RESEARCH). True
  observe/veto + nested visualization requires the **forked `claude-agent-acp` v0.33.x**:
  `agentID` forward (attribution), relaxing the `parent_tool_use_id===null` streaming gate
  (nested text/thinking), and the deny-with-guidance `_meta` passthrough (veto+steer).
- **Effort: high; highest fragility in the whole roadmap** (a ~3,750-line bridge file under
  heavy churn; diffs land in the hottest message-routing zone). Must be re-based on v0.33.x
  first (Proto-6), which already forwards `_meta.claudeCode.parentToolUseId`.
- **Dependencies:** **The bridge fork.** This is the only roadmap item genuinely blocked on
  Path-2.
- **Why it's a win (later):** it's the only way to see *below* L2 without forcing all fan-out
  through capo `start_agent`. Do it **after** Wins 1–4 prove the loop is worth the investment.

> **Honorable mention — permission-ask UI (canUseTool).** Surface pending `canUseTool`
> requests as an Allow / Deny-with-guidance queue in the FE. Medium effort, medium-high
> trust/safety impact, fork-free for the Allow/Deny shell (the *deny-with-guidance message*
> reaching the model needs the bridge patch). Fold the shell into Phase B alongside WIN 1.

---

## 3. Sequenced roadmap (current state → usable)

Legend: **[FF]** fork-free · **[REG]** needs in-flight-turn registry (BE, fork-free) ·
**[FORK]** needs the `claude-agent-acp` v0.33.x bridge fork.

### Phase A — Make it a console (FE-only, fork-free) — *the smallest valuable step*
Ship the direct-manipulation shell + responsiveness against endpoints that already exist.
- **A1 [FF]** Per-agent action bar in the detail pane: steer box + Interrupt + Stop buttons →
  `POST /api/commands`. (WIN 1 shell.) *This is the single highest-ROI first slice.*
- **A2 [FF]** Non-blocking conductor: switch the web path's `start_agent` to `detached`; the
  conductor returns immediately and workers run in the background. (WIN 2a.)
- **A3 [FF]** Reload/reconnect: replay `/api/events?from=0` on load to rehydrate agents +
  in-flight activity, then resume the SSE tail. (WIN 2b.)
- **A4 [FF]** Conductor → worker **tree** in the sidebar (nest the existing flat list by spawn
  relationship). (WIN 4, L1→L2 only.)
- **Exit:** the user can launch via the conductor, watch a live tree, drill into a worker, and
  *issue* steer/interrupt/stop from the UI — and reloading doesn't lose state. (Steer/cancel
  are recorded but may not yet hit the live worker — fixed in Phase B.)

### Phase B — Make the actions real + reviewable (BE registry, still fork-free)
- **B1 [REG]** Build the **in-flight ACP turn registry** (session_id → cancel handle / prompt
  channel). The load-bearing missing primitive. The spawned `claude-code-acp` child is created
  per turn and its handle discarded today (`acp_live.rs:236`); keep it alive.
- **B2 [REG]** Wire `interrupt`/`stop` → ACP `session/cancel`; wire `steer` → a follow-up
  `session/prompt` on the same live session. Now WIN 1's buttons actually reach the worker.
- **B3 [FF]** Real turn/run **lifecycle** read model (running/queued/failed/done + timing);
  enrich `/api/dashboard`; surface detached-worker failures as events. (WIN 4 backend.)
- **B4 [FF]** Per-worker **diff/results pane** (`/api/diff?session=`) + worktree-per-worker +
  project/worktree picker; rethink `collect_results` for isolated worktrees. (WIN 3 + the
  worktree/multi-project gap.)
- **B5 [FF]** `canUseTool` permission-ask **Allow/Deny** queue in the FE (shell only;
  deny-with-guidance text waits on the fork).
- **Exit:** direct steer/interrupt/cancel genuinely affect running workers; the user reviews
  real diffs; runs are launchable on a chosen project/worktree with honest status. **This is
  the "usable daily driver" milestone** — reachable with **no bridge fork**.

### Phase C — See below L2 (needs the bridge fork)
- **C1 [FORK]** Re-base references to `claude-agent-acp` v0.33.x; apply the unified additive,
  env-flag-gated `_meta` patch set (Proto-6): `agentID` forward, relax the
  `parent_tool_use_id===null` streaming gate, deny-message + `interrupt:false` passthrough.
- **C2 [FORK]** L3 **workflow/sub-agent tree** nested under each worker; deny-with-guidance
  wired into the permission UI (veto+steer).
- **Exit:** a worker's internal workflow/native sub-agents are observable + vetoable;
  full-depth legibility.

### Cross-cutting: legibility polish
The legible-streaming conductor + agents sidebar already landed (REFOCUS-SUMMARY) and polish is
in progress. It threads through every phase rather than being a milestone: in **Phase A** it
makes the new tree + action bar readable; in **Phase B** it renders lifecycle status chips and
diffs; in **Phase C** it renders nested sub-agent rows + veto badges. Keep it continuous, not
gated.

### Where the named candidates land
- **async start_agent** → A2 (flag already exists; flip the web path).
- **per-agent steering UI** → A1 (shell) + B2 (real delivery).
- **interrupt/cancel** → A1 (shell) + B2 (real delivery via `session/cancel`).
- **persistence / reconnect** → A3.
- **results/artifacts viewing** → B4.
- **worktree/project management** → B4.
- **workflow-tree visualization** → A4 (L1→L2, free) + C2 (below L2, needs fork).
- **bridge fork / native sub-agent observe+veto / CLI streaming / auth-beyond-loopback** →
  Phase C and beyond (auth is a fork-free product decision, deferrable until remote use).

---

## 4. Risks & the durable ceiling

- **The durable ceiling (ACP cannot model per-sub-agent sessions).** `SessionId` is flat and
  opaque; `RequestPermissionRequest` carries no sub-agent attribution; the spec has no
  parent/child primitive in v1 or v2 (CONTROL-PLANE-RESEARCH Investigation C, confirmed durable
  in Thread B). Therefore **full workflow visibility has exactly two routes:** (1) **capo-native
  `start_agent` fan-out** — force all delegation through capo so each unit is a first-class,
  observable capo session (Path-1; fork-free; this is what Phase A's tree exploits at L1→L2),
  or (2) **the bridge fork** — observe/veto native sub-agents in-process via `agentID` +
  relaxed streaming gate (Path-2; Phase C). There is no third option, and no spec change will
  give per-sub-agent sessions. Plan accordingly: favor Path-1 fan-out for anything you want
  first-class control over; use the fork only for residual native sub-agent visibility.

- **Steering is recorded, not delivered (today).** The biggest backend gotcha: until the
  in-flight-turn registry (B1) exists, the steer/interrupt/stop buttons mutate state without
  touching the live worker. Risk: shipping A1 alone can *look* like it works while doing
  nothing to a running agent. Mitigation: ship A1 with honest UI state, prioritize B1/B2.

- **Synchronous conductor masked by prompt-engineering.** Responsiveness today depends on the
  conductor being prompted to always use `detached`+`collect_results` (`conductor_goal()`,
  `main.rs:326-341`) — fragile. A2 replaces prompt-luck with a real flag flip.

- **Verbatim-prose redaction.** Transcript rehydration (A3) and results viewing (B4) collide
  with capo's content-hash redaction policy (`reply_text()`, `main.rs:529-539`). Needs a
  deliberate policy decision on what raw prose the FE may show.

- **Worktree vs `collect_results`.** Per-worker worktrees (B4) break the shared-workspace
  assumption that makes cross-agent result collection trivial today. Don't ship worktrees
  without reworking results collection.

- **Bridge-fork fragility (Phase C).** A ~3,750-line file under heavy churn, diffs in the
  hottest zone; keep changes small, additive, env-flag-gated, re-based on v0.33.x, and pursue
  upstreaming to drive maintenance toward zero. SDK coupling caps how first-class sub-agents
  can ever be; Pro/Max SDK availability is an external risk.

- **Sequencing risk.** The temptation is to start the fork (it's the "hard" research item).
  Resist: Wins 1–4 deliver a usable daily driver with **zero fork**, and the fork is the
  highest-cost/highest-fragility item. Bank Phase A+B value first; let it justify Phase C.

---

## 5. The first milestone (do this first)

**Phase A — "Make it a console":** A1 (per-agent steer/interrupt/stop buttons →
`/api/commands`), A2 (flip the web `start_agent` to `detached` so the conductor stays
responsive), A3 (replay `/api/events?from=0` on load to survive reload), A4 (nest the sidebar
into a conductor→worker tree). All FE-only, all against endpoints/flags that already exist, no
bridge fork. The headline first slice within it is **A1**: it is a few dozen lines of HTML/JS
and turns the read-only detail pane into a real control surface — the largest demo→driver step
for the least effort. Immediately follow with **B1/B2** (the in-flight-turn registry) so those
buttons actually reach a live worker, since steering is currently recorded but not delivered.
