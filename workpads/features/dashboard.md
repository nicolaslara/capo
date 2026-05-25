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
