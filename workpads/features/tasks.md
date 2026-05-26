# Feature Tasks

## Objective

Turn the prototype gate findings into independently executable feature work, while keeping each feature behind explicit dependencies, evidence standards, and review gates.

Feature work starts after the prototype gate. Dedicated feature files hold the detailed backlogs; this file is the routing index.

## Feature Workpads

| Workpad | Focus | First dependency |
| --- | --- | --- |
| `agent-connectors.md` | Real local Codex/Claude connector proof | Prototype P7 harness |
| `dogfood-bridge.md` | Import/index Capo workpads and write reviewed evidence/update artifacts | Prototype P11/P12 evidence export |
| `dashboard.md` | Reusable query surface plus richer TUI/web dashboard | Prototype P13 text dashboard |
| `permissions-tools.md` | Capability profile hardening, approval policy variants, tool wrappers | Prototype P8 audit path |
| `memory-eval.md` | Source-linked memory records, performance reports, review outcomes | Prototype P9/P11 |
| `voice.md` | Conversational Capo loop from P14 contract | Prototype P14 |
| `remote-runtime.md` | Remote runtime/tunnel adapters | Local real-agent semantics |
| `state-store.md` | State persistence resilience and ORM/typed projection strategy | Prototype P2 state store |

## F0 - Split Feature Workpads

Status: completed

Acceptance:

- Each selected feature has its own workpad or clearly scoped section.
- Dependencies and gates are recorded.
- Prototype learnings are reflected in the task order.

Evidence:

- `workpads/prototype/knowledge.md` Prototype Gate section
- `workpads/features/agent-connectors.md`
- `workpads/features/dogfood-bridge.md`
- `workpads/features/dashboard.md`
- `workpads/features/permissions-tools.md`
- `workpads/features/memory-eval.md`
- `workpads/features/voice.md`
- `workpads/features/remote-runtime.md`

## F1 - Real Local Agent Connector Proof

Status: in_progress

Source workpad: `agent-connectors.md`

Acceptance:

- Run the opt-in Codex local smoke through the existing restrictive harness or record why it cannot safely run.
- Verify no credential/session material is read, persisted, or exported.
- Decide whether Claude Code smoke is ready or needs a separate restricted-CLI compatibility slice.

Progress:

- AC1 Codex smoke is waiting on explicit user opt-in. The local Codex CLI exists and the non-secret harness/preflight tests pass, but the real subscription-backed process has not been run.
- AC2 Claude Code restricted-args verification is completed for installed `claude 2.1.150`.
- AC3 deterministic normalized adapter replay through controller/state is completed for Codex and Claude fixtures, but the real-agent controller path remains pending until at least one real local adapter stream is run.
- AC4 connector readiness surface is completed. `capo adapter readiness` reports configured Codex/Claude opt-in gates and smoke-plan safety metadata without launching provider CLIs or inspecting credentials.
- AC5 durable connector readiness state is completed. `capo adapter readiness --record` persists readiness rows and the dashboard renders the remaining dogfood blocker.
- AC6 real smoke evidence contract is completed. `capo adapter smoke-report record` can persist skipped/failed/passed smoke reports and refuses passed reports without a clean credential scan plus expected marker.
- AC7 dogfood readiness gate is completed. `capo adapter dogfood-gate` and `capo dashboard` now derive first real-agent dogfood readiness from recorded connector evidence without launching provider CLIs.
- AC8 smoke artifact scan enforcement is completed. Passed smoke reports now require an artifact root that Capo scans for unredacted credential/session markers before recording the report.
- AC9 local adapter launch contract is completed. Codex and Claude Code now share reusable launch-plan builders that produce runtime configs/requests without launching provider CLIs.
- AC10 controller dispatch planning is completed. `capo adapter plan-launch` resolves agent intent into a safe, prompt-redacted local adapter runtime contract without executing provider CLIs.
- AC11 durable dispatch plan read model is completed. `capo adapter plan-launch --record` persists prompt-redacted dispatch plans and the dashboard renders them through the shared query surface.
- AC12 workpad next adapter plan is completed. `capo workpad plan-next` composes markdown next-task selection with prompt-redacted Codex/Claude dispatch planning without importing or starting the task.
- AC13 dispatch execution gate is completed. `capo adapter dispatch-gate` checks recorded dispatch plans against the shared real-agent dogfood gate and fails closed before any provider CLI execution.
- AC14 dispatch gate audit trail is completed. `capo adapter dispatch-gate --record` persists prompt-redacted gate decisions and dashboard/query surfaces render them without claiming provider execution.
- AC15 dispatch fixture replay is completed. `capo adapter replay-dispatch` links recorded dispatch plans and ready gates to deterministic fixture replay without launching provider CLIs or retaining raw prompt/provider text.
- AC16 dispatch replay read model is completed. Dispatch fixture replay outcomes are persisted, rebuilt, and surfaced through dashboard/query with fixture hashes, counts, session/run refs, and `provider_cli_executed=false`.
- AC17 dispatch chain status is completed. `capo adapter dispatch-status` summarizes a recorded dispatch plan, latest gate, latest replay, and next safe action from shared read models without rendering raw prompts or fixture text.
- AC18 dispatch execution request audit is completed. `capo adapter execution-request --record` persists blocked or waiting-on-opt-in real-dispatch requests separately from plans, gates, and fixture replays without launching provider CLIs.
- AC19 dispatch prompt source contract is completed. Recorded dispatch plans now get prompt-source rows that distinguish non-replayable inline prompts from hash-guarded workpad-derived prompts without storing raw prompt text.
- AC20 dispatch prompt materialization dry run is completed. `capo adapter materialize-prompt --record` verifies prompt materialization readiness without rendering prompts or launching provider CLIs.
- AC21 real dispatch runner preflight is completed. `capo adapter run-preflight` composes recorded plans, execution requests, prompt materialization, and explicit provider opt-in into one provider-free readiness check.
- AC22 guarded local dispatch runner surface is completed. `capo adapter run-local` consumes the preflight, fails closed until explicit opt-in, and only reaches `LocalProcessRunner` for hash-verified workpad prompts.
- AC23 dispatch execution outcome read model is completed. `run-local --record` now persists blocked or future executed local dispatch outcomes through shared state/query/dashboard rows.
- AC24 dispatch status execution introspection is completed. `dispatch-status` now summarizes the latest dispatch execution outcome alongside plan, gate, and replay state.
- AC25 dispatch chain evidence export is completed. `dispatch-evidence` writes a prompt-redacted Capo evidence artifact for plan/gate/replay/execution review.
- AC26 dispatch status query contract is completed. `dispatch-status` now renders a reusable `capo-query` summary instead of assembling dispatch-chain state in the CLI.
- AC27 latest dispatch status selection is completed. `dispatch-status --latest [--agent NAME]` selects the latest dispatch-chain status through the shared query contract.
- AC28 latest dispatch evidence export is completed. `dispatch-evidence --latest [--agent NAME]` exports prompt-redacted evidence for the latest dispatch chain through the shared query selector.
- AC29 dispatch tool observation evidence is completed. Dispatch evidence exports now include observed-only native tool observations recorded by fixture replay, without rendering raw prompts, provider fixture text, or tool input/output.
- AC30 adapter smoke report evidence export is completed. `smoke-report evidence` writes a prompt/output-redacted Capo evidence artifact for connector proof or blocker review.
- AC31 adapter smoke report status query is completed. `smoke-report status` exposes exact and latest connector smoke status through the shared query contract.
- AC32 latest adapter smoke evidence export is completed. `smoke-report evidence --latest` exports connector proof/blocker artifacts through the shared latest smoke selector.
- AC33 adapter dogfood gate evidence export is completed. `adapter dogfood-gate evidence` writes a connector-level gate artifact for first real-agent dogfood review.

Evidence:

- `crates/capo-adapters/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `codex --version`: `codex-cli 0.133.0`
- `codex exec --help`
- `claude --version`: `2.1.150 (Claude Code)`
- `claude --help`
- `cargo test -p capo-adapters local_smoke_plan`
- `cargo test -p capo-adapters local_adapter_smoke_runner`
- `cargo test -p capo-adapters artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets`
- `cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` without `CAPO_RUN_CODEX_LOCAL_SMOKE=1`
- `cargo test -p capo-controller replay -- --nocapture`
- `cargo test -p capo-cli adapter_fixture -- --nocapture`
- `cargo test -p capo-cli adapter_readiness -- --nocapture`
- `cargo test -p capo-state adapter_readiness -- --nocapture`
- `cargo test -p capo-state adapter_smoke -- --nocapture`
- `cargo test -p capo-cli adapter_smoke -- --nocapture`
- `cargo test -p capo-query adapter_dogfood -- --nocapture`
- `cargo test -p capo-cli adapter_dogfood -- --nocapture`
- `cargo test -p capo-cli adapter_smoke -- --nocapture`
- `cargo test -p capo-adapters launch_plan -- --nocapture`
- `cargo test -p capo-controller local_adapter_dispatch -- --nocapture`
- `cargo test -p capo-cli adapter_plan_launch -- --nocapture`
- `cargo test -p capo-state adapter_dispatch_plan -- --nocapture`
- `cargo test -p capo-query adapter_dispatch -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-state adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-state adapter_dispatch_replay -- --nocapture`
- `capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID [--state PATH]`
- `cargo test -p capo-state adapter_dispatch_execution_request -- --nocapture`
- `capo adapter execution-request --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]`
- `cargo test -p capo-state adapter_dispatch_prompt_source -- --nocapture`
- `cargo test -p capo-state adapter_dispatch_prompt_materialization -- --nocapture`
- `capo adapter materialize-prompt --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]`
- `capo adapter run-preflight --dispatch-plan DISPATCH_PLAN_ID [--state PATH]`
- `capo adapter run-local --dispatch-plan DISPATCH_PLAN_ID [--state PATH]`
- `cargo test -p capo-state adapter_dispatch_execution -- --nocapture`
- `capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID [--state PATH]`
- `capo adapter dispatch-evidence --dispatch-plan DISPATCH_PLAN_ID --out DIR [--state PATH]`
- `capo adapter dispatch-evidence --latest [--agent NAME] --out DIR [--state PATH]`
- `cargo test -p capo-query adapter_dispatch_status -- --nocapture`
- `cargo test -p capo-query latest_adapter_dispatch_status -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture` verifies dispatch evidence observed tool activity
- `capo adapter smoke-report evidence --smoke-report SMOKE_REPORT_ID --out DIR [--state PATH]`
- `cargo test -p capo-query adapter_smoke_report -- --nocapture`
- `cargo test -p capo-cli adapter_smoke -- --nocapture`
- `capo adapter smoke-report status --smoke-report SMOKE_REPORT_ID [--state PATH]`
- `capo adapter smoke-report status --latest [--adapter codex|claude] [--state PATH]`
- `capo adapter smoke-report evidence --latest [--adapter codex|claude] --out DIR [--state PATH]`
- `capo adapter dogfood-gate evidence --out DIR [--state PATH]`
- Focused F1 connector safety reviews: provider-artifact cleanup blocker found and fixed; real-agent readiness remains unclaimed pending opt-in smoke

## F2 - Workpad Dogfood Bridge

Status: completed

Source workpad: `dogfood-bridge.md`

Acceptance:

- Index/import `TASKS.md`, `project.md`, and workpad files into Capo-readable task records.
- Write Capo-owned evidence/update artifacts without corrupting user-authored markdown.
- Preserve markdown as the source-of-truth fallback.

Progress:

- DB1 workpad index is completed.
- DB2 task import is completed.
- DB3 reviewed artifacts are completed.
- DB4 next workpad selection is completed with a read-only `capo workpad next` command.
- DB5 start-next dispatch is completed with explicit import plus fake-controller dispatch while preserving markdown.
- DB6 dogfood readiness surface is completed with a shared query/CLI summary of connector, workpad, and dispatch-chain prerequisites.
- DB7 dogfood readiness evidence export is completed with a Capo-marked markdown artifact and project-level evidence record.
- DB8 dogfood readiness component refs is completed with connector, workpad, dispatch-chain, and project-evidence refs rendered through the shared readiness query.
- DB9 runtime target dogfood readiness is completed with available runtime target counts/refs included in shared readiness, dashboard, voice, and evidence surfaces.

Evidence:

- `crates/capo-workpads/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-workpads`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused review subagent: blockers found and fixed
- `capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID]`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`
- Focused DB2 review subagent: source-fingerprint recurrence and task overwrite blockers found and fixed
- `capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR [--expected-hash HASH] [--task TASK_ID] [--summary TEXT]`
- `capo workpad apply --proposal PATH --confirm`
- Manual DB3 smoke: `workpad index`, `workpad import`, `workpad propose`
- Focused DB3 review subagent: proposal overwrite/idempotency blocker found and fixed
- `capo workpad next [--path PATH]`
- `capo workpad start-next --agent NAME [--path PATH]`
- `capo dogfood readiness [--state PATH]`
- `capo dogfood readiness --out DIR [--state PATH]`
- `cargo test -p capo-query dogfood_readiness -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-query dogfood_readiness -- --nocapture`
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`
- `cargo test -p capo-query dogfood_readiness -- --nocapture` verifies runtime target readiness gating
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture` verifies runtime target refs in readiness/evidence/dashboard output
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture` verifies runtime target readiness in voice answers

## F3 - Query Surface And Dashboard Upgrade

Status: completed

Source workpad: `dashboard.md`

Acceptance:

- Extract dashboard/read-model aggregation out of `capo-cli`.
- Keep CLI/dashboard/voice/web consumers on the same query contract.
- Add richer dashboard view only after the query boundary is reusable.

Progress:

- DS1 query surface extraction is completed.
- DS2 richer operator dashboard view is completed with project/session/status filters, tool-call refs, memory-packet refs, and fail-closed filter parsing.
- DS3 workpad queue visibility is completed with shared query rows and CLI dashboard rendering.
- DS4 workpad queue filters are completed with explicit `--workpad-path` and `--workpad-status` filters.
- DS5 project evidence visibility is completed with shared query rows and CLI dashboard rendering for project-level evidence artifacts.
- DS6 dogfood readiness dashboard summary is completed with shared query computation and CLI dashboard rendering of the overall readiness verdict.
- DS7 shared next workpad selection is completed with CLI workpad next/plan/start paths routed through the shared dashboard query helper.
- DS8 shared tool activity summary is completed with compact project/agent tool counts exposed by `capo-query` and consumed by voice tool-activity rendering.
- DS9 dashboard tool activity summary is completed with project-level governed tool-call and observed-only tool-observation totals rendered in `capo dashboard`.
- DS10 dashboard dogfood readiness component refs is completed with connector, workpad, dispatch-chain, and project-evidence refs rendered in the operator dashboard.
- DS11 dashboard latest adapter smoke summary is completed with latest any/Codex/Claude smoke-report shortcuts sourced from the shared query selector.
- DS12 dashboard runtime target control readiness is completed with target-readiness rows sourced from the shared runtime target control-readiness query.

Evidence:

- `crates/capo-query/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-query`
- `cargo test -p capo-cli dashboard_rejects_malformed_filters`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`
- `cargo test -p capo-query workpad_tasks -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-query workpad -- --nocapture`
- `cargo test -p capo-cli dashboard_rejects_malformed_filters -- --nocapture`
- `cargo test -p capo-query project_dashboard_includes_project_level_evidence -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-query dogfood_readiness -- --nocapture`
- `cargo test -p capo-query next_actionable_workpad -- --nocapture`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`
- `cargo test -p capo-query project_dashboard_aggregates_agents_sessions_runs_evidence_and_events -- --nocapture`
- `cargo test -p capo-cli voice_recent_work -- --nocapture`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
- `cargo test -p capo-cli adapter_smoke -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies dashboard runtime target readiness before activation, after activation, and after revocation
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused DS1 query-boundary review: test coverage gap found and fixed
- Focused DS2 dashboard review: project-filter and malformed-filter blockers found and fixed; broad any-status filter documented as intentional v0 behavior

## F4 - Capability And Tool Hardening

Status: completed

Source workpad: `permissions-tools.md`

Acceptance:

- Add stricter policy variants beyond trusted-local allow-all.
- Expand instrumented wrappers for tools Capo can execute directly.
- Keep provider-native tools observed-only unless Capo receives structured lifecycle evidence.

Progress:

- PT1 static policy variant is completed.
- PT2 user approval queue is completed with CLI request/list/decide commands and guarded durable grant/denial mapping.
- PT3 wrapper expansion is completed with runtime/file/git/workpad wrappers and permission-bound artifact instrumentation.
- PT4 ACP client capability gating is completed with advertisement decisions derived from registered wrapper tools and the selected permission policy.
- PT5 ACP session setup capability plan is completed with adapter setup consuming the Capo tool capability gate before advertising filesystem or terminal capabilities.
- PT6 ACP client handler wrapper routing is completed with filesystem/terminal calls mapped to Capo wrapper requests only when advertised by setup.
- PT7 adapter native tool observation contract is completed with observed-only classifications for ACP, Codex, and Claude fixture tool updates.
- PT8 observed-only tool observation state projection is completed with durable append/read/rebuild coverage.
- PT9 query and evidence visibility is completed with observed-only tool observations surfaced through the shared session dashboard row and CLI/evidence views.
- PT10 adapter replay observation ingestion is completed with normalized adapter tool events automatically appending observed-only tool observation rows.
- PT11 session status tool introspection is completed with governed tool calls and observed-only tool observations rendered in per-agent status.
- PT12 git commit wrapper is completed with already-staged commits governed through the runtime wrapper/tool audit path.
- PT13 wrapper tool CLI surface is completed with an explicit provider-free `capo tool run-wrapper` operator command.
- PT14 recorded wrapper tool invocations are completed with opt-in `run-wrapper --record` persistence into artifact, session, run, and governed tool-call projections.

Evidence:

- `crates/capo-tools/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-controller/src/lib.rs`
- `cargo test -p capo-tools`
- `cargo test -p capo-controller denied_static_permission_stops_tool_invocation_in_controller_path`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused PT1 permission reviews: scope parsing, grant scoping, decision durability, denied controller execution, and permission event IDs blockers found and fixed
- `crates/capo-cli/src/main.rs`
- `crates/capo-core/src/lib.rs`
- `cargo test -p capo-state permission_approval`
- `cargo test -p capo-cli permission_approval_queue_maps_decisions_to_scoped_grants`
- Focused PT2 permission reviews: concurrent decisions, durable `allow_always`, once-grant reuse, missing grant-created audit events, and state-layer JSON validation blockers found and fixed; re-review found no blockers
- `cargo test -p capo-tools`
- `cargo test -p capo-tools acp_client_capabilities -- --nocapture`
- `cargo test -p capo-adapters acp_session_setup -- --nocapture`
- `cargo test -p capo-adapters acp_client -- --nocapture`
- `cargo test -p capo-adapters acp_terminal -- --nocapture`
- `cargo test -p capo-adapters adapter_tool_observations -- --nocapture`
- `cargo test -p capo-state tool_observations -- --nocapture`
- `cargo test -p capo-query project_dashboard_aggregates_agents_sessions_runs_evidence_and_events -- --nocapture`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`
- `cargo test -p capo-controller fixture_replay -- --nocapture`
- `cargo test -p capo-cli adapter_fixture_replay_cli_exports_evidence_without_raw_provider_text -- --nocapture`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`
- `cargo test -p capo-tools git_commit -- --nocapture`
- `cargo test -p capo-cli tool_run_wrapper -- --nocapture`
- `cargo test -p capo-cli tool_run_wrapper -- --nocapture` verifies recorded wrapper tool-call dashboard visibility
- Focused PT3 wrapper reviews: split authorization replay, arbitrary workpad reads, artifact path escaping, unredacted input artifacts, misleading permission status, same-tool replay, runtime run ID paths, and ambiguous context hashing blockers found and fixed

## F5 - Memory And Evaluation Reports

Status: completed

Source workpad: `memory-eval.md`

Acceptance:

- Promote source-linked memory records beyond packet-only evidence.
- Add outcome/performance reports for completed agent work.
- Keep provenance and review state visible in read models.

Progress:

- ME1 memory record read models are completed.
- ME2 task outcome reports are completed.
- ME3 review feedback loop is completed.
- ME4 review finding dashboard visibility is completed with project/session review findings in the shared query and CLI dashboard.
- ME5 task outcome dashboard visibility is completed with project/session outcome reports in the shared query and CLI dashboard.

Evidence:

- `crates/capo-state/src/lib.rs`
- `crates/capo-eval/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-state memory_record`
- `cargo test -p capo-state task_outcome`
- `cargo test -p capo-eval`
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`
- `cargo test -p capo-query review_findings -- --nocapture`
- `cargo test -p capo-cli dashboard_renders_review_findings -- --nocapture`
- `cargo test -p capo-query task_outcome_reports -- --nocapture`
- `cargo test -p capo-cli dashboard_renders_task_outcome_reports -- --nocapture`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused ME1 memory read-model review: replayable-source filtering and fail-closed projection decode blockers found and fixed.
- Focused ME2 report reviews: self-referential reruns, overwrite safety, review-outcome derivation, terminal-status guard, and report/artifact/event identity blockers found and fixed; final focused review found no blockers.
- Focused ME3 review feedback review: follow-up identity and unchecked link blockers found and fixed; final focused re-review found no blockers.

## F6 - Voice Control Integration

Status: completed

Source workpad: `voice.md`

Acceptance:

- Route P14 voice command plans through the controller/query/permission boundaries.
- Use dummy transcripts until retention/redaction paths are proven.
- Require visible confirmation for privileged voice actions.

Progress:

- V1 controller integration is completed.
- V2 voice permission confirmation is completed.
- V3 retention and redaction smoke is completed.
- V4 dogfood readiness conversation is completed with a read-only voice intent over the shared dashboard readiness query.
- V5 recent work conversation is completed with read-only project and agent recent-work questions over the shared dashboard query.
- V6 review needs conversation is completed with a read-only project review/outcome question over the shared dashboard query.
- V7 next work conversation is completed with a read-only project workpad-queue question over the shared dashboard query.
- V8 confirmed start-next conversation is completed with visible approval before importing and dispatching the next workpad task through the fake/local controller path.
- V9 dispatch status conversation is completed with a read-only dispatch-chain status question over `ProjectDashboard::adapter_dispatch_status(...)`.
- V10 latest dispatch status conversation is completed with read-only project and agent-scoped questions over `ProjectDashboard::latest_adapter_dispatch_status(...)`.
- V11 latest connectivity exposure conversation is completed with read-only remote-control exposure questions over `ProjectDashboard::latest_connectivity_exposure(...)`.
- V12 recent-work tool activity conversation is completed with governed tool calls and observed-only tool observations included in project/agent recent-work answers.
- V13 explicit tool activity conversation is completed with read-only project and agent-scoped questions over shared dashboard tool-call and tool-observation rows.
- V14 adapter smoke status conversation is completed with read-only exact/latest connector smoke questions over shared dashboard query selectors.
- V15 latest runtime target status conversation is completed with read-only latest/filter questions over `ProjectDashboard::latest_runtime_target(...)`.
- V16 runtime target control readiness conversation is completed with read-only target readiness questions over `ProjectDashboard::runtime_target_control_readiness(...)`.

Evidence:

- `crates/capo-voice/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-voice`
- `cargo test -p capo-cli voice -- --nocapture`
- `cargo test -p capo-voice dogfood_readiness -- --nocapture`
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`
- `cargo test -p capo-voice recent_work -- --nocapture`
- `cargo test -p capo-cli voice_recent_work -- --nocapture`
- `cargo test -p capo-voice review_needs -- --nocapture`
- `cargo test -p capo-cli voice_review_needs -- --nocapture`
- `cargo test -p capo-query next_actionable_workpad -- --nocapture`
- `cargo test -p capo-voice next_work -- --nocapture`
- `cargo test -p capo-cli voice_next_work -- --nocapture`
- `cargo test -p capo-voice start_next_work -- --nocapture`
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`
- `cargo test -p capo-voice dispatch_status -- --nocapture`
- `cargo test -p capo-voice latest_dispatch_status -- --nocapture`
- `cargo test -p capo-voice latest_connectivity -- --nocapture`
- `cargo test -p capo-cli voice_dispatch_status -- --nocapture`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`
- `cargo test -p capo-voice recent_work -- --nocapture`
- `cargo test -p capo-cli voice_recent_work -- --nocapture`
- `cargo test -p capo-voice tool_activity -- --nocapture`
- `cargo test -p capo-cli voice_recent_work -- --nocapture` verifies explicit tool activity questions
- `cargo test -p capo-voice adapter_smoke -- --nocapture`
- `cargo test -p capo-cli voice_adapter_smoke -- --nocapture`
- `cargo test -p capo-voice runtime_target -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies latest runtime target voice status
- `cargo test -p capo-voice runtime_target -- --nocapture` verifies runtime target readiness voice planning
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies runtime target readiness voice rendering
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## F7 - Remote Runtime And Tunnel

Status: in_progress

Source workpad: `remote-runtime.md`

Acceptance:

- Add a non-local runtime/tunnel adapter only after local real-agent behavior is stable.
- Keep runtime process ownership separate from connectivity exposure.
- Require explicit permission/audit for public or remote access.

Progress:

- RR1 loopback remote runtime contract is completed without Tailscale or cloud credentials.
- RR2 tunnel adapter stub is completed with endpoint resolution, health, exposure scope, and permission requirement records kept separate from runtime process refs.
- RR3 explicit exposure policy read model is completed with blocked/active/revoked exposure states, linked durable grants, and health visibility.
- RR4 dashboard exposure visibility is completed through the shared query surface and CLI dashboard rendering.
- RR5 connectivity exposure operator surface is completed with a provider-free `connectivity expose-stub` command that records blocked private/public exposure intent without opening tunnels or running agents.
- RR6 connectivity exposure approval bridge is completed with commands to queue a permission approval from a blocked exposure and activate it only after a matching durable allow grant exists.
- RR7 connectivity exposure revocation surface is completed with a command that records revoked exposure state, disabled health, and unreachable status without managing real tunnels or runtime processes.
- RR8 connectivity exposure evidence export is completed with a Capo-owned project evidence artifact for endpoint/owner/channel/scope/status/health/grant/revocation review.
- RR9 latest connectivity exposure status is completed with shared exact/latest exposure selectors and a read-only CLI status surface with owner/channel filters.
- RR10 latest connectivity exposure evidence export is completed with filtered latest-selector export through the shared connectivity exposure query.
- RR11 runtime target inventory is completed with first-class runtime target metadata persisted separately from connectivity exposures and provider dispatch plans.
- RR12 runtime target exposure validation is completed: recorded runtime-target exposures fail closed unless the target is registered.
- RR13 runtime target endpoint consistency is completed: recorded runtime-target exposures fail closed when they use a different endpoint than the target's configured endpoint.
- RR14 runtime target availability guard is completed: recorded runtime-target exposures fail closed unless the target is available.
- RR15 runtime target status update surface is completed with a provider-free `runtime target set-status` command.
- RR16 runtime target status query surface is completed with shared exact target selection and a read-only operator command.
- RR17 voice runtime target status query is completed with a read-only input intent over the shared target selector.
- RR18 runtime target evidence export is completed with a Capo-owned project evidence artifact for placement/status review.
- RR19 latest runtime target status is completed with a shared latest selector and read-only `runtime target status --latest` filters.
- RR20 latest runtime target evidence export is completed with filtered latest-selector export through the shared runtime target query.
- RR21 runtime target control readiness is completed with a shared query and read-only CLI command combining target availability with latest control exposure state.
- RR22 runtime target control readiness evidence export is completed with a Capo-owned project evidence artifact for the aggregate target/control-exposure readiness state.
- RR23 latest runtime target control readiness is completed with filtered latest-selector readiness through the shared runtime target query.
- RR24 latest runtime target control readiness evidence export is completed with filtered latest-selector export through the shared runtime target query.
- F7 remains `in_progress` until the real local-agent connector dependency is satisfied; remote execution semantics are still contract-level and loopback/stubbed.

Evidence:

- `crates/capo-runtime/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-query/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-runtime remote_runtime -- --nocapture`
- `cargo test -p capo-runtime tunnel -- --nocapture`
- `cargo test -p capo-state connectivity_exposure -- --nocapture`
- `cargo test -p capo-query connectivity -- --nocapture`
- `cargo test -p capo-cli dashboard_renders_connectivity -- --nocapture`
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`
- `capo connectivity exposure-evidence --exposure EXPOSURE_ID --out DIR [--state PATH]`
- `capo connectivity exposure-evidence --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel CHANNEL] --out DIR [--state PATH]`
- `capo connectivity exposure-status --exposure EXPOSURE_ID [--state PATH]`
- `capo connectivity exposure-status --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel CHANNEL] [--state PATH]`
- `cargo test -p capo-state runtime_targets -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture`
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture`
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture` verifies runtime target endpoint consistency
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture` verifies disabled target exposure rejection
- `cargo test -p capo-cli runtime_target -- --nocapture`
- `cargo test -p capo-cli connectivity_expose_stub -- --nocapture` verifies status update enables later exposure
- `cargo test -p capo-query runtime_target -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies exact runtime target status query
- `cargo test -p capo-voice runtime_target -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies voice runtime target status query
- `capo runtime target evidence --target TARGET_ID --out DIR [--state PATH]`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies runtime target evidence export
- `capo runtime target status --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] [--state PATH]`
- `cargo test -p capo-query runtime_target -- --nocapture`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies latest runtime target status selection
- `capo runtime target evidence --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] --out DIR [--state PATH]`
- `cargo test -p capo-cli runtime_target -- --nocapture` verifies latest runtime target evidence export
- `capo runtime target readiness --target TARGET_ID [--state PATH]`
- `cargo test -p capo-query runtime_target -- --nocapture`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies runtime target control readiness before activation, after activation, and after revocation
- `capo runtime target readiness-evidence --target TARGET_ID --out DIR [--state PATH]`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies runtime target readiness evidence export and dashboard project evidence visibility
- `capo runtime target readiness --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] [--state PATH]`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies latest runtime target readiness selection and missing-filter errors
- `capo runtime target readiness-evidence --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] --out DIR [--state PATH]`
- `cargo test -p capo-cli connectivity_exposure_approval -- --nocapture` verifies latest runtime target readiness evidence export and missing-filter errors
- `cargo test`
- `cargo fmt --check`
- `git diff --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

## F8 - State Store Resilience

Status: in_progress

Source workpad: `state-store.md`

Acceptance:

- Decide whether continued manual `rusqlite` projection expansion is acceptable.
- Compare current Rust database options against Capo's event-log/projection architecture.
- Queue concrete hardening or migration tasks before adding another broad state-model surface.

Progress:

- SS1 state store library decision is completed. Manual `rusqlite` SQL is acceptable as the current implementation, but no longer the preferred path for broad projection growth. Diesel is the first migration candidate to spike; SQLx remains the second candidate for an async/server-oriented path; SeaORM is deferred for the controller core; a typed in-house `rusqlite` projection registry remains the shortest safe hardening option.
- SS2 state crate test module split is completed. The first maintainability slice moved the large inline `capo-state` test module into its own file before deeper state/query/projection splits.
- SS2a state event/error module split is completed. Stable event envelope, redaction, artifact, recovery, and error/result types now live in focused modules with crate-root re-exports preserved.
- SS2b state projection type module split is completed. Projection/read-model type definitions now live in `projections.rs` with crate-root re-exports preserved, while SQL codec/rebuild behavior remains unchanged.
- SS2c state schema module split is completed. SQLite migration DDL, compatibility column backfills, and projection-table reset helpers now live in `schema.rs` without changing schema or rebuild behavior.
- SS2d state projection codec module split is completed. Projection-log row encode/decode logic now lives in `codec.rs` while apply SQL, queries, and public APIs remain unchanged.
- SS2e state projection apply module split is completed. Read-model apply SQL and projection watermark updates now live in `apply.rs` while event append, projection-log insertion, queries, and public APIs remain unchanged.
- SS2f state query module split is completed. Read-only projection and event query methods now live in `queries.rs` while append, recovery, rebuild, projection-log insertion, and public APIs remain unchanged.

Evidence:

- `workpads/features/state-store.md`
- `crates/capo-state/src/lib.rs`
- Diesel docs: https://docs.diesel.rs/main/diesel/index.html
- Diesel migrations docs: https://docs.diesel.rs/main/diesel_migrations/index.html
- SQLx repository/docs: https://github.com/launchbadge/sqlx
- SeaORM docs: https://www.sea-ql.org/SeaORM/docs/index/
- rusqlite repository/docs: https://github.com/rusqlite/rusqlite
- `git diff --check`
- `cargo test -p capo-state`
- `cargo fmt --check`
- `cargo test --workspace --all-targets`
- `cargo clippy --all-targets --all-features -- -D warnings`
