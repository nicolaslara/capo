# Dashboard Webclient Tasks

## Objective

Build a browser dashboard/web client for Capo that gives a human operator a
clear, inspectable, and good-looking view of projects, agents, sessions, goals,
evidence, validation, reviews, and execution history.

This workpad is intentionally design-gated. The implementation should not start
until a design brief is written, reviewed, revised, and accepted. After
implementation starts, each meaningful UI slice requires screenshot review and
iteration until the interface works and looks good on desktop and mobile.

## Status

Planned. Created on 2026-05-28.

The existing `workpads/features/dashboard.md` is complete and covers shared
query/text-dashboard work. This workpad is the browser client and visual product
surface.

## Product Principles

- Build the actual operator app as the first screen, not a landing page.
- Treat the browser UI as a client/input surface. It must not own orchestration
  state or read persistence internals directly.
- Prefer dense, operational layouts over decorative marketing composition.
- Keep normal views human-readable; keep raw IDs and low-level metadata behind
  details/debug surfaces.
- Use visual evidence. Every implemented UI slice needs screenshots and at
  least one iteration pass when issues are visible.
- Keep desktop and mobile usable. Text must not overlap, truncate badly, or
  make controls shift unexpectedly.

## DWC0 - Workpad And Routing

Status: completed on 2026-05-28.

Acceptance:

- Create a dedicated dashboard/webclient workpad instead of reopening the
  completed shared dashboard feature.
- Add it to the project queue after `goal-orchestration` by default.
- Add load lists and rules to `workpads/WORKPADS.md`.
- Update project routing docs so future agents can discover the workpad.
- Record design/review/screenshot iteration as mandatory workflow.

Evidence:

- `TASKS.md`
- `workpads/WORKPADS.md`
- `AGENTS.md`
- `WORKING.md`
- `workpads/dashboard-webclient/tasks.md`
- `workpads/dashboard-webclient/knowledge.md`
- `workpads/dashboard-webclient/references.md`

## DWC1 - Product Brief And User Workflows

Status: pending.

Acceptance:

- Define the target operator personas and use cases:
  - local developer supervising one or more agents;
  - reviewer auditing what happened;
  - operator recovering a stalled or blocked run;
  - future remote/mobile operator.
- Define the first dashboard workflows:
  - understand current project health;
  - inspect active agents and sessions;
  - attach/steer/stop/interrupt through server commands;
  - inspect recent activity, tools, evidence, validation, and review status;
  - understand goal progress and historical execution story once available.
- Define non-goals for v0 webclient.
- Decide which workflows can use current server/query data and which depend on
  goal-orchestration.

Verification:

- Product brief recorded in `knowledge.md` or a linked design doc.
- Review notes identify missing/ambiguous workflows.
- `git diff --check`.

## DWC2 - Information Architecture And View Map

Status: pending.

Acceptance:

- Define top-level navigation and routes.
- Define view hierarchy for:
  - project overview;
  - agents;
  - sessions/runs;
  - goals;
  - evidence;
  - reviews/validation;
  - historical reports;
  - settings/debug details.
- Define empty/loading/error/offline states for each first-slice view.
- Define which details are shown by default and which are hidden behind
  inspect/debug controls.

Verification:

- IA diagram or structured route map recorded.
- Review confirms views map to server/query contracts rather than frontend-only
  state.

## DWC3 - Visual Design Brief

Status: pending.

Acceptance:

- Define visual direction for an operational agent-control dashboard:
  restrained, dense, readable, scan-friendly, and not marketing-like.
- Define layout principles for desktop and mobile.
- Define typography, spacing, color roles, status/severity semantics, and icon
  usage.
- Define core components:
  - app shell;
  - nav/sidebar;
  - status strip;
  - agent list/table;
  - session detail;
  - timeline/story view;
  - evidence/review/validation cards or rows;
  - command/steering panel;
  - details/debug drawer.
- Include accessibility constraints: contrast, focus states, keyboard
  navigation, reduced motion, and readable tap targets.

Verification:

- Design brief reviewed against frontend guidance and Capo product principles.
- User-sensitive design decisions listed for acceptance.

## DWC4 - Design Review And Acceptance Gate

Status: pending.

Acceptance:

- Review DWC1-DWC3 before implementation.
- Identify design risks and missing states.
- Revise the design brief and view map based on review.
- Mark design accepted only when:
  - first-slice user workflows are clear;
  - views have data ownership and error states;
  - visual direction is specific enough to implement;
  - screenshot QA plan is defined.

Verification:

- Review notes and acceptance decision recorded in `knowledge.md`.
- Any rejected feedback is recorded with rationale.

## DWC5 - Webclient Stack And App Boundary

Status: pending.

Acceptance:

- Choose the first webclient stack based on repository fit and maintainability.
- Decide where the webclient lives in the repo.
- Define how the webclient talks to the Capo server/query surface.
- Define dev-server command, build command, test command, and screenshot command.
- Add only the minimal scaffold needed for the accepted design's first slice.
- Keep the server/controller state boundary intact.

Verification:

- Stack decision and commands recorded.
- New dependencies checked for license and current upstream status.
- Scaffold builds/runs.

## DWC6 - API/Query Contract For Web Views

Status: pending.

Acceptance:

- Define the read APIs needed by the first web views.
- Prefer existing shared query contracts before adding new server endpoints.
- Add typed DTOs/view models for frontend consumption.
- Define streaming or polling strategy for active agent/session updates.
- Define failure behavior for stale server, missing state, and permission errors.

Verification:

- Contract tests or fixture snapshots.
- Webclient can render fixture data without a live provider.

## DWC7 - First Usable Dashboard Slice

Status: pending.

Acceptance:

- Implement the first vertical slice:
  - project overview;
  - active agent/session list;
  - status/readiness summary;
  - recent activity;
  - evidence/review/validation summaries when data exists;
  - details/debug affordance.
- Use fixture or mocked server data first, then live local server data if
  available.
- Avoid decorative empty cards; every first-screen element should help the
  operator scan state or act.

Verification:

- Unit/component tests where applicable.
- Manual smoke through local dev server.
- Desktop and mobile screenshots captured.

## DWC8 - Screenshot Review And Visual Iteration Loop

Status: pending.

Acceptance:

- Review screenshots at minimum desktop and mobile viewports.
- Check for:
  - blank or broken render;
  - overlapping text;
  - awkward truncation;
  - unclear hierarchy;
  - unreadable contrast;
  - excessive empty space;
  - one-note palette;
  - controls that resize/shift unexpectedly;
  - missing loading/error/empty states.
- Record findings and iterate until the slice works and looks good.
- Preserve screenshot artifacts or paths as evidence.

Verification:

- Before/after screenshots recorded.
- Visual issues either fixed or explicitly deferred with rationale.

## DWC9 - Agent Detail And Steering View

Status: pending.

Acceptance:

- Implement an agent/session detail view with:
  - current status;
  - latest reply/result;
  - recent timeline;
  - tool activity;
  - evidence/review needs;
  - command/steering panel;
  - interrupt/stop controls with confirmations.
- All mutations go through Capo server commands.
- Hide raw IDs and debug metadata behind a details drawer.

Verification:

- Mutations tested against mocked/local server paths.
- Screenshot review/iteration for detail view.

## DWC10 - Goal, Story, And Historical Report Views

Status: pending.

Acceptance:

- After goal-orchestration read models exist, implement goal/story/report views:
  - goal status and requirements;
  - intent/progress timeline;
  - evidence ledger;
  - validation/review status;
  - blockers and confidence;
  - historical execution report.
- Render missing goal data gracefully when the server is older or the feature is
  disabled.

Verification:

- Fixture coverage for complete, blocked, partial, contradicted, and redacted
  goal stories.
- Screenshot review/iteration.

## DWC11 - Responsive And Accessibility Pass

Status: pending.

Acceptance:

- Verify desktop, tablet, and mobile layouts.
- Verify keyboard navigation for primary workflows.
- Verify focus states, labels, contrast, and tap targets.
- Avoid viewport-width font scaling; use stable responsive layout constraints.
- Ensure controls and repeated items have stable dimensions and no layout shift.

Verification:

- Browser/screenshot evidence across target viewports.
- Accessibility checklist recorded.

## DWC12 - End-To-End Browser Smoke

Status: pending.

Acceptance:

- Start the Capo server and webclient dev server.
- Load the dashboard in a browser.
- Exercise the first supported workflow end to end:
  - view project state;
  - inspect an agent/session;
  - send/steer a message or use a safe mocked mutation;
  - inspect updated recent activity/evidence;
  - open details/debug view.
- No raw secrets, provider tokens, or sensitive transcripts should be exposed.

Verification:

- Browser automation or manual transcript with screenshots.
- Server/webclient commands recorded.
- Bugs filed or fixed before completion.

## DWC13 - Design Review Gate Before Completion

Status: pending.

Acceptance:

- Perform a final design review after implementation and screenshot iteration.
- Compare final UI against the accepted design brief.
- Confirm any deviations are intentional and recorded.
- Confirm the UI is usable enough for the next dogfood/control pass.

Verification:

- Review notes recorded.
- Residual UX risks and follow-up tasks documented.

## DWC14 - Workpad Gate And Handoff

Status: pending.

Acceptance:

- Run final verification commands:
  - webclient format/lint/test/build commands;
  - relevant Rust/server tests if API/query code changed;
  - screenshot/browser smoke;
  - `git diff --check`.
- Record the final URL/dev command and screenshot artifact paths.
- Decide whether next work should deepen webclient features, move to
  goal-orchestration UI, or harden deployment/packaging.
- Mark the workpad complete only after design, implementation, review, and
  visual QA evidence are present.

Verification:

- Final verification output recorded in this workpad.
- `workpads/dashboard-webclient/knowledge.md` updated with decisions and known
  gaps.
