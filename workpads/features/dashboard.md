# Dashboard Feature

## Objective

Move dashboard data access into a reusable query surface and build richer operator views without letting UI code own orchestration state.

## Prototype Inputs

- P13 added a text dashboard that reads SQLite projections.
- P15 decided that text dashboard is enough for first dogfood, while richer views can follow.

## Dependencies

- CLI, dashboard, voice, mobile, and web views must render the same read-model/query contract.
- No dashboard view should read live adapter/runtime process state directly.

## Tasks

### DS1 - Query Surface Extraction

Status: completed

Acceptance:

- Extract agent/session/dashboard aggregation from `capo-cli` into a reusable controller or query crate/module.
- Keep output independent from terminal rendering.
- Preserve P12/P13 assertions through existing CLI commands.

Evidence:

- `crates/capo-query/src/lib.rs`
- `crates/capo-query/Cargo.toml`
- `crates/capo-cli/src/main.rs`
- `Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-query`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Decision:

- Add `capo-query` as the reusable read-model aggregation crate for operator surfaces.
- Keep `capo-query` dependent only on `capo-core` and `capo-state`, so CLI, dashboard, voice, mobile, and web views can share the same query contract without coupling to terminal rendering or controller side effects.
- Move project dashboard aggregation into `ProjectDashboard`, `AgentDashboardRow`, and `SessionDashboardRow`.
- Keep CLI output formatting in `capo-cli`; it now renders the `capo-query` dashboard model instead of assembling SQLite projections directly.

Review:

- Focused review confirmed dashboard aggregation moved out of CLI and dependencies are clean. It requested stronger query-contract tests for project filtering, idle agents, event limits, and missing sessions; those tests were added before completion.

### DS2 - Operator Dashboard View

Status: completed

Acceptance:

- Show active agents, sessions, goals, blockers, confidence, evidence refs, tool calls, and memory packet refs.
- Add filtering by project/session/status.
- Keep dashboard rendering read-only.

Evidence:

- `crates/capo-query/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-query`
- `cargo test -p capo-cli dashboard_rejects_malformed_filters`
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Decision:

- Extend the reusable `capo-query` dashboard contract with tool-call and memory-packet refs before rendering richer operator output.
- Keep `capo dashboard` as a read-only text operator view for this slice; defer TUI/web presentation until the shared query shape is more stable.
- Support `capo dashboard --project PROJECT_ID`, `--session SESSION_ID`, and `--status STATUS`.
- Treat `--status` as an any-status filter over agent, session, and run status in this first operator view. Split status domains later if this becomes ambiguous in dogfood use.
- Reject malformed or unknown dashboard filters rather than silently widening the displayed state.

Review:

- Focused dashboard review found two completion blockers: no user-facing project filter and malformed filters that could be silently ignored. Both were fixed with CLI regression coverage before completion.
- The same review noted broad `--status` semantics as a low residual risk. The behavior is documented above and left intentionally broad for the first CLI operator view.

### DS3 - Workpad Queue Visibility

Status: completed

Acceptance:

- Expose indexed workpad task rows through the shared dashboard/query surface.
- Render source path, source anchor, observed markdown status, Capo execution status, and default Capo task ID in the CLI dashboard.
- Keep the dashboard read-only and preserve markdown as the source-of-truth fallback.

Evidence:

- `ProjectDashboard.workpad_tasks` in `../../crates/capo-query/src/lib.rs`.
- CLI dashboard workpad task rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query workpad_tasks -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Put workpad queue visibility in the shared query contract rather than a CLI-only read. Voice, web, mobile, and future TUI views should consume the same `ProjectDashboard` workpad task rows.
- Render both observed markdown status and Capo execution status so operators can distinguish source truth from Capo-tracked execution.

### DS4 - Workpad Queue Filters

Status: completed

Acceptance:

- Add explicit dashboard filters for workpad task path and workpad task status.
- Keep workpad queue filters independent from the existing agent/session/run `--status` filter.
- Reject malformed workpad filter flags fail-closed.
- Preserve shared query ownership of the filter behavior.

Evidence:

- `ProjectDashboardQuery::with_workpad_path` and `with_workpad_status` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo dashboard --workpad-path PATH --workpad-status STATUS` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query workpad -- --nocapture`: passed.
- `cargo test -p capo-cli dashboard_rejects_malformed_filters -- --nocapture`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Use explicit workpad filter names instead of overloading `--status`. This keeps operator intent clear when filtering agent/session state versus markdown workpad queue state.
- `--workpad-status` matches either observed markdown status or Capo execution status, because dashboard rows intentionally show both state dimensions.

### DS5 - Project Evidence Visibility

Status: completed

Acceptance:

- Expose project-level evidence rows through the shared dashboard/query surface.
- Keep session evidence refs scoped to sessions while showing migration/checkpoint evidence at project scope.
- Render project evidence IDs, kinds, artifact refs, and confidence in the CLI dashboard.
- Keep the dashboard read-only and derived from persisted projections.

Evidence:

- `SqliteStateStore::project_evidence(...)` in `../../crates/capo-state/src/lib.rs`.
- `ProjectDashboard.project_evidence` in `../../crates/capo-query/src/lib.rs`.
- CLI dashboard project-evidence rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query project_dashboard_includes_project_level_evidence -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Project evidence currently means evidence rows with `session_id IS NULL`. This avoids duplicating session evidence while making dogfood readiness and migration checkpoint reports visible from the shared operator dashboard.
- Keep project evidence in `ProjectDashboard` rather than adding a CLI-only lookup so voice, web, mobile, and future TUI surfaces consume the same read contract.
