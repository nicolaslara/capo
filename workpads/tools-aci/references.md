# Tools And ACI References

## Objective

Record the local and external sources that shape the tools-aci workpad. Dated
claims reflect 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/tool-exposure.md`
  - The canonical tool model this workpad implements.
  - Key facts: Capo-exposed tools are controller capabilities, not agent
    identities; every visible tool call becomes a `ToolCall` record and durable
    event sequence; execution is gated by `PermissionPolicy` even under the
    trusted local profile; raw inputs/outputs that may contain secrets become
    artifacts with `redaction_state`, not inline event blobs. The
    `ToolExposure` enum has `Capo`/`Runtime`/`AdapterNative`/`ProviderNative`/
    `Mcp`/`Fake` variants. `ToolDefinition` carries `schema_json`,
    `required_scopes_json`, `risk` (`low`/`medium`/`high`/`critical`),
    `redaction_policy_json`, `exposure`, and `instrumentation_level`. The
    invocation lifecycle defines the canonical events `tool.call_requested`,
    `tool.invocation_started`, `tool.output_artifact_recorded`,
    `tool.output_observed`, `tool.observation_recorded`, `tool.call_completed`,
    `tool.call_failed`. `ToolInvocation` carries `correlation_id`,
    `permission_decision_id`, `capability_grant_use_id`,
    `started_at`/`completed_at`. `capo.test_run` is named as a specialized shell
    wrapper with test/evidence metadata (lines 196-198). `capo.file_write` is
    specified to record before/after hashes AND diff artifacts. Adapter-native
    tool updates with stable external IDs dedupe during replay (line 352).
    `capo.shell_run`, `capo.file_read`, `capo.file_write`, `capo.git_status`,
    `capo.git_diff`, `capo.test_run`, and `capo.memory_search` are listed as
    deferred-but-expected wrapper tools.
- `workpads/architecture/acp-replay-dedupe.md`
  - Key facts: `toolCallId` is stable within an ACP session and
    `tool_call_update` fields are partial replacements, so adapter-native tool
    updates must dedupe and merge on replay rather than append duplicates.
- `workpads/harness-research/daily-driver-review.md`
  - Grounded readiness facts driving this workpad's framing.
  - Key facts: verdict is NO (not yet a daily driver); the permission engine +
    durable grants exist but are inert and not wired into the loop; the only live
    path is a read-only one-shot Codex; verification is operator-asserted with no
    test/lint runner; the runtime buffers-then-caps output so a long successful
    run is misclassified as an error; the strongest dimension is the
    event-sourced SQLite state core (idempotency, projections, replayable
    rebuild). The phased roadmap puts wiring permissions/ToolExposure and a real
    VerificationRunner in the safety phase, after the loop is real.

## Local Product And Implementation Sources

- `crates/capo-tools/src/runtime_wrappers.rs`
  - Key facts (observed 2026-05-29): `RuntimeToolWrappers` implements the wrapper
    tools and exposes `authorize_and_invoke` (line 242). `file_write`
    (lines 426-455) is a blind whole-file overwrite recording only before/after
    `content_hash` with no precondition check and a hash-summary artifact rather
    than a real diff. `redact_bytes` (lines 515-523) is a literal substring
    replace over the configured `redaction_rules`, applied to INPUT artifacts
    only; output is never redacted. `resolve_workspace_path` /
    `ensure_under_workspace` (lines 525-549) enforce path confinement against
    `workspace_root`. Risk levels are assigned per tool (`capo.shell_run` high,
    `capo.git_commit` high). Tool audit events are emitted as in-memory
    `ToolAuditEvent` values, not persisted projections.
- `crates/capo-tools/src/lib.rs`
  - Key facts (observed 2026-05-29): `ToolExposure::invoke` hard-routes both the
    `Capo` and `Runtime` variants to `FakeToolExposure.invoke` (lines 67-73), so
    the real registry/wrapper path is dead code. `CapoToolRegistry` and the
    `CAPO_OWNED_TOOLS` / `CAPO_WRAPPER_TOOLS` lists are defined here; both
    registries expose `authorize_and_invoke`. `ToolDefinition` (lines 294-306)
    carries `schema_json` for input but has no `output_schema` and no
    `redaction_policy_json` field.
- `crates/capo-tools/src/runtime_wrapper_paths.rs`
  - Key facts (observed 2026-05-29): `ensure_under_workspace` (line 77)
    canonicalizes the workspace root and rejects any path that does not start
    with it; `workspace_path` and `nearest_existing_ancestor` resolve and
    validate candidate paths. This is the containment engine `capo.apply_patch`
    and `capo.search` reuse.
- `crates/capo-controller/src/lib.rs`
  - Key facts (observed 2026-05-29): the controller is constructed with
    `tools: ToolExposure::fake()` (line 72), so even though the real wrappers
    exist they are never reached from the controller path.
- `workpads/goal-orchestration/tasks.md` (`GO2`)
  - The reporting-tool contract this workpad pre-lands.
  - Key facts: `GO2` (lines 80-110) defines the first reporting tool surface
    (`capo.report_intent`, `capo.report_progress`, `capo.record_evidence`,
    `capo.report_confidence`, `capo.record_assumption`, `capo.raise_blocker`,
    `capo.request_review`, `capo.record_review`, `capo.record_validation`,
    `capo.complete_requirement`, `capo.complete_subtask`), requires schemas,
    required scopes, risk levels, redaction policy, and a `mutates_state` flag per
    tool, and states that reports may explain intent/confidence but do not replace
    observed evidence. `goal-orchestration/knowledge.md` defines the
    observed/reported/validated/reviewed/contradicted/stale/redacted evidence
    statuses and that completion requires requirement-level evidence, not global
    confidence.

## External Sources

- SWE-agent agent-computer interface (ACI)
  - https://github.com/princeton-nlp/SWE-agent
  - Observed 2026-05-29.
  - Key facts: the ACI thesis is that a constrained, concise, decision-grade
    interface (bounded output, structured feedback, guardrails, syntax checks on
    edit) makes agents dramatically more effective than a raw shell. Directly
    motivates narrow typed output, structured retryable edit errors, and
    syntax/lint-on-edit here.
- Aider repo-map and editblock editing
  - https://github.com/Aider-AI/aider
  - Observed 2026-05-29.
  - Key facts: aider ranks repository context via a tree-sitter repo-map and
    edits via search/replace editblocks with perfect / whitespace-tolerant /
    dotdotdot / edit-distance matching; a failed match returns a structured
    `SearchReplaceNoExactMatch`-shaped error; `auto_lint` runs `lint_edited` after
    an edit and feeds findings back as a reflected message. Motivates the
    apply-patch matching ladder, the structured no-match error, and the
    lint-on-edit reflection loop.
- Codex `apply-patch` crate (local repo)
  - `workpads/references/repos/openai-codex/codex-rs/apply-patch/`
  - Observed 2026-05-29.
  - Key facts: a typed `Hunk` model (`parser.rs`), fuzzy context location via
    `seek_sequence.rs`, and a structured `ApplyPatchError` / `ApplyPatchFailure`
    rather than a raw error string; `apply_patch_tool_instructions.md` documents
    the patch grammar. Proves the typed-hunk + fuzzy-seek + structured-error shape
    for `capo.apply_patch`.
- Codex `file-search` crate (local repo)
  - `workpads/references/repos/openai-codex/codex-rs/file-search/`
  - Observed 2026-05-29.
  - Key facts: a dedicated, bounded file-search utility that keeps locating cheap
    and avoids whole-file dumps. Motivates the bounded `capo.search` + locator
    with per-call and byte caps and an explicit truncation marker.
- Codex `tools` crate (local repo)
  - `workpads/references/repos/openai-codex/codex-rs/tools/`
  - Observed 2026-05-29.
  - Key facts: tools are registered with explicit JSON schemas
    (`tool_definition.rs`, `json_schema.rs`, `tool_spec.rs`) and a typed
    `function_call_error.rs` / `tool_output.rs` result path. Proves the
    input+output schema discipline behind extending `ToolDefinition` with an
    `output_schema` and validating each tool's emitted result.
