# Dogfood Bridge Feature

## Objective

Make Capo able to read and track its own project workpads while preserving markdown files as the human-auditable source of truth and preventing destructive writes.

## Prototype Inputs

- P11 exports Capo-owned workpad-like evidence without corrupting existing markdown.
- P12 proves state recovery, redirect, interrupt, and evidence refs for two fake sessions.
- The prototype gate passed with the gap that Capo cannot yet import/index the project workpad tree as first-class work.

## Dependencies

- Use SQLite for operational task/session state.
- Treat `TASKS.md`, `project.md`, and `workpads/**` as human-authored source files unless Capo writes a clearly marked artifact.

## Tasks

### DB1 - Workpad Index

Status: pending

Acceptance:

- Index `TASKS.md`, `project.md`, and selected `workpads/**` files into Capo-readable workpad refs.
- Store paths, hashes, headings, objective text, task IDs/statuses, and observed timestamps.
- Do not modify source markdown.

### DB2 - Capo Task Import

Status: pending

Acceptance:

- Convert selected workpad tasks into Capo task records with source anchors.
- Preserve distinction between observed markdown status and Capo execution status.
- Re-indexing is idempotent and detects source drift.

### DB3 - Reviewed Workpad Artifacts

Status: pending

Acceptance:

- Write Capo-owned evidence/update proposal artifacts without overwriting user-authored files.
- Require explicit confirmation before applying changes to source workpads.
- Provide rollback/fallback instructions for first dogfood.
