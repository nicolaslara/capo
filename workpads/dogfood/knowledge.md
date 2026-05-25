# Dogfood Knowledge

## Objective

Record the evidence, risks, migration decisions, and rollback plan for using Capo to manage its own development.

Dogfood gate not passed.

## Initial Rule

Do not move project execution into Capo until prototype state recovery, inspection, interruption, and markdown fallback are proven.

## Gate Evidence

Prototype gate passed with constraints on 2026-05-25.

What is proven:

- Capo can track two fake agents/sessions, send work, inspect read models, redirect, interrupt/stop, recover after restart, and export markdown evidence.
- The text dashboard is sufficient for first dogfood inspection.
- Evidence export refuses to overwrite non-Capo markdown files, preserving the human-authored fallback.

Dogfood constraints:

- Do not claim real coding-agent dogfood until at least one opt-in Codex local smoke is recorded through the real connector harness.
- Do not let Capo modify source workpads until workpad indexing/import and reviewed update artifacts are implemented.
- Keep markdown files as the source of truth until D1 proves import/index idempotency and non-destructive write behavior.

Recommended first dogfood path:

- D0 can start after the prototype gate to formalize the migration and rollback plan.
- D1 should depend on `workpads/features/dogfood-bridge.md`.
- D2 should depend on either a real Codex connector proof or explicitly run only as a fake-agent workpad-management rehearsal.
