# Dashboard Webclient Completion Audit - 2026-05-28

## Objective Audited

Build a browser dashboard/web client for Capo that gives a human operator a
clear, inspectable, and good-looking view of projects, agents, sessions, goals,
evidence, validation, reviews, and execution history.

The workpad also required direct browser testing with screenshots and
interactive verification.

## Verdict

Complete for the first dashboard-webclient workpad slice.

The completed slice is fixture-backed and uses a mocked server-command API. It
does not claim live HTTP integration with the Capo server. That is the next
dashboard product step, not a blocker for this workpad's accepted fixture-first
browser client.

## Requirement Evidence

| Requirement | Evidence | Status |
| --- | --- | --- |
| Product brief and workflows | `workpads/dashboard-webclient/design.md` defines operators, workflows, and non-goals. | Complete |
| Information architecture | `design.md` defines overview/goals/settings views and state handling. | Complete |
| Visual design brief | `design.md` defines operational dashboard direction, layout, components, and accessibility constraints. | Complete |
| Design review gate | `design.md` records accepted fixture-first design and residual gap. | Complete |
| Webclient stack | `web/dashboard/` uses dependency-free HTML/CSS/JS plus Node built-in dev server. | Complete |
| API/query contract | `web/dashboard/fixtures/dashboard.json` and `scripts/dev-server.mjs` expose fixture read data and mocked server commands. | Complete |
| First usable dashboard | `index.html`, `styles.css`, and `app.js` implement overview, agents, detail, command, activity, evidence/review/validation, goals, settings, and debug details. | Complete |
| Screenshot review and iteration | Browser screenshots captured; first status-chip layout issue fixed. | Complete |
| Agent detail and steering | Browser smoke selects an agent and queues a mocked `steer_agent` command. | Complete |
| Goal/story view | Goals tab renders fixture goal/story data and has screenshot evidence. | Complete |
| Responsive/accessibility pass | CSS breakpoints, focus states, labels, no viewport font scaling, desktop/mobile screenshots. | Complete |
| End-to-end browser smoke | `node web/dashboard/scripts/browser-smoke.mjs` opens the dashboard, interacts with controls, and captures screenshots. | Complete |
| Final design review | Residual risks documented in `knowledge.md`. | Complete |
| Workpad handoff | `TASKS.md`, `WORKING.md`, `WORKPADS.md`, README, tasks, knowledge, and references updated. | Complete |

## Verification Commands

```sh
node web/dashboard/scripts/verify.mjs
node web/dashboard/scripts/browser-smoke.mjs
git diff --check
```

## Screenshot Evidence

- `workpads/dashboard-webclient/screenshots/dashboard-desktop.png`
- `workpads/dashboard-webclient/screenshots/dashboard-detail-command.png`
- `workpads/dashboard-webclient/screenshots/dashboard-goals.png`
- `workpads/dashboard-webclient/screenshots/dashboard-mobile.png`

## Handoff

Next dashboard work should add a live Capo-server HTTP/query adapter, preserve
the same client/input boundary, and expand goal/story views once
goal-orchestration read models exist.
