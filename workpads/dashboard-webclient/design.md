# Dashboard Webclient Design

## Objective

Define the accepted first browser dashboard slice for Capo before implementation.
The slice should be useful to a local operator supervising agents, while keeping
all orchestration state owned by Capo server/query boundaries.

## Product Brief

### Operators

- Local developer supervising one or more coding agents.
- Reviewer auditing what happened and which evidence supports it.
- Operator recovering a stalled, blocked, timed-out, or unclear run.
- Future remote/mobile operator who needs a compressed status view.

### First Workflows

- Understand project health from the first viewport: agent count, active runs,
  blocked work, evidence, reviews, validations, and goals.
- Inspect active agents and sessions in a master/detail layout.
- Read each agent's latest result, evidence, tool activity, and recent events.
- Queue a safe steering, interrupt, or stop command through a server-command
  boundary.
- Open debug details for raw ids and lower-level metadata.
- Inspect goals, evidence, reviews, validations, and execution history when
  the data exists; otherwise show graceful unavailable/empty states.

### Non-Goals For V0

- No direct SQLite reads from the browser.
- No raw provider transcript retention in browser storage.
- No live Codex/Claude subscription execution from the dashboard.
- No remote exposure by default.
- No marketing/landing-page surface.

## Information Architecture

The first screen is the operator dashboard.

```text
/dashboard
  overview/status strip
  agent master list
  selected agent/session detail
  recent activity timeline
  evidence/review/validation lanes
  command panel
  debug drawer

/dashboard?view=goals
  goal summary
  requirements
  blockers
  validation/review state
  historical reports

/dashboard?view=settings
  server connection
  polling mode
  fixture/live mode
  debug metadata policy
```

For the first static slice, routes are implemented as in-page tabs and hashless
controls so the app works from a plain static server.

## Data Ownership

The browser consumes a view model shaped like the shared query/read models:

- `project`
- `agents[]`
- `goals[]`
- `evidence[]`
- `reviews[]`
- `validations[]`
- `activity[]`
- `debug`

Mutations are represented as typed command requests:

- `steer_agent`
- `interrupt_agent`
- `stop_agent`

The first dev server stores these commands in memory as a mocked server-command
path. It does not edit Capo's SQLite state. A live server adapter can later
replace the same API boundary.

## Visual Design

The visual direction is operational and scan-focused:

- dense status strip at the top;
- two-pane desktop layout with agent list and selected detail;
- single-column mobile layout with persistent command affordances;
- restrained neutral base with semantic green/yellow/red/blue accents;
- compact rows and tables over decorative cards;
- raw ids hidden behind a debug drawer;
- stable component dimensions to avoid layout shift.

Avoid oversized hero sections, gradients, decorative backgrounds, and nested
cards. The first viewport must show the product state immediately.

## Component Map

- App shell: header, server badge, mode badge, tab rail.
- Status strip: agents, active, blocked, evidence, reviews, validations.
- Agent list/table: name, status, adapter, result, evidence/review counts.
- Session detail: result, goal, recent events, tool activity.
- Timeline: recent agent and command events.
- Evidence/review/validation lanes: compact rows with status semantics.
- Command panel: steer text area, interrupt, stop, command log.
- Details drawer: ids, policies, fixture/live mode metadata.
- Goal/story panel: current requirements, blockers, validation state, history.

## States

- Loading: stable skeleton rows.
- Empty: "No agents yet" with a server connection hint.
- Error/offline: server status and retry affordance.
- Fixture mode: explicit fixture badge.
- Redacted/partial: visible status labels, not silent omission.

## Accessibility And Responsive Constraints

- Text must not overlap at 1280px desktop or 390px mobile simulation.
- Buttons have visible focus states.
- Color is not the only status signal; status text is always present.
- Touch targets are at least 40px high where interactive.
- No viewport-width font scaling.
- Reduced-motion preference disables transitions.

## Review Decision

Accepted for the first slice on 2026-05-28.

The design is intentionally fixture-first with a mocked server-command API. That
lets the browser dashboard reach visual and interaction quality now without
coupling the frontend to Capo persistence internals. The residual product gap is
live HTTP integration with the Capo server/query contract.
