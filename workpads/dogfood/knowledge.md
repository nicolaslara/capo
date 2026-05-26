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
