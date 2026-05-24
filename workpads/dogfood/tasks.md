# Dogfood Tasks

## Objective

Move Capo project execution into Capo only after the prototype can safely track sessions, preserve state, expose evidence, and keep markdown workpads as an auditable fallback.

Dogfood work starts after the prototype gate.

## D0 - Dogfood Readiness Review

Status: pending

Acceptance:

- Prototype evidence reviewed.
- Risks of moving project execution into Capo listed.
- Rollback/fallback plan recorded.

## D1 - Import Capo Workpads

Status: pending

Acceptance:

- Capo can index or import `TASKS.md`, `project.md`, and `workpads/`.
- No destructive writes without explicit confirmation.
- Markdown remains the fallback source of truth.

## D2 - Run First Capo-Managed Task

Status: pending

Acceptance:

- A real Capo project task is created, assigned, tracked, and reviewed through Capo.
- Evidence is recorded both in Capo state and markdown fallback.

## D3 - Dogfood Gate

Status: pending

Acceptance:

- Decision recorded in `knowledge.md`.
- Remaining manual workflow parts listed.
