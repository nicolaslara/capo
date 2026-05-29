# Dashboard Webclient Knowledge

## Objective

Capture design decisions, review findings, visual QA notes, and implementation
lessons for Capo's browser dashboard/web client.

## Scope Decision

Create a new `dashboard-webclient` workpad.

The completed `workpads/features/dashboard.md` work produced the shared query
and text dashboard surface. That is an input to the web client, not the web
client itself. The browser app needs its own design and screenshot-review loop
because visual quality, responsive behavior, interaction design, and browser
smoke evidence are different work from query aggregation.

The workpad is queued after `goal-orchestration` by default. A web shell can be
started earlier if explicitly pulled forward, but the richest dashboard depends
on goal/story/evidence/validation projections from goal orchestration.

## Design Principles

- Capo is an operational tool, so the UI should be quiet, dense, readable, and
  optimized for repeated inspection.
- The first viewport should be the actual dashboard, not marketing copy.
- The browser client is a client/input surface. It renders read models and
  sends commands; it does not own controller state.
- Every visible section should answer an operator question or support a safe
  action.
- Normal views should use product language. Raw IDs, hashes, provider flags, and
  policy internals belong behind details/debug affordances.
- The UI should make uncertainty explicit: blocked, unreviewed, unvalidated,
  stale, redacted, and partial evidence must look different from complete and
  validated work.

## First UX Questions

- What is running right now?
- What did each agent do recently?
- Which work is blocked or waiting on a permission/user decision?
- What evidence exists for the latest result?
- Has this work been reviewed or validated?
- What can I safely do next: inspect, steer, stop, interrupt, review, or export?
- Later, for goals: what was the objective, what requirements remain, and what
  story does Capo tell about the execution?

## Visual Direction

Use an operational dashboard style:

- restrained color palette with semantic status colors, not a single-hue theme;
- compact rows/tables for scan-heavy state;
- split-pane or master/detail layout for agents and sessions;
- timeline/story views for activity and evidence;
- details drawers for raw metadata;
- clear icon+label controls for safe actions;
- confirmation states for interrupt/stop/mutating actions.

Avoid:

- landing-page hero sections;
- decorative cards inside cards;
- gradient/orb backgrounds;
- oversized marketing headings;
- hiding important agent status behind abstract visuals.

## Review Policy

Design must be reviewed before implementation. Implementation must be reviewed
with screenshots before completion.

Each UI slice should produce:

- desktop screenshot;
- mobile screenshot;
- notes on visual issues;
- fix/iterate pass;
- final screenshot or explicit residual risk.

Completion is not just "build succeeds." The UI must look coherent and support
the intended operator workflow.

## Data Ownership

The frontend should consume:

- shared query/read models;
- server command APIs;
- typed view models/DTOs;
- fixture data for design and tests.

The frontend should not:

- read SQLite directly;
- scan artifact directories directly;
- parse raw provider logs;
- make scheduling or permission decisions;
- store sensitive transcripts or credentials in browser persistence by default.

## Completion Notes - 2026-05-28

The first dashboard webclient slice is complete as a dependency-free static app
under `web/dashboard/`.

Stack decision:

- Use plain HTML/CSS/JavaScript for the first slice.
- Serve with a tiny Node dev server using built-in modules only.
- Keep fixture data and mocked command handling local to `web/dashboard`.
- Do not add a package manager dependency until the live webclient surface needs
  routing, component tests, or a richer build pipeline.

Implemented views:

- Overview status strip.
- Agent master list and selected session detail.
- Command panel for mocked `steer_agent`, `interrupt_agent`, and `stop_agent`
  server-command requests.
- Recent activity.
- Evidence, reviews, and validation lanes.
- Goals/story fixture view.
- Settings view.
- Debug details drawer for raw ids and lower-level metadata.

Browser verification:

- Dev server command: `node web/dashboard/scripts/dev-server.mjs`.
- Static verifier: `node web/dashboard/scripts/verify.mjs`.
- Browser smoke: `node web/dashboard/scripts/browser-smoke.mjs`.
- Dev URL: `http://127.0.0.1:4173`.
- Screenshots:
  - `workpads/dashboard-webclient/screenshots/dashboard-desktop.png`;
  - `workpads/dashboard-webclient/screenshots/dashboard-detail-command.png`;
  - `workpads/dashboard-webclient/screenshots/dashboard-goals.png`;
  - `workpads/dashboard-webclient/screenshots/dashboard-mobile.png`.

Screenshot review findings:

- Initial agent status chips stretched too wide in row layout.
- Fixed `.status` to use inline-flex, start alignment, and nowrap.
- Final desktop/mobile screenshots show nonblank, framed, scan-friendly UI with
  no obvious overlap.

Residual risks:

- The webclient currently uses fixture data and a mocked server-command API. A
  live HTTP/query adapter is the next real product step.
- Goal/story data is fixture-backed until goal-orchestration read models exist.
- Accessibility validation is checklist/browser-smoke level, not a full audit.

## Open Questions

- Which crate or binary should own the eventual live HTTP/query adapter?
- Should live updates use polling first, or introduce server-sent events once
  active agent runs become common?
- Should screenshot artifacts stay in workpads permanently or move to a
  gitignored QA artifact directory after design acceptance?

## Operator Console Rebuild - 2026-05-29

The first static slice's design was rejected as low quality. It was replaced by a
full operator console under `web/app/`, designed through the
`dashboard-design-iteration` workflow and live-wired to the real server.

Design selection:

- Built 5 production-grade design directions as standalone mockups
  (`design-explorations/`), rendered them, and scored them with a 4-lens judge
  panel (hierarchy/typography, operational info-design, production craft,
  accessibility). Scores in `design-explorations/` notes.
- `terminal-native` was selected (user call), refined for craft, and shipped in
  BOTH light and dark. Tokens live in `web/app/src/index.css`.

Stack (supersedes the dependency-free decision now that routing/build are needed):

- Vite + React + TypeScript + Tailwind v4 + Radix + cmdk (bun), chosen to mirror
  `../openchamber` so its patterns transfer. The old static app under
  `web/dashboard/` remains as fixtures + the `shoot.mjs` screenshot tool.

Screens (7): Overview, Agents (inspect + control with dispatch pipeline +
steer/interrupt/stop), Chat console (inline permission cards), Goals,
Activity/Events, Tools/Permissions, Settings. Shared NavRail, ⌘K command palette,
IDE status bar.

Live wiring:

- New crate `crates/capo-web` (axum + tokio + tower-http) wraps an in-process
  `CapoServer` (`CapoServer::open` + `handle`), opened inside `spawn_blocking`
  per request (the SQLite-backed controller is not Send across awaits).
- Endpoints: `GET /api/dashboard` (typed snapshot mapped to the console's JSON
  model), `POST /api/commands` (steer/interrupt/stop), `GET /api/events` (SSE,
  polls the snapshot every 1.5s), and static serving of `web/app/dist`.
- The front-end auto-detects the facade (fetch `/api/dashboard`); if present it
  switches to `live` mode + subscribes to SSE, else falls back to fixtures.
- Verified live against `.capo-dev`: 7 real agents, real adapters (incl. `acp`),
  real dispatch pipeline state. Evidence in `screenshots/console/LIVE-*.png`.

Run:

- Front-end dev: `cd web/app && bun install && bun dev` (http://127.0.0.1:5273, fixtures).
- Live: `cd web/app && bun run build` then
  `CAPO_STATE_ROOT=.capo-dev cargo run -p capo-web` (http://127.0.0.1:4177).
- Screenshots: `node web/dashboard/scripts/shoot.mjs --out <dir> <url>` (honors `?theme=`).

Live data:

- The facade reads the lanes (activity / event log, evidence, reviews,
  validations) directly from `capo-query` (`project_dashboard`), so Overview,
  the agent table, the dispatch pipeline, the ledgers, and the Activity event
  log are all LIVE. Mutations (steer/interrupt/stop) still go through
  `CapoServer::handle`. Verified against `.capo-dev`: 7 agents, 50 real events.
- Still fixture-only in live mode: goals, tool catalog, permission queue, and
  chat history — these need new projections / ServerCommands (goal projection,
  tool-catalog query, permission-queue exposure, event→chat synthesis). They
  show honest empty states in live mode and full data in fixture mode.

Responsive: fixed a shell grid-overflow bug (a nested `grid-rows` wrapper with an
implicit `auto` column let content force the layout wider than the viewport; the
fix is `grid-cols-1` on that wrapper + an explicit `56px minmax(0,1fr)` template
on the app shell). All screens now fit 390px — dense tables reflow to stacked
cards (Overview) or scroll within their panel (Activity, Tools, status bar).

No auth / remote-exposure; the facade is loopback-only.

## Next Goal - Agent transcript + live conversation (streaming)

The headline next feature: an operator should be able to **see what an agent has
actually done** (its full turn transcript — messages, tool calls, outputs,
evidence) and **send a message and watch the agent's response come back, ideally
streaming**.

This is gated on backend work, so do it in this order:

1. **Build out the agent turn loop first.** Today most `.capo-dev` agents are
   fixture/replay-backed; steering a session-less agent returns
   `AgentHasNoActiveSession`. We need a real turn loop where an operator message
   opens/continues a session and the agent produces a streamed response.
2. **Expose a per-agent transcript + a streaming response channel** from the
   server (e.g. an event/turn stream keyed by session, surfaced over the
   `capo-web` SSE endpoint, or token streaming if the provider supports it).
3. **Then wire the console Chat screen** to render the real transcript and stream
   the agent's reply token-by-token (the Chat UI, composer, and SSE plumbing
   already exist; `web/app/src/data/store.tsx` currently appends optimistic
   operator messages and surfaces command errors).

Deferred until the above: live steer/interrupt/stop work for agents WITH a
session today; the console now surfaces the `AgentHasNoActiveSession` error
clearly instead of failing silently, but full two-way conversation needs the
turn loop + streaming above.
