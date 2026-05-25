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
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`
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

Evidence:

- `crates/capo-state/src/lib.rs`
- `crates/capo-eval/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-state memory_record`
- `cargo test -p capo-state task_outcome`
- `cargo test -p capo-eval`
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`
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

Evidence:

- `crates/capo-voice/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-voice`
- `cargo test -p capo-cli voice -- --nocapture`
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
