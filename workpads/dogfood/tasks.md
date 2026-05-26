# Dogfood Tasks

## Objective

Move Capo project execution into Capo only after the prototype can safely track sessions, preserve state, expose evidence, and keep markdown workpads as an auditable fallback.

Dogfood work starts after the prototype gate.

Prototype gate passed with constraints on 2026-05-25. Dogfood planning can start, but running real Capo project work through Capo still depends on real local connector proof and workpad import/update safety.

## D0 - Dogfood Readiness Review

Status: completed

Acceptance:

- Prototype evidence reviewed.
- Risks of moving project execution into Capo listed.
- Rollback/fallback plan recorded.
- Decide whether first rehearsal uses fake agents or waits for Codex connector proof.

Evidence:

- `workpads/dogfood/knowledge.md` records the first dogfood readiness checkpoint, risks, rollback/fallback plan, and recommended first dogfood path.
- `capo dogfood readiness`: `ready=true`, `status=ready_for_first_dogfood`, no blockers, no next actions.
- First rehearsal can use the proven Codex connector path. The dispatch chain has both deterministic fixture replay evidence and a bounded real Codex proof execution with clean artifact scanning.

## D1 - Import Capo Workpads

Status: completed

Acceptance:

- Capo can index or import `TASKS.md`, `project.md`, and `workpads/`.
- No destructive writes without explicit confirmation.
- Markdown remains the fallback source of truth.
- Source anchors and file hashes are stored so re-import detects drift.

Evidence:

- `capo-workpads::index_project_workpads` selects `TASKS.md`, `project.md`, and the curated `workpads/` markdown set without writing source files.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed. The regression covers top-level `TASKS.md` and `project.md` file refs with objective/hash data, top-level `TASKS.md` task import, source-hash drift checks, guarded proposal/apply behavior, and unchanged source markdown.
- Live repo smoke with temporary state: `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` returned `files=44`, `tasks=211`; `capo workpad next --path workpads/dogfood/tasks.md --state <tmp>` selected `workpads:dogfood:tasks.md#d1`.

## D2 - Run First Capo-Managed Task

Status: pending

Acceptance:

- A real Capo project task is created, assigned, tracked, and reviewed through Capo.
- Evidence is recorded both in Capo state and markdown fallback.
- If real provider execution is not used, label the run as a fixture or fake-agent rehearsal rather than full real-agent dogfood.

## D3 - Dogfood Gate

Status: pending

Acceptance:

- Decision recorded in `knowledge.md`.
- Remaining manual workflow parts listed.
