# Dogfood References

Record dogfood migration references and evidence here.

## D1 Import Capo Workpads

Observed 2026-05-26.

- Workpad indexer: `../../crates/capo-workpads/src/lib.rs`
- Workpad CLI import/proposal/apply surface: `../../crates/capo-cli/src/workpad.rs`
- Regression coverage: `../../crates/capo-cli/src/tests.rs`
- Top-level queue/source docs indexed by the curated path set: `../../TASKS.md`, `../../project.md`
- Live repo smoke with temporary state: `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` returned `files=44`, `tasks=211`.
- Live repo smoke with temporary state: `capo workpad next --path workpads/dogfood/tasks.md --state <tmp>` selected `workpads:dogfood:tasks.md#d1`.
- No new third-party dependencies were added for D1.
