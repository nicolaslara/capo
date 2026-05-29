# Tools And ACI Knowledge

## Objective

Capture decisions for making Capo's agent-computer interface real and high
quality: wire the existing-but-dead-routed tool layer into the
`RealBoundaryController` turn loop, extend tool definitions with typed input AND
output schemas plus risk/scope/redaction metadata, raise edit/patch/search/test
quality to daily-driver standard, instrument every call with provenance and
input-and-output redaction, and pre-land the goal-orchestration `GO2`
agent-reporting/evidence tool surface that the autonomy ledger depends on.

This workpad implements `workpads/architecture/tool-exposure.md` and the
`workpads/goal-orchestration/tasks.md` `GO2` reporting contract. It cites those
designs and does not redesign the goal model or the `GO2` schema.

## Scope Decision

Create a new `tools-aci` workpad in Phase 3 (agent-computer interface), running
parallel with `streaming-transport` after Phase 1 (`real-turn-loop`). It depends
hard only on `real-turn-loop` (the real loop must invoke tools);
`streaming-transport` is a soft/parallel integration (a thin follow-on streams
tool-call/result frames once both exist), not a hard prerequisite.

This is a BUILD-ON workpad, not greenfield. The substrate already exists: the
`CapoToolRegistry`, the `RuntimeToolWrappers` (`capo.shell_run`,
`capo.file_read`, `capo.file_write`, `capo.git_status`, `capo.git_diff`,
`capo.git_commit`), `authorize_and_invoke`, `PermissionPolicy`, and path
containment (`ensure_under_workspace`) are all built. The load-bearing gap is
that `ToolExposure::invoke` hard-routes both the `Capo` and `Runtime` variants to
`FakeToolExposure` (`crates/capo-tools/src/lib.rs:67-73`) and the controller is
constructed with `ToolExposure::fake()`
(`crates/capo-controller/src/lib.rs:72`), so the entire real tool path is dead
code. The work here is therefore: WIRE `authorize_and_invoke` into the real loop;
EXTEND `ToolDefinition` with output schemas and risk/scope/redaction metadata;
RAISE ACI quality (narrow typed output, a structured edit/patch tool with
syntax/lint-on-edit, a search/locator, a typed test tool); INSTRUMENT calls with
provenance and input-and-output redaction backed by real projections; and
IMPLEMENT the `GO2` agent-reporting/evidence tools as a distinct
`source=agent_reported` event class.

This workpad is distinct from its neighbors. `real-turn-loop` owns the substrate
that calls tools. `safety-gates` owns permission enforcement, the grant
lifecycle, and `score_run`/the verification gate; ACI never scores a run, it only
produces the typed test/lint evidence the gate consumes. `goal-autonomy` owns the
goal model, the continuation scheduler, and the audit/projection semantics of
reports; ACI only pre-lands the `GO2` tool surface and the observed-vs-reported
distinction it relies on.

Deferred `tool-exposure.md` tools that graduate to implemented here so doc and
code converge: `capo.shell_run`, `capo.file_read`, `capo.file_write`,
`capo.git_status`, `capo.git_diff`, plus the genuinely-new `capo.apply_patch`,
`capo.search`, and `capo.test_run`. `capo.memory_search` stays deferred to the
memory workpad.

## Wire The Real Path, Kill The Fake Routing

The first and load-bearing decision is to stop routing real tools to the fake.
`ToolExposure::invoke` must dispatch the `Capo` and `Runtime` variants into
`CapoToolRegistry::authorize_and_invoke` and
`RuntimeToolWrappers::authorize_and_invoke` (both already exist,
`runtime_wrappers.rs:242` and `lib.rs:283`), and the `RealBoundaryController`
must be built with the real registry/wrappers, not `ToolExposure::fake()`. `Fake`
stays an explicit, test-only variant the real path never defaults to.

Tool dispatch DRIVES the existing execution substrate rather than forming a
second pipeline. A tool-invoking turn reuses the dispatch primitives and the
canonical event sequence from `tool-exposure.md`
(`tool.call_requested` -> `permission.requested` -> `permission.decided` ->
`tool.invocation_started` -> `tool.output_artifact_recorded` ->
`tool.output_observed` -> `tool.call_completed`); it does not duplicate
run-completion semantics with a parallel `append_dispatch_run_exit`. This honors
the boundary model's one-orchestration-path rule and reconciles ACI tool results
with the dispatch-run-exit events / execution-status projections from
`real-turn-loop`.

## Narrow Typed I/O, Not Raw Blobs

Tools return narrow typed output validated against a declared `output_schema`,
not raw blobs. The existing `ToolDefinition`
(`crates/capo-tools/src/lib.rs:294-306`) carries only `schema_json` for input; it
gains an `output_schema` and a `redaction_policy_json`, matching
`tool-exposure.md`'s `ToolDefinition` record and codex's input+output schema
discipline (the codex `tools` crate registers tools with explicit JSON schemas in
`tool_definition.rs` / `json_schema.rs`). Every registered tool
(`CAPO_OWNED_TOOLS`, `CAPO_WRAPPER_TOOLS`) must declare a non-empty
`output_schema`, non-empty `required_scopes_json`, a `risk` level
(`low`/`medium`/`high`/`critical`), and a `redaction_policy_json`. A registry test
makes "narrow typed output" checkable rather than convention: each tool's emitted
result validates against its declared `output_schema`. Risk stays aligned with
the existing wrapper assignments (`capo.shell_run` high, `capo.git_commit` high,
`capo.file_write` medium).

The wrappers gain narrow typed output instead of only status/summary/artifact
blobs. `capo.shell_run` output carries exit status, a `passed` interpretation,
duration, and `output_artifact_id`, with inline output bounded by the existing
`output_limit_bytes` cap and full output in the artifact. Critically, a
successful run that exceeds the output cap is NOT classified as failed (the
runtime today returns `Err(OutputLimitExceeded)` and discards artifacts on
overflow); output is truncated with truncation recorded in the typed result.

`capo.file_write` stops being a blind whole-file overwrite. Today it records only
before/after `content_hash` (`runtime_wrappers.rs:426-455`) and clobbers
unconditionally. It gains an expected-precondition hash OR a structured replace,
returns a typed precondition-failed result without writing when the hash does not
match the on-disk file, and emits a unified-diff artifact rather than only a
hash summary. Blind clobbers become impossible.

## Edit/Patch Quality With Lint-On-Edit Is The Headline Lever

Edit/patch quality with syntax/lint-on-edit is the single highest-value
daily-driver ACI lever, and it is entirely absent today (no edit/patch/search/test
tools exist). `capo.apply_patch` is a new tool with a typed patch model behind one
typed interface: search/replace hunks with whitespace/fuzzy-tolerant location
(aider-style perfect / whitespace / dotdotdot / edit-distance fallbacks), or a
codex-style unified-patch parser. Codex's `apply-patch` crate already proves the
shape: a typed `Hunk` model (`parser.rs`), fuzzy context location via
`seek_sequence.rs`, and a structured `ApplyPatchError` rather than a raw string.

On a failed match the tool returns a STRUCTURED retryable error
(aider's `SearchReplaceNoExactMatch`-shaped: which path, which hunk, nearest
candidate) the loop can reflect on and retry, not a raw error string. A
successful apply returns a typed diff result (files touched, hunks
applied/rejected, changed line ranges) with the full diff as a redacted artifact.
After applying, a syntax/lint check runs (Rust-first via `rustfmt --check`; the
interface is language-pluggable) and returns typed lint findings
(`file`, `line`, `rule`, `message`) the loop can reflect on and repair, mirroring
aider's `auto_lint` -> `lint_edited` -> reflected-message loop. Patch writes reuse
the wrapper path confinement (`ensure_under_workspace`,
`runtime_wrappers.rs:525-549`) so a patch cannot edit outside the workspace.

`capo.search` is ripgrep-backed through the runtime runner, plus a bounded
file/symbol locator returning typed capped results (`path:line:preview`, max N
matches per call, total byte cap), inspired by aider's repo-map and codex's
file-search crate. Results carry an explicit truncation marker when the cap is
hit, so the agent knows the result is partial rather than silently incomplete;
reads stay inside the workspace and respect redaction on previews. Output is
decision-grade and bounded: the agent finds edit targets without the tool dumping
whole files.

`capo.test_run` / `capo.check` is a specialized shell wrapper
(`tool-exposure.md:196-198`) returning a typed
`{command, exit_status, passed, failing_items, duration_ms, output_artifact_id}`,
with `failing_items` capturing failing test names or the first-N failure lines,
inline output capped, and full output in a redacted artifact. It emits typed
evidence only; it does NOT compute a score or own the verification gate.
`safety-gates`' `VerificationRunner` consumes this typed record and owns
`score_run`. It records `started_at`/`completed_at` and wall-clock `duration_ms`
for later evaluation.

## Provenance, Redaction, And Artifact Instrumentation

Every tool call is instrumented with queryable provenance and timing, and is
replayable via fake implementations. Redaction is enforced on BOTH input and
output at the tool boundary. Today `redact_bytes`
(`runtime_wrappers.rs:515-523`) is a literal substring replace applied to INPUT
only; output (shell stdout/stderr) is exactly where secrets leak and is never
touched. The fix is a real policy: configurable patterns PLUS a default
credential-shape/high-entropy scan that reuses capo-runtime's credential scanning
and `RedactionRule` machinery, recording a `redaction_state` per artifact, applied
to input and output alike. Raw inputs/outputs that may contain secrets are stored
as artifacts with `redaction_state`, never inline in event blobs.

The `tool-exposure.md` `ToolInvocation` and `ToolObservation` projections and the
`tool.invocation_started` / `tool.output_artifact_recorded` /
`tool.observation_recorded` events become real state (today they are design-only,
emitted only as in-memory `ToolAuditEvent` strings). Provenance is queryable end
to end: a `correlation_id` ties command -> turn -> permission -> tool -> artifact
-> adapter event, plus `permission_decision_id` and `capability_grant_use_id` per
invocation, with `started_at`/`completed_at` captured per call. A restart/replay
test proves the same provenance rebuilds identically. Adapter-native tool updates
with stable external IDs dedupe on replay (`tool-exposure.md:352`,
`acp-replay-dedupe.md`: `toolCallId` is stable within a session and
`tool_call_update` fields are partial replacements).

## The GO2 Reporting And Evidence Tool Surface

The `GO2` agent-reporting/evidence tools are implemented here because tool
registration is an ACI concern. Each tool is registered in the typed registry
with `schema`/`required_scopes`/`risk`/`redaction_policy`/`mutates_state`, per
`workpads/goal-orchestration/tasks.md:86-104`: `capo.report_intent`,
`capo.report_progress`, `capo.record_evidence`, `capo.report_confidence`,
`capo.record_assumption`, `capo.raise_blocker`, `capo.request_review`,
`capo.record_review`, `capo.record_validation`, `capo.complete_requirement`,
`capo.complete_subtask`.

The load-bearing decision is that reports are claims, not proof. Agent reports
persist as a DISTINCT event/projection class tagged `source=agent_reported`
(carrying confidence), separate from observed tool evidence tagged
`source=runtime_output`/`adapter_event`, so completion is never reachable by
agent assertion alone. Each report event carries an idempotency key so duplicate
submissions dedupe on replay. This cites `GO2` as the design source and does not
redesign the schema; the scope here is emission plus fakes, with the
projection/audit semantics validated in `goal-autonomy` (`GA-2`/`GA-6`).

## ACI Lessons From Daily Drivers

- SWE-agent's ACI thesis: a concise, constrained, decision-grade interface beats
  a raw shell. Narrow typed output, bounded results, structured errors, and
  feedback-on-edit are what make a model effective, not raw capability. This is
  why every tool returns validated typed output and why edit/patch returns
  structured retryable errors and lint findings.
- Codex `apply-patch`: a typed `Hunk` model, fuzzy context location
  (`seek_sequence.rs`), and a structured `ApplyPatchError` are the proven shape
  for a reliable edit tool; codex's `tools` crate also proves explicit
  input/output JSON-schema discipline per tool.
- Codex `file-search`: a dedicated, bounded file-search tool keeps locating cheap
  and prevents whole-file dumps; our `capo.search` + locator follows this with
  per-call and byte caps and an explicit truncation marker.
- Aider repo-map and editblocks: ranked repo context plus search/replace blocks
  with whitespace/fuzzy/dotdotdot fallbacks and an `auto_lint` -> reflected-error
  repair loop are the highest-leverage daily-driver edit ergonomics; our
  apply-patch and lint-on-edit mirror this directly.

## Non-Goals

- Do not enforce permissions or manage the grant lifecycle; that is
  `safety-gates`.
- Do not score runs or own the verification gate; ACI provides the typed
  test/lint evidence and `safety-gates` owns `score_run`.
- Do not redesign the `GO2` schema or the goal model; cite goal-orchestration as
  the design source.
- Do not implement OS sandboxing; that is `depth`.
- Do not build a second orchestration path; the loop drives the existing dispatch
  primitives.
- No web client.

## Open Questions

- Edit/patch model: aider-style search/replace editblocks or a codex-style
  unified-patch parser, or both behind one typed interface? Leaning toward one
  typed interface that accepts either input form so the loop sees a single
  structured result and error contract.
- Is the post-apply lint check language-pluggable from day one or Rust-only
  (`rustfmt --check`) first? Leaning Rust-first with a pluggable interface so
  additional languages slot in without a redesign.
- How aggressive should the default credential-shape/high-entropy output scan be
  before it produces false positives that hide useful output from the agent?
- Should the typed test tool's `failing_items` cap be line-count, byte, or
  failure-count based, and is that cap shared with the search truncation policy?
