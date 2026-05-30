# Tools And Agent-Computer Interface Tasks

## Objective

Make Capo's agent-computer interface real and high quality by wiring the
existing-but-dead-routed tool layer (`CapoToolRegistry`, `RuntimeToolWrappers`,
`PermissionPolicy`) into the `RealBoundaryController` turn loop and raising the
ACI to daily-driver quality. This workpad adds per-tool input AND output
schemas with risk/scope/redaction metadata, a structured edit/patch tool with
syntax/lint-on-edit feedback, a search/locator tool, a typed test/check tool,
per-call provenance with input-and-output redaction backed by real projections,
and the goal-orchestration `GO2` agent-reporting/evidence tools the autonomy
ledger depends on. It implements the `tool-exposure.md` design and the `GO2`
reporting contract; it does not redesign them.

## Status

Planned. Phase 3 (agent-computer interface), runs parallel with
`streaming-transport` after Phase 1. Depends on `real-turn-loop` (the real loop
must invoke tools). `ACI0` defines routing and scope; all implementation tasks
remain pending.

## Feature Set

- Real tool dispatch wired into the loop, replacing the fake-only routing in
  `ToolExposure::invoke`.
- Per-tool input AND output schemas plus risk/scope/redaction metadata on
  `ToolDefinition`.
- Narrow typed output for existing wrappers and a tightened `file_write`.
- Structured edit/patch tool with whitespace/fuzzy-tolerant matching, structured
  retryable no-match errors, and syntax/lint-on-edit findings.
- Search/grep plus a bounded file/symbol locator with explicit truncation.
- Typed test/check tool emitting decision-grade pass/fail evidence (no scoring).
- Per-call provenance, input-and-output redaction, and real `ToolInvocation` /
  `ToolObservation` projections rebuilt on replay.
- The `GO2` agent-reporting/evidence tool surface persisted as
  `source=agent_reported`, distinct from observed evidence.
- Deterministic fake tool implementations and a replayable test gate.

## ACI0 - Workpad, Routing, Scope, And Reconciliation

Status: pending.

Acceptance:

- Decide and record that this is a new `tools-aci` workpad, distinct from
  `real-turn-loop` (the substrate that calls tools), `safety-gates` (permission
  enforcement, grant lifecycle, and `score_run`), and `goal-autonomy` (goal,
  continuation, and audit semantics of reports).
- State that this workpad IMPLEMENTS `workpads/architecture/tool-exposure.md` and
  the `workpads/goal-orchestration/tasks.md` `GO2` reporting contract; it cites
  them and does not redesign the goal model or the `GO2` schema.
- Declare the seam to `safety-gates`: ACI defines and instruments tools and
  produces typed test/lint evidence; `safety-gates` enforces `PermissionPolicy`,
  owns grant lifecycle, and computes `score_run`. ACI never scores a run.
- Declare the seam to `goal-autonomy`: ACI pre-lands the `GO2` reporting tool
  surface and persists `source=agent_reported` vs observed evidence;
  `goal-autonomy` (`GA-2`/`GA-6`) validates the projection/audit semantics.
- List which `tool-exposure.md` deferred tools graduate to implemented here so
  doc and code converge: `capo.shell_run`, `capo.file_read`, `capo.file_write`,
  `capo.git_status`, `capo.git_diff`, plus the genuinely-new `capo.apply_patch`,
  `capo.search`, and `capo.test_run`; note `capo.memory_search` stays deferred to
  the memory workpad.
- Record the acceptance+verification invariant: no task in this workpad completes
  on operator self-attestation alone; every manual smoke is paired with a
  deterministic assertion (wire snapshot, exit status, or replay).

Verification:

- `workpads/tools-aci/tasks.md`, `knowledge.md`, and `references.md` exist and
  cite `tool-exposure.md`, `acp-replay-dedupe.md`, and `GO2`.
- Scope/seam decision reviewed against `boundaries.md` and the daily-driver
  review.
- `git diff --check`.

Must not do:

- Do not enforce permissions, manage grants, or compute a score; those belong to
  `safety-gates`.
- Do not redesign the `GO2` schema or the goal model.

## ACI1 - Wire Real Tool Dispatch Into The Loop

Status: done.

Evidence:

- `ToolExposure::invoke` (`crates/capo-tools/src/lib.rs`) no longer routes
  `Capo`/`Runtime` to `FakeToolExposure`: it is now the fake-only summary shim
  (panics for real variants) and a new typed
  `ToolExposure::authorize_and_invoke(ToolExposureRequest, &PermissionPolicy)
  -> ToolExposureResult` dispatches `Capo` ->
  `CapoToolRegistry::authorize_and_invoke` and `Runtime` ->
  `RuntimeToolWrappers::authorize_and_invoke`; a cross-variant request is
  rejected, never downgraded to fake.
- `RealBoundaryController` (`crates/capo-controller/src/real_controller.rs`) is
  constructed with REAL exposures (`ToolExposure::capo()` always live; real
  runtime wrappers via `with_runtime_tools(RuntimeToolConfig)`); the test-only
  `Fake` exposure is never installed. New `dispatch_tool_call` drives the new
  core `FakeBoundaryController::dispatch_tool_call`
  (`crates/capo-controller/src/tool_dispatch.rs`), which reuses the existing
  `scoped_event`/`append_event`/`ToolCallProjection` primitives and normalizes
  the typed audit events onto the canonical
  `tool.call_requested -> permission.requested -> permission.decided ->
  capability.grant_used -> tool.invocation_started ->
  tool.output_artifact_recorded -> tool.output_observed -> tool.call_completed
  -> tool.result_delivered` sequence; it does NOT call
  `append_dispatch_run_exit` (no second pipeline).
- Tests added: `cargo test -p capo-tools` (invoke no longer routes
  Capo/Runtime to fake; authorize_and_invoke dispatches the real registry and
  real wrappers; cross-variant rejection) and `cargo test -p capo-controller`
  (a real turn invokes `capo.agent_status` through `authorize_and_invoke` and
  persists the canonical observed sequence keyed to the turn; a real
  `capo.file_read` turn flows through the runtime wrappers; real exposures are
  not the fake default).
- Scope boundary (loop wiring): ACI1 lands and proves the REAL dispatch SEAM --
  `RealBoundaryController::dispatch_tool_call` runs the real registry/wrappers
  end-to-end (authorize -> invoke -> persisted canonical events + projection),
  exercised through the controller's public dispatch entrypoint. The autonomous
  observe->decide->emit turn loop does NOT yet auto-select and auto-invoke tools
  on a model's behalf (the loop's `send_task` memory-packet shim still uses the
  fake summary `ToolExposure::fake()` for its turn-context summary); promoting
  the dispatch seam into the autonomous loop's decision step is owned by the
  later ACI tasks + `safety-gates`. ACI1's claim is the seam is real and
  driveable, not that the loop autonomously calls tools yet.
- Remediation (review findings on this task): the deny/fail dispatch paths now
  drive the persisted `ToolCallProjection` to its TERMINAL status
  (`denied`/`failed`) instead of sticking at `requested` -- the deny/fail audit
  kinds (`tool.call_canceled`/`tool.call_failed`) have no loop `EventKind`, so
  the terminal projection is attached to the dispatch's last persisted event
  when no `tool.call_completed` is emitted (`tool_dispatch.rs`). The dispatched
  `tool.*` events of one call now share a stamped `item_id` (the
  `tool_call_id`), so `reconstruct_turn_finished` dedups them to a SINGLE
  observed tool ref per call (replay-identity). New deterministic tests:
  `real_controller_denied_capo_dispatch_persists_denied_projection`,
  `real_controller_failed_runtime_dispatch_persists_failed_projection`,
  `real_controller_dispatched_tool_call_reconstructs_as_single_observed_ref`.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`:
  `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -- -D
  warnings` clean; `cargo test --workspace` => all passed, 0 failed.

Acceptance:

- Replace the fake-only routing in `ToolExposure::invoke`
  (`crates/capo-tools/src/lib.rs:67-73`, which sends both `Capo` and `Runtime`
  variants to `FakeToolExposure.invoke`) with typed dispatch that calls
  `CapoToolRegistry::authorize_and_invoke` and
  `RuntimeToolWrappers::authorize_and_invoke`.
- Construct the `RealBoundaryController` (from `real-turn-loop`) with the real
  registry/wrappers instead of `ToolExposure::fake()`
  (`crates/capo-controller/src/lib.rs:72`); keep `Fake` as an explicit
  test-only variant the real path never defaults to.
- A real loop turn invoking `capo.file_read` or `capo.shell_run` flows through
  `authorize_and_invoke` and emits the real audit event sequence
  (`tool.call_requested` -> `permission.requested` -> `permission.decided` ->
  `tool.invocation_started` -> `tool.output_artifact_recorded` ->
  `tool.output_observed` -> `tool.call_completed`).
- Tool dispatch DRIVES the existing execution substrate rather than forming a
  second pipeline: a tool-invoking turn reuses the dispatch primitives and does
  not duplicate run-completion semantics with `append_dispatch_run_exit`.
- A deterministic test proves the fake path is no longer the default for `Capo`
  and `Runtime` variants in the real controller.

Verification:

- Focused `cargo test -p capo-controller` proving a turn invokes a Capo-governed
  tool through `authorize_and_invoke` and persists an observed tool-result event.
- Focused `cargo test -p capo-tools` proving `ToolExposure::invoke` no longer
  routes `Capo`/`Runtime` to `FakeToolExposure`.
- `cargo fmt`.

Must not do:

- Do not build a second orchestration path; the loop drives the existing
  dispatch primitives.

## ACI2 - Per-Tool Input And Output Schemas Plus Metadata

Status: done.

Evidence:

- `ToolDefinition` (`crates/capo-tools/src/lib.rs`) gains two fields matching
  `tool-exposure.md`: `output_schema` (`{"output":{...}}` descriptor in the same
  lightweight shape as the existing `schema_json` input descriptor) and
  `redaction_policy_json` (`{"strategy":...,"fields":[...]}`). A `TOOL_RISK_LEVELS`
  constant (`low`/`medium`/`high`/`critical`) plus `ToolDefinition::risk_is_valid`
  pin risk to the doc's levels, and `ToolDefinition::validate_output` checks an
  emitted result object against the declared `output_schema` (field
  presence/scalar+array type, `?`-suffix optional) so "narrow typed output" is a
  checkable contract rather than convention. No new crate dependency: the
  validator mirrors the existing hand-rolled `schema_json` descriptor convention
  via `serde_json` rather than pulling a full JSON-Schema engine.
- Every registered tool now declares the metadata. Capo-owned tools
  (`describe_tool`) emit `CAPO_REGISTRY_OUTPUT_SCHEMA`
  (`{output:string, output_artifact_id:string}`) and a per-tool
  `capo_redaction_policy`; runtime wrappers (`runtime_wrappers.rs`) emit
  `WRAPPER_OUTPUT_SCHEMA` (`{status:string, summary:string,
  output_artifacts:string[]}`) and a per-tool `wrapper_redaction_policy`.
  `CapoToolResult::narrow_output` / `WrapperToolResult::narrow_output` produce the
  validatable result objects. Existing wrapper risk assignments are preserved
  (`capo.shell_run` high, `capo.git_commit` high, `capo.file_write` medium,
  reads low).
- Tests added (`crates/capo-tools/src/tests.rs`):
  `every_registered_tool_declares_output_schema_risk_scope_and_redaction`
  (non-empty `output_schema`/`required_scopes_json`/`redaction_policy_json` and a
  valid `risk` for all `CAPO_OWNED_TOOLS` + `CAPO_WRAPPER_TOOLS`),
  `wrapper_risk_levels_reconcile_with_tool_exposure`,
  `capo_registry_emitted_results_validate_against_their_output_schema`,
  `wrapper_emitted_results_validate_against_their_output_schema` (real
  fixture-workspace runs through `authorize_and_invoke`), and
  `output_schema_validation_rejects_a_wrong_shaped_result` (negative: the
  validator is a real check, not a rubber stamp).
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => all passed, 0 failed (capo-tools: 32 passed);
  `git diff --check` clean.

Acceptance:

- Extend the existing `ToolDefinition` (`crates/capo-tools/src/lib.rs:294-306`,
  which today carries only `schema_json` for input) with an `output_schema`
  field and a `redaction_policy_json` field, matching `tool-exposure.md`'s
  `ToolDefinition` and codex's input+output schema discipline.
- Require every registered tool (`CAPO_OWNED_TOOLS` and `CAPO_WRAPPER_TOOLS`) to
  declare a non-empty `output_schema`, non-empty `required_scopes_json`, a
  `risk` level, and a `redaction_policy_json`.
- Add a registry test that each tool's emitted result validates against its
  declared `output_schema`, so "narrow typed output" is checkable rather than
  convention.
- Add a registry test asserting `risk`, `required_scopes_json`, and
  `redaction_policy_json` are present and non-empty for every tool.
- Keep `risk` aligned with `tool-exposure.md` levels (`low`/`medium`/`high`/
  `critical`) and reconcile with the existing wrapper risk assignments
  (`capo.shell_run` high, `capo.git_commit` high, `capo.file_write` medium).

Verification:

- Focused `cargo test -p capo-tools` for schema presence and output validation.
- `cargo fmt`.
- `git diff --check`.

## ACI3 - Narrow Typed Output For Wrappers And Tightened file_write

Status: done (gate green after `reached_completion` initializer fix).

Evidence:

- Narrow typed output for every wrapper (`crates/capo-tools/src/runtime_wrappers.rs`):
  `WrapperToolResult` gains a `typed_output: Value` field and
  `narrow_output()` now returns that per-tool object (not the generic
  status/summary/artifact blob). Execution wrappers (`capo.shell_run`,
  `capo.git_status`, `capo.git_diff`, `capo.git_commit`) emit
  `{status, exit_status, passed, duration_ms, output_artifact_id, truncated}`
  (`EXEC_OUTPUT_SCHEMA`); `capo.file_read` emits
  `{status, path, bytes_read, content_hash, output_artifact_id}`
  (`FILE_READ_OUTPUT_SCHEMA`); `capo.file_write` emits
  `{status, path, mode, before_hash, after_hash, bytes_written,
  output_artifact_id, expected_hash?, actual_hash?}`
  (`FILE_WRITE_OUTPUT_SCHEMA`). `describe_tool` now declares the per-tool
  `output_schema` via `wrapper_output_schema(tool_id)`. Deny/fail paths emit a
  schema-valid typed output via `denied_typed_output`/`failed_typed_output`
  (`runtime_wrapper_types.rs`).
- Tightened `capo.file_write`: accepts an `expected_hash` precondition OR a
  structured `replace`/`with` substitution (in addition to whole-file
  `content`); a precondition mismatch returns a typed `precondition_failed`
  result carrying `expected_hash`/`actual_hash` WITHOUT writing (blind clobbers
  impossible). It now emits a real unified-diff artifact (`similar` crate,
  `default-features = false`, `text` only) instead of a before/after hash
  summary.
- `capo.shell_run` over-cap success is NOT failed: execution wrappers run via a
  new `uncapped_runtime_runner()` (runtime limit `usize::MAX`) so the full
  output is preserved in the artifact rather than triggering
  `OutputLimitExceeded`; the wrapper compares the artifact size against the
  configured inline `output_limit_bytes` cap and records `truncated` in the
  typed result while keeping `status=exited`/`passed=true`.
- Tests added (`crates/capo-tools/src/tests.rs`):
  `shell_run_typed_output_carries_exit_status_passed_duration_and_artifact`,
  `shell_run_over_cap_success_is_truncated_not_failed`,
  `file_read_typed_output_carries_path_bytes_and_hash`,
  `file_write_emits_a_unified_diff_artifact`,
  `file_write_precondition_mismatch_does_not_clobber`,
  `file_write_structured_replace_edits_in_place`, plus a deny-path
  schema-validation assertion in
  `file_wrappers_record_input_output_artifacts_and_reject_workspace_escape`.
  Existing ACI2 output-schema validation tests still cover the wrappers.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` clean;
  `cargo test --workspace` => 343 passed, 0 failed (capo-tools: 38 passed);
  `git diff --check` clean.
- Gate remediation: the `WrapperExecution` struct carried a documented
  `reached_completion` field (and a `completed()` constructor) that the six
  handler initializer sites in `runtime_wrappers.rs` (`shell_run`, `git_command`,
  `git_commit`, `file_read`, `file_write` success, `file_write`
  precondition-failed) did not set, which broke compilation
  (`missing field reached_completion`). Fixed by routing the five
  unit-of-work-completing returns through `WrapperExecution::completed(...)`
  (`reached_completion: true`) and setting `reached_completion: false` on the
  `precondition_failed` no-op return, matching the `invoke_authorized` match
  arms that branch on `execution.reached_completion`. Re-ran the gate from
  `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check` clean;
  `cargo clippy --all-targets --all-features -- -D warnings` clean;
  `cargo test --workspace` => 343 passed, 0 failed (capo-tools: 38 passed);
  `git diff --check` clean.

Acceptance:

- Add narrow typed output to the existing wrappers (`capo.shell_run`,
  `capo.file_read`, `capo.file_write`, `capo.git_status`, `capo.git_diff`,
  `capo.git_commit`) so each returns a validated typed result rather than only
  status/summary/artifact blobs
  (`crates/capo-tools/src/runtime_wrappers.rs:251-455`).
- Tighten `capo.file_write` (`runtime_wrappers.rs:426-455`, today a whole-file
  overwrite recording only before/after `content_hash`) to accept an
  expected-precondition hash OR a structured replace, and to emit a unified-diff
  artifact rather than only a before/after hash summary.
- A `file_write` whose expected-precondition hash does not match the on-disk file
  returns a typed precondition-failed result without writing, so blind clobbers
  are impossible.
- `capo.shell_run` typed output carries exit status, a `passed` interpretation,
  duration, and `output_artifact_id`, with inline output bounded by the existing
  `output_limit_bytes` cap and full output in the artifact.
- A successful run that exceeds the output cap is NOT classified as failed:
  output is truncated with truncation recorded in the typed result, proven by a
  deterministic over-cap successful-run test.

Verification:

- Focused `cargo test -p capo-tools` for typed wrapper output, precondition-fail
  `file_write`, unified-diff artifact, and over-cap success classification.
- `cargo fmt`.
- `git diff --check`.

## ACI4 - Structured Edit/Patch Tool With Lint-On-Edit

Status: done.

Evidence:

- New `capo.apply_patch` runtime-wrapper tool (added to `CAPO_WRAPPER_TOOLS`,
  `crates/capo-tools/src/lib.rs`) with a typed search/replace patch model behind
  one typed interface (`crates/capo-tools/src/apply_patch.rs`): each hunk is
  located by a cascade of aider-style strategies -- `perfect` -> `whitespace`
  (per-line trim) -> `dotdotdot` (`...` elided interior, anchored head+tail) ->
  edit-distance `fuzzy` (sliding-window line similarity, threshold 0.80), plus an
  empty-search `insert` for create/append. The handler lives in
  `runtime_wrappers.rs::apply_patch`.
- Structured retryable no-match: a hunk no strategy can locate returns a typed
  `no_match` result (status `no_match`, `rejected_hunk_index`, `reject_reason`,
  `nearest_line`, plus a nearest-candidate preview in the summary), shaped after
  aider's `SearchReplaceNoExactMatch` -- not a raw error string -- and writes
  nothing. It flows through the non-completed audit shape (no
  `tool.call_completed`) like the ACI3 `precondition_failed` guard.
- Successful apply returns a typed diff result
  (`APPLY_PATCH_OUTPUT_SCHEMA`): `path`, `hunks_total`, `hunks_applied`,
  `hunks_rejected`, `changed_line_ranges` (1-based inclusive), and the full
  unified diff (via the existing `similar` differ) as a redacted artifact
  (`apply_patch_diff`), never inline.
- Lint-on-edit (`crates/capo-tools/src/lint.rs`): after applying, a
  language-pluggable lint check runs (`Linter::for_path`); Rust files run
  `rustfmt --check` through the bounded runtime runner and parse into typed
  findings (`file`, `line`, `rule`, `message`) with `lint_status`
  `passed`/`failed`/`skipped`/`unavailable`. The runner clears the environment,
  so the linter program is resolved to an absolute path against the current
  `PATH` for deterministic spawn. `auto_lint:false` opts out.
- Path confinement: patch writes go through the existing
  `resolve_workspace_path` (which calls `ensure_under_workspace`), so an absolute
  escape or `..` traversal is rejected with a typed `failed` result and the
  out-of-workspace file is untouched.
- Tests added: unit tests in `apply_patch.rs` (perfect/whitespace/dotdotdot/
  fuzzy/no-match/insert) and `lint.rs` (rustfmt parse + pluggable selection),
  plus integration tests in `tests.rs` exercising the real wrapper
  `authorize_and_invoke`: `apply_patch_clean_apply_returns_typed_diff_and_changed_ranges`,
  `apply_patch_whitespace_and_fuzzy_tolerant_location`,
  `apply_patch_rejected_hunk_returns_structured_retryable_error_without_writing`,
  `apply_patch_lint_on_edit_returns_typed_findings`,
  `apply_patch_lint_passes_on_well_formatted_rust`,
  `apply_patch_cannot_edit_outside_the_workspace`. The wrapper-count assertion was
  updated 8 -> 9; ACI2's "every tool declares output_schema/risk/scope/redaction"
  and the schema-validation tests cover `capo.apply_patch` automatically.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => 359 passed, 0 failed (capo-tools: 54 passed);
  `git diff --check` clean.

Acceptance:

- Implement `capo.apply_patch` with a typed patch model: search/replace hunks
  with whitespace/fuzzy-tolerant location (aider-style perfect / whitespace /
  dotdotdot / edit-distance), or a codex-style unified-patch parser, behind one
  typed interface.
- On a failed match, return a STRUCTURED retryable error (aider's
  `SearchReplaceNoExactMatch`-shaped: which path, which hunk, nearest candidate)
  the loop can reflect on and retry, not a raw error string.
- A successful apply returns a typed diff result: files touched, hunks
  applied/rejected, and changed line ranges; the full diff is a redacted
  artifact.
- After applying, run a syntax/lint check (Rust-first via `rustfmt --check`; the
  interface is language-pluggable) and return typed lint findings
  (`file`, `line`, `rule`, `message`) the loop can reflect on and repair,
  mirroring aider's `auto_lint` -> `lint_edited` -> reflected message.
- Patch writes reuse the wrapper path-confinement (`ensure_under_workspace`,
  `crates/capo-tools/src/runtime_wrappers.rs:525-549`) so a patch cannot edit
  outside the workspace.
- Deterministic tests cover clean apply, fuzzy/whitespace-tolerant apply,
  rejected-hunk structured error, and lint-failure-with-typed-findings, all over
  fake/real fixtures with no live provider.

Verification:

- Focused `cargo test -p capo-tools` for clean/fuzzy/rejected/lint paths.
- Out-of-workspace patch rejection test.
- `cargo fmt`.
- `git diff --check`.

## ACI5 - Search/Grep And Bounded Locator

Status: done (gate green after `clip_preview` tuple / `preview_clipped`
initializer fix and dropping ripgrep `--max-count` so `total_matches` is honest).

Evidence:

- New `capo.search` runtime-wrapper tool (added to `CAPO_WRAPPER_TOOLS`,
  `crates/capo-tools/src/lib.rs`) is ripgrep-backed through the bounded runtime
  runner (`runtime_wrappers.rs::search`): it runs `rg --json --sort path` (the
  `--sort path` forces deterministic, single-threaded ordering) confined to the
  workspace and parses ripgrep's line-delimited JSON into typed, capped
  `{path, line, preview}` matches. ripgrep is resolved to an ABSOLUTE path
  against the current `PATH` (the runner clears the env), like the ACI4 linter.
  An `rg` exit of 1 (no matches) is a normal empty search, not a failure; an exit
  of 2+ (bad regex) is surfaced as a typed handler error.
- Bounded/decision-grade result model (`crates/capo-tools/src/search.rs`):
  `apply_caps` enforces TWO caps -- a per-call match cap (`max_matches`, default
  50) AND a total preview byte cap (`max_preview_bytes`, default 8 KiB), plus a
  per-line clip (256 bytes) so one pathological long line cannot blow the budget.
  The typed output (`SEARCH_OUTPUT_SCHEMA`) carries `matches`, `returned_matches`,
  `total_matches`, an explicit `truncated` boolean, and a `truncation_reason`
  (`none`/`match_cap`/`byte_cap`) so the agent knows the result is partial rather
  than silently incomplete. Whole files are never inlined: a bounded search emits
  no output artifact, only the capped `path:line:preview` triples. The first
  match is always kept (clipped) so an oversized line still yields one usable
  result. Callers may tighten/widen the caps via `max_matches`/`max_preview_bytes`
  (non-positive values are rejected so a cap can never be disabled into a
  whole-repo dump).
- Path confinement + redaction: the search root defaults to the workspace and may
  be narrowed to a confined subpath via the shared `resolve_workspace_path`
  (`ensure_under_workspace`), so a `..`/absolute escape is rejected with a typed
  `failed` result before ripgrep runs. Every preview line is scrubbed through the
  configured `redact_bytes` BEFORE capping/returning, and the per-tool redaction
  policy declares the `preview` field (`wrapper_redaction_policy`).
- Tests added: unit tests in `search.rs` (rg-JSON parse, skip-unparseable,
  match-cap, byte-cap-before-match-cap, under-both-caps, first-match-always-kept,
  per-line clip) plus integration tests in `tests.rs` exercising the real wrapper
  `authorize_and_invoke` over a fixture repo:
  `search_returns_typed_bounded_path_line_preview_matches`,
  `search_per_call_match_cap_truncates_with_explicit_marker`,
  `search_total_byte_cap_truncates_with_explicit_marker`,
  `search_empty_result_is_a_successful_not_failed_call`,
  `search_redacts_secrets_in_previews`,
  `search_cannot_read_outside_the_workspace`. The wrapper-count assertion was
  updated 9 -> 10; ACI2's "every tool declares output_schema/risk/scope/redaction"
  test covers `capo.search` automatically.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => 378 passed, 0 failed (capo-tools: 72 passed);
  `git diff --check` clean.
- Gate remediation: the working tree carried an unfinished `preview_clipped`
  change that broke compilation -- `clip_preview` was destructured as a
  `(String, bool)` tuple in `apply_caps` but still returned a plain `String`
  (`search.rs:110`), and two `SearchMatch` initializers (the `parse_ripgrep_json`
  push and the `sample_match` test helper) omitted the new `preview_clipped`
  field. Fixed by making `clip_preview` return `(String, bool)` (clipped text +
  whether a clip occurred) and setting `preview_clipped: false` on the
  raw-parse and test initializers (raw matches are unclipped; `apply_caps` sets
  the flag when it clips). Separately, the `search` handler was passing
  `--max-count = max_matches` (a PER-FILE ripgrep cap) which made the reported
  `total_matches` a capped undercount (6 instead of 21 in the cap fixture),
  failing `search_per_call_match_cap_truncates_with_explicit_marker`; removed it
  so ripgrep reports the true pre-cap total while `apply_caps` still enforces the
  per-call match cap and the total byte cap in-process and the runner's
  `artifact_limit_bytes` ceiling stays the hard backstop for a pathological hot
  query. Re-ran the gate from `/Users/nicolas/devel/capo-wt/tools-aci`:
  `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -- -D
  warnings` exit 0; `cargo test --workspace` => 0 failed (capo-tools: 72 passed,
  including the previously-failing match-cap test).

Acceptance:

- Implement `capo.search` (ripgrep-backed through `RuntimeRunner`) and a bounded
  file/symbol locator returning typed capped results
  (`path:line:preview`, max N matches per call, total byte cap), inspired by
  aider's repomap and codex's file-search.
- Results carry an explicit truncation marker when the cap is hit, so the agent
  knows the result is partial rather than silently incomplete.
- Search/locator reads stay inside the workspace via the existing path
  confinement and respect redaction on previews.
- Output is decision-grade and bounded: the agent finds edit targets without the
  tool dumping whole files.
- A deterministic test on a fixture repo proves the per-call cap, the total byte
  cap, and the truncation signal.

Verification:

- Focused `cargo test -p capo-tools` for capped results and truncation marker.
- `cargo fmt`.
- `git diff --check`.

## ACI6 - Typed Test/Check Tool

Status: done.

Evidence:

- New `capo.test_run` runtime-wrapper tool (added to `CAPO_WRAPPER_TOOLS`,
  `crates/capo-tools/src/lib.rs`) is a specialized shell wrapper
  (`runtime_wrappers.rs::test_run`) returning the typed
  `{command, exit_status, passed, failing_items, duration_ms,
  output_artifact_id}` record (`TEST_RUN_OUTPUT_SCHEMA`), plus
  `failing_items_total`/`failing_items_truncated` and wall-clock
  `started_at`/`completed_at`. It runs through the existing bounded runtime
  runner (full output preserved up to the artifact ceiling) like the ACI3
  execution wrappers. `passed` is the exit-status interpretation (exit 0 ==
  passed); the observed `status` is `"failed"` for a non-zero exit-under-test
  (consistent with `capo.shell_run`) but the call still COMPLETES
  (`tool.call_completed`) because it delivered a full evidence record -- the
  decision-grade signal is `passed:false` + `failing_items`, not a tool error.
- `failing_items` is BOUNDED, decision-grade evidence
  (`crates/capo-tools/src/test_run.rs`): `extract_failing_items` prefers failing
  test NAMES from recognized harness shapes (cargo test `... FAILED`, cargo
  nextest `FAIL [..]`, pytest `FAILED ...`, TAP `not ok`) and falls back to the
  first-N non-empty lines when no names are recognized. It is capped by a count
  cap (`max_failing_items`, default 20, caller-tightenable; non-positive
  rejected) AND a per-item byte cap (256), with an explicit elision marker line
  when the count cap fires and `failing_items_truncated` set, so the inline
  payload never dumps the whole log. The FULL output always lives in a redacted
  artifact (`test_run_output`, scrubbed through `redact_bytes`,
  `redaction_state=redacted`).
- Evidence-only, no scoring: the handler computes no score and owns no gate. The
  `test_run.rs` module + `TEST_RUN_OUTPUT_SCHEMA` doc comments state explicitly
  that `safety-gates`' `VerificationRunner` consumes this typed record and owns
  `score_run`; ACI never scores a run. `started_at`/`completed_at` are recorded
  as millis-since-epoch (no date/time crate in the workspace, consistent with the
  integer `duration_ms`) for later wall-clock evaluation by the gate.
- Tests added: unit tests in `test_run.rs` (passing has no items; cargo/pytest/
  TAP name parsing; first-lines fallback; count-cap truncation marker; per-item
  byte clip) plus integration tests in `tests.rs` exercising the real wrapper
  `authorize_and_invoke` with a deterministic `/bin/sh` fake command:
  `test_run_passing_command_returns_typed_passed_record_with_timing`,
  `test_run_failing_command_captures_bounded_failing_items_and_full_artifact`,
  `test_run_caps_inline_failing_items_with_explicit_truncation_marker`,
  `test_run_redacts_secrets_in_the_output_artifact`. The wrapper-count assertion
  was updated 10 -> 11; ACI2's "every tool declares output_schema/risk/scope/
  redaction" and the output-schema-validation tests cover `capo.test_run`
  automatically.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => 388 passed, 0 failed (capo-tools: 82 passed);
  `git diff --check` clean.

Acceptance:

- Implement `capo.test_run` / `capo.check` as a specialized shell wrapper
  (`tool-exposure.md:196-198`) returning a typed result
  `{command, exit_status, passed, failing_items, duration_ms,
  output_artifact_id}`.
- `failing_items` captures failing test names or the first-N failure lines;
  inline output is capped to N lines/bytes and the full output lives in a
  redacted artifact.
- The tool emits typed evidence only: it does NOT compute a score or own the
  verification gate. State explicitly that `safety-gates`' `VerificationRunner`
  consumes this typed record and owns `score_run`.
- The typed result records `started_at`/`completed_at` and a wall-clock
  `duration_ms` for later evaluation.
- Deterministic tests with a fake command cover both pass and fail, assert output
  is bounded, and assert the full output is in an artifact.

Verification:

- Focused `cargo test -p capo-tools` for pass/fail typed result and bounded
  output.
- `cargo fmt`.
- `git diff --check`.

Must not do:

- Do not implement scoring logic; that is `safety-gates`.

## ACI7 - Per-Call Provenance And Input-And-Output Redaction

Status: done.

Evidence:

- Real redaction policy (`crates/capo-runtime/src/lib.rs`): new `RedactionPolicy`
  combines the operator-declared literal `RedactionRule` patterns with a default
  credential-shape/high-entropy scan (`scan_credential_shapes` /
  `is_credential_shaped`: known key prefixes `AKIA`/`ASIA`/`sk-`/`ghp_`/`AIza`/
  `glpat-`/`xox[bap]-`/`github_pat_`, `Bearer <token>` headers, and long
  high-entropy credential-char runs with letter+digit mix and >=12 distinct
  chars), returning `(bytes, redaction_state)`. `key=value` prefixes are stripped
  on the FIRST `=` so base64 `=` padding is preserved. The runner's
  `redact_output` now runs the full policy (process stdout/stderr gets the
  credential scan, not just literal patterns). No new crate dependency -- the scan
  is a hand-rolled token pass consistent with the existing FNV `content_hash`.
- Input AND output redaction at the wrapper boundary
  (`crates/capo-tools/src/runtime_wrappers.rs`): `redact_bytes` now delegates to
  the policy, a new `redact_bytes_with_state` records the real `redaction_state`,
  and a new `write_redacted_artifact` scrubs OUTPUT content before it lands in an
  artifact. `file_read`, `file_write` (diff), and `apply_patch` (diff) output
  artifacts now go through it, so a secret in tool OUTPUT -- not only input -- is
  redacted; `record_input_artifact` uses the same policy.
- `ToolInvocation`/`ToolObservation` projections carry provenance + timing
  (`crates/capo-state/src/projections.rs`): `ToolCallProjection` gains a
  `provenance: ToolCallProvenance` (`correlation_id`, `permission_decision_id`,
  `capability_grant_use_id`, `started_at`, `completed_at`). Wired through the
  schema (`tool_calls` columns), `apply.rs` INSERT/UPDATE, codec encode/decode
  (provenance rides in the projection-record payload), and
  `tool_calls_for_session`, so a restart/replay rebuilds it identically. The
  `tool.invocation_started`/`tool.output_artifact_recorded`/`tool.output_observed`
  events are persisted by the existing dispatch path (ACI1).
- Queryable provenance end to end (`crates/capo-controller/src/tool_dispatch.rs`):
  `dispatch_tool_call` captures wall-clock `started_at`/`completed_at` around the
  real authorize+invoke and stamps the `ToolCall` projection with a
  `correlation_id` (`corr-<session>-<run>-<turn>-<tool_call>`, the tool_call_id
  being the shared event item ref / join key), a `permission_decision_id`
  (`decision-<grant>`), and a per-invocation `capability_grant_use_id`
  (`grant-use-<tool_call>-<grant>`). Non-dispatch construction sites default
  provenance cleanly; the CLI wrapper ties a correlation/decision/grant id.
- Tests added: `cargo test -p capo-runtime` (RedactionPolicy rules-then-scan,
  credential-shape recognition, ordinary-text-left-untouched false-positive
  guard, Bearer-token scrub, rules-only skips the scan); `cargo test -p
  capo-tools` (`file_read_redacts_a_configured_secret_in_the_output_artifact`,
  `file_read_credential_shape_scan_redacts_an_unnamed_secret_in_output` -- proving
  a known secret in tool OUTPUT is redacted in the artifact, not only input);
  `cargo test -p capo-state`
  (`tool_call_provenance_and_timing_persist_and_rebuild_identically` -- projection
  carries provenance/timing and rebuilds byte-identically on replay);
  `cargo test -p capo-controller`
  (`real_controller_dispatch_persists_provenance_and_timing_that_replays_identically`
  -- the real dispatch path persists the queryable correlation/decision/grant
  chain + timing and replays identically).
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => all 26 suites passed, 0 failed (capo-tools 86,
  capo-state 37, capo-runtime 23, capo-controller 36); `git diff --check` clean.

Acceptance:

- Apply redaction to BOTH input and output artifacts. Today `redact_bytes`
  (`crates/capo-tools/src/runtime_wrappers.rs:515-523`) is a literal substring
  replace applied to input only; extend it to a real policy: configurable
  patterns PLUS a default credential-shape/high-entropy scan that reuses
  capo-runtime's credential scanning and `RedactionRule` machinery, recording a
  `redaction_state` per artifact.
- Implement the `ToolInvocation` and `ToolObservation` projections and the
  `tool.invocation_started` / `tool.output_artifact_recorded` /
  `tool.observation_recorded` events from `tool-exposure.md` (today design-only,
  emitted only as in-memory `ToolAuditEvent` strings) so provenance is queryable.
- Provenance is queryable end to end: a `correlation_id` ties command -> turn ->
  permission -> tool -> artifact -> adapter event, plus
  `permission_decision_id` and `capability_grant_use_id` per invocation.
- Capture `started_at`/`completed_at` per call for later wall-clock evaluation.
- A redaction test proves a known secret in tool OUTPUT is redacted in the
  artifact (not only input).
- A projection test proves `ToolInvocation` rows carry provenance and timing, and
  a restart/replay test proves the same provenance rebuilds identically.

Verification:

- Focused `cargo test -p capo-tools` and `cargo test -p capo-state` for
  redaction, projections, and provenance.
- Restart/replay test rebuilding `ToolInvocation`/`ToolObservation` projections.
- `cargo fmt`.

## ACI8 - Agent-Reporting And Evidence Tools (GO2)

Status: done.

Evidence:

- New `agent_reports.rs` module (`crates/capo-tools/src/agent_reports.rs`)
  registers the 11 `GO2` reporting tools as a distinct Capo-owned surface
  (`CAPO_REPORTING_TOOLS`, `AgentReportRegistry`): `capo.report_intent`,
  `capo.report_progress`, `capo.record_evidence`, `capo.report_confidence`,
  `capo.record_assumption`, `capo.raise_blocker`, `capo.request_review`,
  `capo.record_review`, `capo.record_validation`, `capo.complete_requirement`,
  `capo.complete_subtask`. Each `describe_tool` declares a `schema` (`{"input"}`),
  non-empty `required_scopes_json` (the `tool:invoke:<tool>` scope plus a
  per-tool `state:write:agent_report`/evidence/blocker/review/validation/
  requirement_status scope), a `risk` (`low`/`medium`/`high`), a
  `redaction_policy_json` (credential_scan over the free-text fields + `body` +
  `output`), and `mutates_state` (pure intent/progress/confidence/assumption are
  observations; evidence/blocker/review/validation/completion mutate the
  autonomy ledger). It CITES `GO2` (`workpads/goal-orchestration/tasks.md:86-104`)
  and does not redesign the schema.
- Agent reports persist as a DISTINCT class tagged `source=agent_reported`
  carrying confidence: `AgentReportRecord` always has
  `source == EVIDENCE_SOURCE_AGENT_REPORTED`, `is_observed_evidence()` is always
  false, and emission yields a `tool.observation_recorded` audit event tagged
  `agent_reported` (NOT a `tool.output_observed` runtime-evidence event).
  `source_is_observed_evidence` keeps `runtime_output`/`adapter_event(:..)`
  distinct from the agent claim. A denied submission emits `accepted=false` and
  no observation, so a report the policy rejected is not a claim of record --
  completion is never reachable by agent assertion alone.
- Each report carries an idempotency key so duplicates dedupe on replay: an
  agent-supplied `submission_id` is authoritative (`agent-report:<id>`), else a
  stable FNV hash over session/tool/body so a re-emitted identical report
  collapses while two distinct reports stay distinct. `AgentReportLedger`
  (the deterministic fake/replayable ledger) dedupes on that key.
- Wired as a real `ToolExposure::AgentReports(AgentReportRegistry)` variant
  (always live in `RealToolExposures`, never the fake default) dispatched through
  the existing typed `authorize_and_invoke` /
  `RealBoundaryController::dispatch_tool_call`; the dispatch normalizes the report
  onto a `tool.observation_recorded` event + a `ToolObservationProjection` tagged
  `source=agent_reported` (carrying confidence), reusing the ACI1/ACI7 substrate
  rather than a second pipeline.
- Tests added: `cargo test -p capo-tools` (registration of all 11 tools; every
  tool declares schema/scope/risk/redaction/mutates_state; report stored as
  `agent_reported` and NOT observed evidence; typed dispatch through the exposure;
  denied report is not a claim; explicit-key dedupe with a distinct report kept;
  keyless identical-body dedupe with different bodies distinct).
  `cargo test -p capo-controller`
  (`real_controller_dispatches_an_agent_report_persisted_as_agent_reported`: the
  dispatched report persists a `tool.observation_recorded` event +
  `tool_observations` row tagged `source=agent_reported` with confidence,
  distinct from observed evidence, and replays identically).
  `cargo test -p capo-state`
  (`agent_reported_observations_are_distinct_from_observed_and_dedupe_on_replay`:
  an `agent_reported` observation is a distinct class from a `runtime_output`
  observed row, the duplicate report submission dedupes on its idempotency key,
  and the classification replays identically).
- Scope: emission + fakes here; projection/audit semantics are validated in
  `goal-autonomy` (`GA-2`/`GA-6`) per the workpad seam.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => all suites passed, 0 failed (capo-tools 93,
  capo-controller 37, capo-state 38); `git diff --check` clean.

Acceptance:

- Register each `GO2` reporting tool in the typed registry with
  `schema`/`required_scopes`/`risk`/`redaction_policy`/`mutates_state`, per
  `workpads/goal-orchestration/tasks.md:86-104`: `capo.report_intent`,
  `capo.report_progress`, `capo.record_evidence`, `capo.report_confidence`,
  `capo.record_assumption`, `capo.raise_blocker`, `capo.request_review`,
  `capo.record_review`, `capo.record_validation`, `capo.complete_requirement`,
  `capo.complete_subtask`.
- Persist agent reports as a DISTINCT event/projection class tagged
  `source=agent_reported` (carrying confidence), separate from observed tool
  evidence tagged `source=runtime_output`/`adapter_event`, so completion is never
  reachable by agent assertion alone.
- Each report event carries an idempotency key so duplicate report submissions
  dedupe on replay.
- Cite `GO2` as the design source and do not redesign the schema; scope is
  emission + fakes here, with projection/audit semantics validated in
  `goal-autonomy` `GA-2`/`GA-6`.
- Provide fake implementations of each reporting tool for replayable tests.
- A test proves a report is stored as `agent_reported` and is NOT
  indistinguishable from observed evidence.

Verification:

- Focused `cargo test -p capo-tools` / `cargo test -p capo-state` proving
  `agent_reported` vs observed classification and idempotent dedupe.
- `cargo fmt`.
- `git diff --check`.

## ACI9 - Tool-Result Normalization Into Events And Projections

Status: done.

Evidence:

- Every dispatched tool result now normalizes into BOTH the `ToolInvocation`
  (`ToolCall`) AND `ToolObservation` projections
  (`crates/capo-controller/src/tool_dispatch.rs`). The prior code only emitted a
  `ToolObservation` projection for the `agent_reported` reporting surface, so a
  query over `tool_observations_for_session` surfaced agent CLAIMS but no
  observed evidence for locally-dispatched Capo/runtime tools. `NormalizedToolResult`
  gains an `observed_evidence: Option<ObservedEvidence>` (set by `from_capo` /
  `from_runtime`, `None` for the report surface), and the dispatch's
  `EventKind::ToolOutputObserved` branch now emits a `ToolObservation` row tagged
  `source=runtime_output` (`EVIDENCE_SOURCE_RUNTIME_OUTPUT`), carrying the
  observed terminal status (the wrapper's own `exited`/`failed`/
  `precondition_failed`/`no_match`, or Capo's `completed`), `instrumentation_level=full`,
  and the output `artifact_id`. The deny path emits no `tool.output_observed`
  event, so no observation row is created for a denied call even though the field
  is populated.
- Observed evidence (`source=runtime_output`) stays a DISTINCT observation class
  from agent-reported claims (`source=agent_reported`, ACI8) and adapter-native
  observed evidence (`source=adapter_event:<adapter>`, `adapter_replay.rs`).
  `source_is_observed_evidence` keeps the observed sources distinct from the
  agent claim; the dispatched observed row carries `confidence="observed"` (it is
  observed, not self-attested) vs the report's numeric self-declared confidence.
- Adapter-native tool updates with stable external IDs already dedupe on replay
  (`crates/capo-controller/src/adapter_replay.rs`: the `toolCallId`-derived
  `adapter_tool_call_id` plus the idempotency key / fallback key), and the
  ACI8 idempotency-keyed agent-report dedupe is unchanged.
- Raw inputs/outputs that may contain secrets stay artifacts with a
  `redaction_state` (ACI7 `write_redacted_artifact` / `record_input_artifact`),
  never inline in event blobs; the observation row references the artifact id, not
  the content.
- Read models expose ordered tool calls (`tool_calls_for_session`) with
  permission decision / grant / timing provenance, input+output artifacts, status,
  and the observation read model (`tool_observations_for_session`) now exposes the
  observed-vs-reported source classification and instrumentation level for BOTH
  classes.
- Tests added/strengthened: `cargo test -p capo-controller`
  (`real_controller_turn_invokes_a_capo_tool_through_authorize_and_invoke` now
  asserts the dispatched observed Capo tool persists a `runtime_output`
  observation row with the output artifact; new
  `real_controller_dispatch_persists_observed_and_reported_distinctly_and_replays`
  dispatches an observed tool AND an agent report in one session, asserts the two
  are DISTINCT observation classes -- `runtime_output` is observed evidence,
  `agent_reported` is not -- and that the separation replays byte-identically
  after `rebuild_projections`). The existing `cargo test -p capo-state`
  `agent_reported_observations_are_distinct_from_observed_and_dedupe_on_replay`
  covers the state-level normalization, idempotency-key dedupe, and
  replay-identity of the observed-vs-reported classes.
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`: `cargo fmt --check`
  clean; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
  `cargo test --workspace` => all suites passed, 0 failed (capo-controller 38,
  capo-state 38, capo-tools 93, capo-server 60).

Acceptance:

- Normalize every tool result (Capo registry, runtime wrappers, edit/patch,
  search, test) into the canonical event sequence and into the `ToolInvocation`
  and `ToolObservation` projections.
- Keep observed tool evidence (`source=runtime_output`/`adapter_event`) a
  distinct event class from agent-reported claims (`source=agent_reported`).
- Adapter-native tool updates with stable external IDs dedupe on replay
  (`tool-exposure.md:352`, `acp-replay-dedupe.md`: `toolCallId` is stable within
  a session; `tool_call_update` fields are partial replacements).
- Raw tool inputs/outputs that may contain secrets are stored as artifacts with
  `redaction_state`, never inline in event blobs.
- Read models expose ordered tool calls with permission decision, artifacts,
  output, status, delivery state, and instrumentation level.

Verification:

- Focused `cargo test -p capo-state` for normalization and dedupe.
- Replay test proving observed vs reported separation survives rebuild.
- `cargo fmt`.

## ACI10 - Fake/Deterministic Tool Implementations

Status: pending.

Acceptance:

- Provide deterministic fake/scripted implementations for every tool
  (`capo.shell_run`, `capo.file_read`, `capo.file_write`, `capo.apply_patch`,
  `capo.search`, `capo.test_run`, the git wrappers, and the `GO2` reporting
  tools) that produce stable outputs without a live provider or real process.
- Fakes emit the same event/artifact/projection shape as the real path so
  replay and projection-rebuild tests can run deterministically.
- Fakes cover both clean and failure paths (e.g. rejected patch hunk, failing
  test command, permission denial).
- The fake variant is clearly test-only and is never the default in the real
  controller (reconciles with `ACI1`).

Verification:

- Focused `cargo test -p capo-tools` exercising fakes for clean and failure
  paths.
- `cargo fmt`.
- `git diff --check`.

## ACI11 - Deterministic Tool Tests, Redaction, Replay, And E2E Gate

Status: pending.

Acceptance:

- Run deterministic fake/scripted tests for every tool, clean and failure paths,
  with NO live provider.
- Run a redaction test asserting a known secret is stripped from BOTH the input
  and output artifacts.
- Run a restart/replay test proving `ToolInvocation`/`ToolObservation`
  projections rebuild identically and adapter-native tool updates with stable
  external IDs dedupe (`tool-exposure.md:352`).
- Run a full ACI e2e path through the real loop: a turn invokes `capo.file_read`,
  `capo.apply_patch` (with lint-on-edit), and `capo.test_run`; observed evidence
  and an `agent_reported` completion claim are persisted distinctly; provenance
  is queryable; the run replays identically.
- Any live-provider tool smoke (real shell/edit against a scratch repo) is behind
  an explicit opt-in env gate mirroring `CAPO_SERVER_RUN_CODEX_LIVE`, with
  secrets stripped, and is paired with a deterministic assertion so completion is
  never operator-asserted alone.

Verification:

- `cargo fmt`.
- `cargo clippy` and focused `cargo test -p capo-tools -p capo-controller
  -p capo-state` for changed crates.
- Restart/replay test and the e2e path above.
- `git diff --check`.
