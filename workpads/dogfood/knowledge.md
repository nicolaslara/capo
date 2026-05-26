# Dogfood Knowledge

## Objective

Record the evidence, risks, migration decisions, and rollback plan for using Capo to manage its own development.

First dogfood readiness checkpoint passed on 2026-05-26. The project has not fully migrated into Capo-managed execution yet.

## Initial Rule

Do not move project execution into Capo until prototype state recovery, inspection, interruption, and markdown fallback are proven.

## Gate Evidence

Prototype gate passed with constraints on 2026-05-25.

What is proven:

- Capo can track two fake agents/sessions, send work, inspect read models, redirect, interrupt/stop, recover after restart, and export markdown evidence.
- The text dashboard is sufficient for first dogfood inspection.
- Evidence export refuses to overwrite non-Capo markdown files, preserving the human-authored fallback.

Historical dogfood constraints:

- Do not claim real coding-agent dogfood until at least one opt-in Codex local smoke is recorded through the real connector harness.
- Do not let Capo modify source workpads until workpad indexing/import and reviewed update artifacts are implemented.
- Keep markdown files as the source of truth until D1 proves import/index idempotency and non-destructive write behavior.

Current checkpoint evidence:

- Codex real local smoke proof exists: `adapter-smoke-codex_exec-b2e582887f9c0820`, with passed status, clean credential scan, marker present, and `real_agent_connector_proven`.
- Local Capo state has an available runtime target: `local-capo`.
- Workpad indexing recorded `files=44` and `tasks=206`.
- The next indexed workpad target is `workpads:features:agent-connectors.md#ac3`, `AC3 - Real-Agent Controller Path`.
- Capo recorded a prompt-redacted Codex workpad dispatch plan: `adapter-dispatch-plan-codex_exec-2e26cf61ba2310e8-7463adb44145eaaf`.
- Capo recorded a ready dispatch gate: `adapter-dispatch-gate-c1e061786ffdd9e6-e76900880ff59e82`.
- Capo replayed the deterministic Codex fixture through the dispatch chain: `adapter-dispatch-replay-402f3ecd6c003a86`, with `provider_cli_executed=false`, `raw_content_policy=content_hashed_not_rendered`, `appended_events=6`, and `tool_events=2`.
- Capo completed a bounded real Codex dispatch proof through the same dispatch chain: `adapter-dispatch-plan-codex_exec-b030193b63cd8c74-5600a749443fe93a` with execution `adapter-dispatch-execution-90ae27de1dd522ae-3390880dca5e76be`, `status=exited`, `exit_code=0`, `credential_scan_status=clean`, and `adapter_stream_ingested=true`.
- `capo dogfood readiness` reports `ready=true`, `status=ready_for_first_dogfood`, all component booleans true, no blockers, and no next actions.
- Project readiness evidence refs now include `evidence-artifact-dogfood-readiness-16a4b2dc9529b580` and `evidence-artifact-dogfood-readiness-38c286e1f2e30354`; these artifacts are local ignored runtime evidence, not committed source.
- `capo dashboard` renders the runtime target, connector proof, workpad index rows, dispatch plan/gate/replay, observed-only adapter-native tool activity, and `project_dogfood_readiness=true`.
- Provider-secret-shaped marker scans over local `.capo-dev` state/evidence returned no matches; raw `Codex fixture response` text was not retained in state/evidence, and the real proof dispatch evidence is prompt-redacted.

Risks before full migration:

- The green checkpoint now includes a real provider proof dispatch, but it still is not a full Capo-managed project task run.
- Source markdown is still the human-authored task authority; Capo has indexed the queue but has not yet imported AC3 as the execution authority or edited source workpads.
- The dashboard exposes enough text-state for first dogfood, but there is no richer web/mobile surface yet.
- Runtime target readiness proves an available local placement record, not a live long-running Capo server or remote tunnel.
- The local state under `.capo-dev/` is intentionally ignored and should be treated as runtime evidence, not a durable repository artifact.

Rollback/fallback plan:

- Keep using `TASKS.md`, workpad markdown, and git commits as the source of truth until D2 proves a Capo-managed task can be created, tracked, reviewed, and reflected back into markdown evidence.
- If Capo state becomes confusing or stale, ignore or delete `.capo-dev/` and rebuild from committed workpads plus the existing connector smoke evidence.
- Do not allow Capo source-writeback to markdown without reviewed proposal artifacts, source-hash checks, and explicit confirmation.
- Real provider dispatch remains opt-in and should keep prompt/output redaction plus artifact scanning.

Recommended first dogfood path:

- D0 can now close using the checkpoint above.
- D1 should import or otherwise bind the next Capo workpad task into Capo while preserving markdown fallback.
- D2 should run the first Capo-managed project task. Start with the proven Codex dispatch path, keep explicit opt-in for real provider execution, and export reviewed evidence back to the markdown fallback.

## D1 - Import Capo Workpads

Status: completed on 2026-05-26.

Decision:

- Treat D1 as the non-destructive import/index gate, not source writeback. Capo can observe and bind project workpad refs, while markdown and git remain the source of truth.
- Keep the indexed file set curated: top-level `TASKS.md`, top-level `project.md`, and selected Capo-owned workpad markdown. This avoids recursively ingesting scratch clones or unrelated references.
- Preserve `content_hash`, objective text, source path, and source anchor in read models so later imports and proposal artifacts can detect drift before mutating or dispatching work.
- Keep writeback disabled. Proposal artifacts and confirmed apply remain guarded; even confirmed apply reports `workpad_apply_supported=false` and `source_modified=false`.

Evidence:

- `capo-workpads::index_project_workpads` selects `TASKS.md`, `project.md`, and curated `workpads/` markdown.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed with top-level `TASKS.md`/`project.md` file assertions and a top-level `TASKS.md#f2` import using expected hash.
- Live repo smoke with temporary state: `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` returned `files=44`, `tasks=211`.
- Live repo smoke with temporary state: `capo workpad next --path workpads/dogfood/tasks.md --state <tmp>` selected `workpads:dogfood:tasks.md#d1`.

Next:

- D2 should create and track the first Capo-managed project task through Capo state while exporting enough markdown evidence for the fallback workflow.

## D2 - Run First Capo-Managed Task

Status: completed on 2026-05-26.

Decision:

- Count the first D2 pass as a fake-agent dogfood rehearsal. It proves Capo can bind a real Capo project workpad task into Capo state, run the controller lifecycle, record review/evaluation artifacts, and preserve markdown fallback. It does not claim full real-agent dogfood.
- Close the rehearsal by stopping the fake-agent session before task-outcome export. Outcome reports intentionally require completed, interrupted, or otherwise closed runs.
- Keep source workpad markdown unchanged during the rehearsal. The fallback evidence is exported as Capo-owned markdown artifacts and summarized in committed workpad notes.

Evidence:

- Regression `dogfood_rehearsal_tracks_capo_managed_task_and_markdown_evidence` creates a project workpad D2 task, starts it through `capo workpad start-next`, stops the fake-agent session, records a no-blockers review, exports session evidence, exports a task outcome report, verifies dashboard visibility, and checks source markdown remains unchanged.
- Live repo smoke with temporary state selected `workpads:dogfood:tasks.md#d2`, created task `task-workpad-workpads-dogfood-tasks-md-d2`, ran `session-dogfood-rehearsal`, stopped it, recorded review `review-finding-d8179ee3d36000bd`, exported `artifact-task-outcome-313ed4c2f4ccd1f6`, and dashboard rendered `review_findings=1` and `task_outcome_reports=1`.

Risk:

- The provider path was not used for this D2 pass. Real Codex dispatch is already proven separately, but this rehearsal only proves the Capo-managed project-task workflow with a fake agent.

Next:

- D3 should decide whether the current fake-agent rehearsal is enough to keep iterating inside Capo, and list any remaining manual workflow pieces before calling the dogfood gate complete.
