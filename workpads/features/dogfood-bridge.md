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

Status: completed

Acceptance:

- Index `TASKS.md`, `project.md`, and selected `workpads/**` files into Capo-readable workpad refs.
- Store paths, hashes, headings, objective text, task IDs/statuses, and observed timestamps.
- Do not modify source markdown.

Evidence:

- `crates/capo-workpads/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-workpads`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`

Decision:

- Add `capo-workpads` as a non-destructive markdown observation crate. It reads markdown and returns observed refs; it does not write source workpads or claim ownership of markdown status.
- Add SQLite projections for `workpad_files` and `workpad_tasks`, fed by a durable `workpad.indexed` event.
- Store `observed_status` separately from `capo_execution_status`, initialized as `observed_only`, so later imports can distinguish markdown truth from Capo execution state.
- Expose the first operator command as `capo workpad index --root <project> --state <state>`.
- Scope indexing to Capo-owned project/workpad docs and direct finding/feature files; do not recurse into `workpads/references/repos/**` or prior-art clone markdown.
- Clear prior workpad projections for the project at the start of each index projection batch so deleted or removed markdown tasks do not remain current after rebuild.
- Accept mixed-case task IDs such as `A2a`, `A5a`, and `R2a`.

Review:

- Focused review found three blockers in the first draft: over-indexing prior-art repos, missing mixed-case task IDs, and stale projection risk. All were fixed before completion.

### DB2 - Capo Task Import

Status: completed

Acceptance:

- Convert selected workpad tasks into Capo task records with source anchors.
- Preserve distinction between observed markdown status and Capo execution status.
- Re-indexing is idempotent and detects source drift.

Evidence:

- `crates/capo-core/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`

Decision:

- Add `capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID]` as the first bridge from observed markdown work into executable Capo task records.
- Keep `observed_status` on `workpad_tasks` as the markdown source observation and set the imported Capo task read model to `capo_execution_status=ready`.
- Mark the imported source workpad task with `capo_execution_status=imported` so operators can distinguish observed-only work from work that Capo is now tracking.
- Store source path, source anchor, content hash, observed status, and workpad task ID in the imported task summary and event payload until DB3 adds richer Capo-owned artifacts.
- Preserve imported workpad execution status across re-indexes for tasks still present in markdown, while allowing reset/re-index to remove stale source refs.
- Use optional `--expected-hash` as the drift guard. Imports fail with `source drift detected` when the caller imported against an old observed file hash.

Review:

- Focused review found two blockers in the first draft: repeated source fingerprints could no-op projection reset/reapply, and `--task` could overwrite an existing Capo task read model. Both were fixed before completion.

### DB3 - Reviewed Workpad Artifacts

Status: pending

Acceptance:

- Write Capo-owned evidence/update proposal artifacts without overwriting user-authored files.
- Require explicit confirmation before applying changes to source workpads.
- Provide rollback/fallback instructions for first dogfood.
