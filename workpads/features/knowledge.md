# Feature Knowledge

## Objective

Capture cross-feature decisions and sequencing until individual feature workpads exist.

Feature phase is ready after prototype gate P15.

## F0 - Split Feature Workpads

Status: completed on 2026-05-25.

Decisions:

- Split the post-prototype backlog by boundary rather than by UI milestone. This keeps real-agent connectors, workpad dogfood, dashboard/query, permissions/tools, memory/eval, voice, and remote runtime independently reviewable.
- The first feature priority should be either `agent-connectors.md` if the goal is real Codex/Claude execution, or `dogfood-bridge.md` if the goal is importing Capo's own workpads before real-agent execution.
- Real local agent execution remains the main product constraint from the prototype gate. Fake agents prove controller/state/evidence semantics, not useful coding output.
- Workpad import/update safety is the main dogfood bridge constraint. Evidence export is safe, but Capo cannot yet manage source workpad files directly.
- Dashboard and voice should share a reusable query surface before adding richer UI or conversational clients.

Follow-up:

- `agent-connectors.md` should start with Codex opt-in smoke because Codex is already wired through restrictive smoke-plan code.
- `dogfood-bridge.md` should preserve the source-of-truth distinction between markdown task status and Capo execution status.

## F2/DB1 - Workpad Index

Status: completed on 2026-05-25.

Decisions:

- Start the dogfood bridge with read-only indexing rather than task execution. Capo can now observe the project workpad tree without mutating source markdown.
- Add `capo-workpads` for markdown scanning and task-status extraction. The crate has no third-party dependencies and deliberately writes nothing.
- Persist workpad observations through SQLite projections:
  - `workpad_files`: path, project, content hash, headings, objective text, observed timestamp, update sequence.
  - `workpad_tasks`: source task ID, project, path, source anchor, title, observed markdown status, Capo execution status, observed timestamp, update sequence.
- Use `observed_only` as the initial Capo execution status for imported markdown tasks. DB2 will decide how selected observed tasks become executable Capo tasks.
- Add `capo workpad index --root PATH --state PATH` as the first non-destructive CLI path for dogfood import.
- After review, constrain the scanner to selected Capo workpad docs rather than recursive `workpads/**` indexing. This prevents prior-art clones and reference repos from becoming Capo task refs.
- Add mixed-case task ID support for headings like `A2a`, `A5a`, and `R2a`.
- Add a reset projection at the start of each workpad index batch so rebuild and re-index remove stale file/task refs.

Verification:

- `cargo test -p capo-workpads`: passed.
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.
- Manual repo smoke `capo workpad index --root /Users/nicolas/devel/capo --state <tmp>` reported `files=43`, `tasks=98` after scoping fixes.

Review:

- Focused review initially blocked DB1 on over-indexing, missed mixed-case task IDs, and stale projections. The fixes above were applied before completion.

Follow-up:

- DB2 should map selected `workpad_tasks` into Capo task records while preserving markdown status as observed source truth.
- DB3 should add reviewed update/evidence proposal artifacts before Capo can apply any changes to source workpads.

## F2/DB2 - Capo Task Import

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad import --workpad-task WORKPAD_TASK_ID` to convert a selected observed workpad task into a normal Capo task read model.
- Default imported task IDs are deterministic from the workpad task ID, with an optional `--task TASK_ID` override for operators.
- Preserve the data boundary:
  - `observed_status` remains the markdown status observed from the source file.
  - `workpad_tasks.capo_execution_status=imported` means Capo has imported that source task.
  - `tasks.capo_execution_status=ready` means the Capo task record is ready for later orchestration.
- Store source path, heading anchor, source hash, observed status, and workpad task ID in the import event payload and task summary until DB3 adds Capo-owned reviewed artifacts.
- Use project-scoped idempotency keys for imports based on task ID, workpad task ID, and source hash so repeated imports of the same observed source do not duplicate events.
- Preserve imported workpad execution status across no-change re-indexes. Re-index still removes stale workpad task refs when the markdown source task disappears, and later restores them if the source content recurs.
- Do not use project-scoped idempotency keys for workpad index events. The projection reset must reapply every observation so A-B-A source changes cannot leave current read models stale.
- Refuse `--task` imports that would overwrite an existing unrelated, active, or session-linked Capo task read model.
- Use `--expected-hash` for optimistic source drift checks. Imports with a stale expected hash fail before writing Capo task state.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.

Review:

- Focused review found two blockers in the first draft: source-fingerprint recurrence could leave stale read models, and imports could clobber existing Capo task state. Both were fixed with regression coverage.

Follow-up:

- DB3 should replace ad hoc source metadata in task summaries with Capo-owned proposal/evidence artifacts.
- Dashboard/query work should expose imported workpad task refs without forcing consumers to parse task summaries.

## F2/DB3 - Reviewed Workpad Artifacts

Status: completed on 2026-05-25.

Decisions:

- Add `capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR` as the first safe write-adjacent dogfood command.
- Proposal artifacts are Capo-owned markdown files marked with `<!-- capo:workpad-proposal -->`. They include source path, source anchor, source hash, observed markdown status, Capo execution status, proposed update text, apply policy, and rollback/fallback instructions.
- Proposal generation records a safe `workpad_update_proposal` artifact row and evidence projection, but does not edit source workpad markdown.
- `capo workpad apply --proposal PATH --confirm` exists as the explicit confirmation surface, but DB3 intentionally leaves source writeback disabled. Confirmed apply reports `workpad_apply_supported=false` and `source_modified=false`.
- Proposal artifact identity includes task ID, workpad task ID, source hash, and proposal text. Different proposal bodies get different artifact files.
- Existing non-Capo files are never overwritten. Existing changed Capo proposal files are also not overwritten, so human review notes cannot be silently erased.

Verification:

- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Manual smoke against this repo with temporary state/output dirs: `workpad index`, `workpad import`, `workpad propose`: passed.

Review:

- Focused review found one blocker in the first draft: repeated proposal writes with different bodies could overwrite the artifact while event idempotency no-opped. The fix was to include proposal text in artifact identity and refuse changed Capo proposal overwrites.

Follow-up:

- Future source writeback should validate source hash at apply time, generate a patch/diff artifact first, and keep a rollback artifact before modifying markdown.
- Dashboard/query work should expose proposal artifact/evidence refs directly instead of making users parse CLI output.

## F1/AC1-AC2 - Local Connector Preflight

Status: in progress on 2026-05-25.

Decisions:

- Do not run the real Codex subscription-backed smoke without explicit user opt-in. The ignored test was executed without `CAPO_RUN_CODEX_LOCAL_SMOKE=1` and stayed inside the opt-in gate.
- Installed Codex is `codex-cli 0.133.0`; `codex exec --help` currently supports the planned safe smoke flags: JSONL output, read-only sandbox, ephemeral mode, ignored user config/rules, and isolated `--cd`.
- Installed Claude Code is `2.1.150`; its help currently supports the restricted noninteractive stream path.
- Tighten the Claude smoke plan with `--no-session-persistence`, `--disable-slash-commands`, and `--tools ""` in addition to plan permission mode, disallowed tools, empty MCP config, and strict MCP config.
- Treat Codex as still unproven for dogfood until the real opt-in smoke runs and artifact/state scans pass.
- Treat Claude restricted args as verified enough for a future opt-in smoke, but do not run Claude without explicit authorization.

Verification:

- `cargo test -p capo-adapters local_smoke_plan`: passed.
- `cargo test -p capo-adapters local_adapter_smoke_runner`: passed.
- `cargo test -p capo-adapters artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets`: passed.
- `cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` without opt-in: passed by skipping the provider process.

Review:

- Focused connector safety review found no blocking issues. It confirmed Codex opt-in gating is preserved, Claude restricted flags match current help, and F1 remains honestly in progress because real subscription-backed smoke has not run.

Skipped verification:

- Real Codex local smoke with `CAPO_RUN_CODEX_LOCAL_SMOKE=1` was not run because explicit opt-in is required.
- Real Claude local smoke with `CAPO_RUN_CLAUDE_LOCAL_SMOKE=1` was not run because this pass only verified restricted arguments.

Follow-up:

- After explicit user opt-in, run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture`, inspect artifacts/state for credential markers, and decide whether Codex is safe enough for first dogfood.
- If Codex passes, AC3 should route a real adapter event stream through controller/state/evidence instead of stopping at adapter-level smoke.
