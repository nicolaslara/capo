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

## D2 First Capo-Managed Task Rehearsal

Observed 2026-05-26.

- Workpad start-next command: `../../crates/capo-cli/src/workpad.rs`
- Review/evidence/outcome command surface: `../../crates/capo-cli/src/evidence.rs`
- Regression coverage: `../../crates/capo-cli/src/tests.rs`
- Live repo smoke with temporary state selected `workpads:dogfood:tasks.md#d2` and created `task-workpad-workpads-dogfood-tasks-md-d2`.
- Live repo smoke session: `session-dogfood-rehearsal`; run: `run-dogfood-rehearsal`.
- Live repo smoke review artifact ID: `artifact-review-finding-d8179ee3d36000bd`.
- Live repo smoke task-outcome artifact ID: `artifact-task-outcome-313ed4c2f4ccd1f6`.
- The D2 pass is explicitly a fake-agent rehearsal, not full real-agent dogfood.
- No new third-party dependencies were added for D2.

## D3 Dogfood Gate

Observed 2026-05-26.

- Dogfood gate decision: `../../workpads/dogfood/knowledge.md`
- Dogfood task ledger: `../../workpads/dogfood/tasks.md`
- Top-level workpad queue closure: `../../TASKS.md`
- Phase routing update: `../../workpads/WORKPADS.md`
- D3 is documentation and gate-state only; no new third-party dependencies were added.
