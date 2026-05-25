# Feature Knowledge

## Objective

Capture cross-feature decisions and sequencing until individual feature workpads exist.

Feature phase is ready after prototype gate P15.

## F0 - Split Feature Workpads

Status: completed on 2026-05-25.

Decisions:

- Split the post-prototype backlog by boundary rather than by UI milestone. This keeps real-agent connectors, workpad dogfood, dashboard/query, permissions/tools, memory/eval, voice, and remote runtime independently reviewable.
- The first feature priority should be either `agent-connectors.md` if the goal is real Codex/Claude execution, or `dogfood-bridge.md` if the goal is importing Capo's own workpads before real-agent execution.
- Real local agent execution remains the main product constraint from the prototype gate. Fake agents prove controller/state/evidence semantics, not useful coding output.
- Workpad import/update safety is the main dogfood bridge constraint. Evidence export is safe, but Capo cannot yet manage source workpad files directly.
- Dashboard and voice should share a reusable query surface before adding richer UI or conversational clients.

Follow-up:

- `agent-connectors.md` should start with Codex opt-in smoke because Codex is already wired through restrictive smoke-plan code.
- `dogfood-bridge.md` should preserve the source-of-truth distinction between markdown task status and Capo execution status.

## F2/DB1 - Workpad Index

Status: completed on 2026-05-25.

Decisions:

- Start the dogfood bridge with read-only indexing rather than task execution. Capo can now observe the project workpad tree without mutating source markdown.
- Add `capo-workpads` for markdown scanning and task-status extraction. The crate has no third-party dependencies and deliberately writes nothing.
- Persist workpad observations through SQLite projections:
  - `workpad_files`: path, project, content hash, headings, objective text, observed timestamp, update sequence.
  - `workpad_tasks`: source task ID, project, path, source anchor, title, observed markdown status, Capo execution status, observed timestamp, update sequence.
- Use `observed_only` as the initial Capo execution status for imported markdown tasks. DB2 will decide how selected observed tasks become executable Capo tasks.
- Add `capo workpad index --root PATH --state PATH` as the first non-destructive CLI path for dogfood import.
- After review, constrain the scanner to selected Capo workpad docs rather than recursive `workpads/**` indexing. This prevents prior-art clones and reference repos from becoming Capo task refs.
- Add mixed-case task ID support for headings like `A2a`, `A5a`, and `R2a`.
- Add a reset projection at the start of each workpad index batch so rebuild and re-index remove stale file/task refs.

Verification:

- `cargo test -p capo-workpads`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.
- Manual repo smoke `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` reported `files=43`, `tasks=98` after scoping fixes.

Review:

- Focused review initially blocked DB1 on over-indexing, missed mixed-case task IDs, and stale projections. The fixes above were applied before completion.

Follow-up:

- DB2 should map selected `workpad_tasks` into Capo task records while preserving markdown status as observed source truth.
- DB3 should add reviewed update/evidence proposal artifacts before Capo can apply any changes to source workpads.
