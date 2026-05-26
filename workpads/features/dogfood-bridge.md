# Dogfood Bridge Feature

## Objective

Make Capo able to read and track its own project workpads while preserving markdown files as the human-auditable source of truth and preventing destructive writes.

## Prototype Inputs

- P11 exports Capo-owned workpad-like evidence without corrupting existing markdown.
- P12 proves state recovery, redirect, interrupt, and evidence refs for two fake sessions.
- The prototype gate passed with the gap that Capo cannot yet import/index the project workpad tree as first-class work.

## Dependencies

- Use SQLite for operational task/session state.
- Treat `TASKS.md`, `project.md`, and `workpads/**` as human-authored source files unless Capo writes a clearly marked artifact.

## Tasks

### DB1 - Workpad Index

Status: completed

Acceptance:

- Index `TASKS.md`, `project.md`, and selected `workpads/**` files into Capo-readable workpad refs.
- Store paths, hashes, headings, objective text, task IDs/statuses, and observed timestamps.
- Do not modify source markdown.

Evidence:

- `crates/capo-workpads/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-workpads`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`

Decision:

- Add `capo-workpads` as a non-destructive markdown observation crate. It reads markdown and returns observed refs; it does not write source workpads or claim ownership of markdown status.
- Add SQLite projections for `workpad_files` and `workpad_tasks`, fed by a durable `workpad.indexed` event.
- Store `observed_status` separately from `capo_execution_status`, initialized as `observed_only`, so later imports can distinguish markdown truth from Capo execution state.
- Expose the first operator command as `capo workpad index --root <project> --state <state>`.
- Scope indexing to Capo-owned project/workpad docs and direct finding/feature files; do not recurse into `workpads/references/repos/**` or prior-art clone markdown.
- Clear prior workpad projections for the project at the start of each index projection batch so deleted or removed markdown tasks do not remain current after rebuild.
- Accept mixed-case task IDs such as `A2a`, `A5a`, and `R2a`.

Review:

- Focused review found three blockers in the first draft: over-indexing prior-art repos, missing mixed-case task IDs, and stale projection risk. All were fixed before completion.

### DB2 - Capo Task Import

Status: completed

Acceptance:

- Convert selected workpad tasks into Capo task records with source anchors.
- Preserve distinction between observed markdown status and Capo execution status.
- Re-indexing is idempotent and detects source drift.

Evidence:

- `crates/capo-core/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`

Decision:

- Add `capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID]` as the first bridge from observed markdown work into executable Capo task records.
- Keep `observed_status` on `workpad_tasks` as the markdown source observation and set the imported Capo task read model to `capo_execution_status=ready`.
- Mark the imported source workpad task with `capo_execution_status=imported` so operators can distinguish observed-only work from work that Capo is now tracking.
- Store source path, source anchor, content hash, observed status, and workpad task ID in the imported task summary and event payload until DB3 adds richer Capo-owned artifacts.
- Preserve imported workpad execution status across re-indexes for tasks still present in markdown, while allowing reset/re-index to remove stale source refs.
- Use optional `--expected-hash` as the drift guard. Imports fail with `source drift detected` when the caller imported against an old observed file hash.

Review:

- Focused review found two blockers in the first draft: repeated source fingerprints could no-op projection reset/reapply, and `--task` could overwrite an existing Capo task read model. Both were fixed before completion.

### DB3 - Reviewed Workpad Artifacts

Status: completed

Acceptance:

- Write Capo-owned evidence/update proposal artifacts without overwriting user-authored files.
- Require explicit confirmation before applying changes to source workpads.
- Provide rollback/fallback instructions for first dogfood.

Evidence:

- `crates/capo-core/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`
- Manual smoke: `capo workpad index`, `capo workpad import`, then `capo workpad propose` against this repo using temporary state/output directories.

Decision:

- Add `capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR [--expected-hash HASH] [--task TASK_ID] [--summary TEXT]` to write Capo-owned proposal artifacts.
- Proposal artifacts start with `<!-- capo:workpad-proposal -->`, record source path/anchor/hash, and include apply policy plus rollback/fallback instructions.
- Proposal writes do not modify source markdown and refuse to overwrite non-Capo files.
- Proposal identity includes the proposal text as well as task/source refs, so different proposal bodies produce different artifacts.
- Changed Capo proposal files are not overwritten; exact same proposal reruns remain idempotent.
- Add `capo workpad apply --proposal PATH --confirm` as a guarded apply surface. DB3 intentionally keeps apply as a confirmed no-op that reports `workpad_apply_supported=false` and `source_modified=false`.

Review:

- Focused review found one blocker: repeated proposal writes with different bodies could overwrite an artifact while the event no-opped. Proposal identity and overwrite guards were fixed before completion.

### DB4 - Next Workpad Selection

Status: completed

Acceptance:

- Select the next actionable indexed workpad task from Capo read models without mutating markdown.
- Prefer `in_progress` source tasks before `pending`, `ready`, or opt-in-blocked tasks.
- Do not return workpad tasks already imported into Capo execution state.
- Allow scoping to a specific workpad path.
- Return the source anchor, observed markdown status, Capo execution status, and default Capo task ID for import.

Evidence:

- `capo workpad next [--path PATH]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- Add a read-only operator command for selecting the next actionable indexed workpad task. It does not import tasks, edit markdown, or write proposal artifacts.
- Selection prefers observed markdown `in_progress`, then `pending`, then `ready`, then `waiting_on_opt_in`.
- Selection only considers `capo_execution_status=observed_only`; once a workpad task is imported, the Capo task record becomes the execution authority for that task.
- The command returns the source workpad task ID, source anchor, observed markdown status, Capo execution status, and deterministic default Capo task ID so the operator can explicitly import or inspect the task next.

### DB5 - Start Next Workpad Task

Status: completed

Acceptance:

- Compose next-workpad selection, explicit import, and controller dispatch without editing markdown.
- Dispatch the selected workpad task to a named registered agent through the existing controller command path.
- Preserve the imported task ID as the Capo execution task ID instead of creating an unrelated goal-derived task.
- Keep the command fake/local until real connector proof is recorded.

Evidence:

- `capo workpad start-next --agent NAME [--path PATH]` in `../../crates/capo-cli/src/main.rs`.
- `FakeBoundaryController::send_task_command` can accept an explicit task ID while preserving existing goal-derived task behavior for ordinary `task send`.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources -- --nocapture`: passed.

Decision:

- `start-next` remains an explicit operator command. It does not change markdown status or automatically choose a real provider connector.
- The command imports only `observed_only` workpad refs selected by DB4, then sends the selected goal through the existing controller/runtime/adapter/tool instrumentation path.
- Existing `task send` behavior remains compatible; explicit task IDs are only used when provided in the command envelope.

### DB6 - Dogfood Readiness Surface

Status: completed

Acceptance:

- Add a read-only operator command that summarizes whether Capo is ready to move its own workpads into Capo-managed dogfood.
- Reuse shared query/read-model state instead of live runtime, provider, or filesystem inspection.
- Report real-agent connector readiness, workpad bridge readiness, dispatch-chain readiness, counts, blockers, and next actions.
- Keep the command honest when provider opt-in evidence is missing.

Evidence:

- `ProjectDogfoodReadiness` and `project_dogfood_readiness(...)` in `../../crates/capo-query/src/lib.rs`.
- CLI `capo dogfood readiness [--state PATH]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.

Decision:

- Treat dogfood readiness as a shared query contract, not a CLI-only checklist.
- Require three independent signals before the summary reports ready: real-agent connector evidence, indexed workpad state, and a recorded dispatch chain.
- The command does not run provider CLIs, inspect credentials, materialize prompts, create tunnels, or edit markdown.

### DB7 - Dogfood Readiness Evidence Export

Status: completed

Acceptance:

- Export the shared dogfood readiness query as a Capo-owned markdown evidence artifact.
- Refuse to overwrite non-Capo or changed Capo readiness artifacts.
- Record artifact and evidence rows without binding the report to a fake session or provider run.
- Keep the report prompt-redacted and derived only from persisted read models.

Evidence:

- CLI `capo dogfood readiness --out DIR [--state PATH]` in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli help_mentions -- --nocapture`: passed.

Decision:

- Readiness evidence uses the marker `<!-- capo:dogfood-readiness -->` and an artifact ID based on the rendered content hash.
- The report is project-level evidence. It records connector, workpad, and dispatch-chain readiness without pretending a provider CLI ran or a session produced the artifact.
- The artifact explicitly states that it does not run provider CLIs, inspect credentials, materialize prompts, open tunnels, or edit markdown.

### DB8 - Dogfood Readiness Component Refs

Status: completed

Acceptance:

- Include component read-model refs in the shared dogfood readiness query.
- Render connector evidence refs, workpad task refs, dispatch chain refs, and project evidence refs in CLI readiness output and readiness evidence artifacts.
- Keep refs metadata-only and do not render raw prompts, provider output, credentials, or source markdown content.

Evidence:

- `ProjectDogfoodReadiness` component ref fields in `../../crates/capo-query/src/lib.rs`.
- CLI `capo dogfood readiness` and readiness evidence rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.

Decision:

- Treat component refs as review breadcrumbs for the migration checkpoint. Operators can see which persisted rows support or block readiness without parsing raw events.
- Use IDs only: smoke report IDs, workpad task IDs, dispatch plan/replay/execution IDs, and project evidence IDs. Raw prompts, provider fixture text, provider output, and markdown source bodies remain excluded.

### DB9 - Runtime Target Dogfood Readiness

Status: completed

Acceptance:

- Include runtime target readiness in the shared dogfood readiness query.
- Require at least one available runtime target before Capo reports ready for first dogfood.
- Render runtime target counts and refs in CLI readiness output, dashboard readiness rows, voice readiness answers, and readiness evidence artifacts.
- Keep the readiness check read-model-derived and provider-free: do not launch runtimes, launch providers, inspect credentials, open tunnels, materialize prompts, request approvals, activate grants, or edit markdown.

Evidence:

- `ProjectDogfoodReadiness` runtime target fields and readiness computation in `../../crates/capo-query/src/lib.rs`.
- CLI/dashboard/voice dogfood readiness rendering in `../../crates/capo-cli/src/main.rs`.
- Voice dogfood-readiness read contract in `../../crates/capo-voice/src/lib.rs`.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p capo-query dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli adapter_dispatch_gate -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.
- `cargo test`: passed.

Decision:

- Treat runtime target readiness as an execution-placement prerequisite distinct from connector proof, workpad bridge state, dispatch-chain state, and connectivity exposure.
- Count only targets with `status=available` as satisfying the gate. Disabled and unhealthy targets remain visible through runtime target status surfaces but do not clear dogfood readiness.
- Use runtime target IDs as review breadcrumbs in readiness output and artifacts. The readiness check does not prove a runtime process is live or that a tunnel is open.
