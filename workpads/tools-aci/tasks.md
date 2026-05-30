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
- Gate run from `/Users/nicolas/devel/capo-wt/tools-aci`:
  `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -- -D
  warnings` clean; `cargo test --workspace` => 329 passed, 0 failed.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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

Status: pending.

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
