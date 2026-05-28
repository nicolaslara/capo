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

## Open Questions

- Should the first webclient stack be Rust-served static assets, a separate
  TypeScript app, or a Rust/WASM-oriented frontend?
- Should the first browser surface be local-only loopback, or should remote
  access be designed from day one but disabled?
- How much live streaming is needed for the first usable slice versus polling?
- Should goal/story views wait until goal-orchestration implementation, or use
  fixtures to validate design earlier?
- What screenshot artifact location should be standardized for visual QA?
