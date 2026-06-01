# Depth Tasks

## Objective

Harden and broaden the working harness: a live ACP JSON-RPC adapter
(`initialize`/`session.new`/`prompt`/`update`/`cancel`/`request_permission` plus
`session/resume` and `session/load`), the Claude workspace-write adapter as a
second real provider, a real memory packet/retrieval path
(`MarkdownMemoryBackend` + SQLite FTS5) that replaces the hardcoded packet
strings, a first OS sandbox tier plus git worktree isolation behind the runtime
boundary, and an optional OTel exporter. Each task carries its own true
prerequisite rather than a blanket dependency on every earlier phase, so breadth
starts the moment that prerequisite lands.

## Status

Planned. All tasks pending.

## Feature Set

- A live ACP JSON-RPC adapter below the `AgentAdapter` trait, including the live
  `session/request_permission` wire round-trip and `session/resume` +
  `session/load` reconciliation against the A2a dedupe rules.
- Claude lifted out of no-tools plan mode into a second opt-in workspace-write
  provider, validating the `AgentAdapter` trait against a second provider.
- A real memory packet path: `MarkdownMemoryBackend` source pointers plus a
  `SqliteFtsMemoryBackend` FTS5 retrieval/search that kills the four hardcoded
  packet strings, with extraction + staleness `MemoryJob`s indexing the repo.
- A first OS sandbox tier (macOS seatbelt / linux landlock+bwrap) and git
  worktree isolation per session/goal, both swappable behind the runtime
  boundary.
- An optional, off-by-default OTel exporter for spans/timing across the loop,
  tools, and runtime.
- Differentiated per-task prerequisites: ACP/Claude/FTS5 on
  `real-turn-loop` + `tools-aci`; sandbox/worktree on `safety-gates`;
  worktree-per-goal additionally on `goal-autonomy`.

## DP0 - Workpad, Routing, Scope, And Per-Task Prerequisite + Verification Invariant

Status: pending.

Acceptance:

- Decide that `depth` is its own Phase 6 workpad that DEEPENS the working harness
  rather than unblocking it, and record in `knowledge.md` why these tasks are
  last in the sequence (`real-turn-loop` -> `streaming-transport`/`tools-aci` ->
  `safety-gates` -> `goal-autonomy` -> `depth`).
- List the boundaries this workpad owns (live ACP JSON-RPC adapter incl. the live
  `request_permission` round-trip, Claude as a second write adapter, real
  `MarkdownMemoryBackend` + FTS5 retrieval, first OS sandbox tier, git worktree
  isolation, optional OTel) and the ones it explicitly defers (the loop/turn
  substrate to `real-turn-loop`; SSE/streaming to `streaming-transport`; tool
  registry/edit/patch/redaction to `tools-aci`; `PermissionPolicy`/grant
  lifecycle/`VerificationRunner`/shadow-git to `safety-gates`; the goal model and
  continuation/auditor to `goal-autonomy`; the web client to the web agent).
- Record the DIFFERENTIATED per-task prerequisites: DP1-DP3 (live ACP), DP4
  (Claude), DP5-DP6 (FTS5 memory) and DP10 depend on `real-turn-loop` +
  `tools-aci`; DP7 (OS sandbox) and DP8 (git worktree isolation) depend on
  `safety-gates` checkpoint/recovery; the worktree-per-goal slice of DP8 depends
  additionally on `goal-autonomy`; DP9 (OTel) and DP11 (live smoke) depend on
  their respective subject tasks. State that no task may begin before its own
  true prerequisite lands.
- Record the workpad decisions: ACP stays an adapter below the `AgentAdapter`
  trait and never the domain model; memory retrieval is FTS5-first with
  vector/embeddings deferred; sandbox/worktrees live behind the runtime boundary
  and are swappable; Claude is the second write adapter for breadth; OTel is
  optional and off by default.
- Record the workpad-wide verification invariant: no task completes on operator
  self-attestation alone; deterministic fake/replay tests land before any live
  ACP/Claude/sandbox provider; every manual smoke is paired with a deterministic
  assertion (wire snapshot, exit status, or restart/replay); live ACP/Claude and
  real-sandbox work stays behind explicit opt-in env gates mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE` / `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`, with
  secrets stripped from all evidence.

Evidence:

- `workpads/depth/tasks.md`
- `workpads/depth/knowledge.md`
- `workpads/depth/references.md`

## DP1 - Live ACP JSON-RPC Adapter: initialize/session.new/prompt/update/cancel/request_permission

Status: done.

Prerequisite: `real-turn-loop` + `tools-aci`.

Acceptance:

- Add a live ACP JSON-RPC 2.0 stdio client below the `AgentAdapter` trait (the
  trait defined in `crates/capo-adapters/src/adapter.rs` by `real-turn-loop`),
  promoting `AcpAdapter` from the fixture-only `session_setup_plan`
  (`crates/capo-adapters/src/acp_client.rs:11`) to a real wire client that the
  loop can drive.
- Implement the agent-call surface from `protocol-provider.md`: `initialize`
  (recording negotiated integer `protocolVersion`, currently stable `1`),
  `session/new`, `session/prompt`, `session/cancel`, and ingestion of
  `session/update` notifications, with the ACP process launched through
  `RuntimeRunner` (adapters never own process groups) and attached after start.
- Implement the live `session/request_permission` CLIENT round-trip on the wire:
  map incoming ACP permission options through `capability-permissions.md` into the
  `safety-gates` `PermissionPolicy` decision and answer the agent with the chosen
  option; the live wire round-trip lands HERE, not in `safety-gates` (which scoped
  this to fakes + option mapping only).
- Route ACP `fs/read_text_file`, `fs/write_text_file`, and `terminal/run` client
  calls through the existing `wrapper_request_for_client_call` mapping
  (`acp_client.rs:52`) into the `tools-aci` runtime wrappers, advertising a
  capability only when the backing `ToolDefinition` + scope exist; reject
  un-advertised capabilities as today.
- Normalize ACP `session/update` variants (message/thought chunks, tool calls,
  tool-call updates, plan updates) into `NormalizedAdapterEvent`s through the
  existing `apply_normalized_adapter_events_with_turn` path so the live adapter
  reuses the loop's ingestion route, never a parallel one.
- Keep ACP strictly an adapter: no `session/update` is directly authoritative for
  read models, and Capo does not expose itself as an ACP agent backend.

Verification:

- Deterministic fixture/replay tests in `crates/capo-adapters` driving a scripted
  ACP server transcript (initialize -> session/new -> prompt -> updates ->
  request_permission -> cancel) with no live process.
- Focused `cargo test -p capo-adapters -p capo-server`.
- `cargo fmt`
- Live ACP smoke deferred to DP11; here only the deterministic transcript runs.
- `git diff --check`

Evidence (DP1 landed 2026-06-01):

- New `crates/capo-adapters/src/acp_wire.rs`: a live JSON-RPC 2.0 ACP wire client
  (`AcpWireClient`) over an abstract `AcpTransport`. Implements `initialize`
  (records negotiated integer `protocolVersion`, stable `1`), `session/new`,
  `session/prompt` (pumps interleaved frames), and `session/cancel`. Ingests
  `session/update` notifications through the SAME `AcpAdapter::normalize_update`
  / `parse_acp_record` path the replay fixtures use (no parallel route), and
  implements the LIVE `session/request_permission` CLIENT round-trip on the wire
  via `map_acp_options_trusted_local`, writing the chosen `optionId`/`cancelled`
  back. Ships a deterministic `ScriptedAcpTransport` (no live process) and a
  `PipedProcessTransport` for the runtime-spawned pipes.
- New `crates/capo-adapters/src/acp_live.rs`: `AcpLiveAdapter` (an `AgentAdapter`
  trait impl, `binding.variant = "acp-live"`, real provider) that launches the
  ACP agent through `RuntimeRunner` (`LocalProcessRunner::spawn_piped_process`,
  so the runtime owns the process group) and drives the wire client; fail-closed
  fast behind `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` + `CAPO_SERVER_RUN_ACP_LIVE`.
- `crates/capo-runtime/src/lib.rs`: added `LocalProcessRunner::spawn_piped_process`
  + `PipedRunningProcess` (piped stdin/stdout for a bidirectional line protocol,
  reusing the existing env-scrub/confinement/process-group ownership).
- `provider_parsers.rs` exposes `AcpAdapter::normalize_update`;
  `local_subscription.rs` adds `AcpAdapter::local_launch_plan` (subscription-safe,
  confined). `fs/*`+`terminal/*` client-call routing reuses the existing
  `wrapper_request_for_client_call` mapping (advertise-only capabilities).
- Deterministic tests (no live process): `scripted_transcript_drives_full_acp_flow`
  (initialize -> session/new -> prompt -> updates -> request_permission ->
  response), `client_writes_wellformed_jsonrpc_including_permission_response`,
  `agent_error_response_surfaces_typed_error`,
  `acp_live_adapter_drives_scripted_transcript_to_turn_output`,
  `acp_live_cancel_accepts_late_update_and_finalizes_cancelled`,
  `acp_live_send_turn_fails_closed_fast_when_gate_off`,
  `acp_live_adapter_reports_real_provider_binding`,
  `acp_local_launch_plan_is_subscription_safe_and_confined`.
- Gate (run from `/Users/nicolas/devel/capo-wt/depth`): `cargo fmt --check` PASS;
  `cargo clippy --all-targets --all-features -- -D warnings` PASS;
  `cargo test --workspace` PASS (0 failed across all crates; capo-adapters 45
  passed / 2 ignored). `git diff --check` clean. Live ACP smoke remains deferred
  to DP11.

DP1 review fixes (2026-06-01):

- Inbound agent REQUESTS are now answered, never silently swallowed. The wire
  pump (`acp_wire.rs::pump_until_response`) distinguishes requests (carry `id`)
  from notifications: advertised `fs/read_text_file` / `fs/write_text_file` /
  `terminal/run` requests route through
  `AcpSessionSetupPlan::route_inbound_client_call` -> `wrapper_request_for_client_call`
  (which rejects un-advertised capabilities and validates path/program params) and
  are answered with a JSON-RPC result naming the backing wrapper tool; an
  un-advertised/invalid call is answered with a JSON-RPC error; any other unknown
  request gets a JSON-RPC `-32601` method-not-found. This closes the deadlock where
  an advertised fs/terminal call fell into the catch-all and got no reply.
- The wire pump now has a read deadline (`ACP_PUMP_READ_TIMEOUT`, overridable via
  `AcpWireClient::with_read_timeout`): a stalled/malicious agent yields
  `AcpWireError::Timeout` instead of hanging the controller turn. The live
  `PipedProcessTransport` drains stdout on a reader thread + `mpsc` so reads are
  deadline-bounded without a blocking-fd hang.
- Permission mapping is now profile-honest. `AcpSessionSetupPlan` carries an
  `AcpPermissionProfile`; `answer_permission` applies the documented TrustedLocal
  option-mapping ONLY under the TrustedLocal profile and FAILS CLOSED (cancels) for
  any other profile rather than self-authorizing on the wire. Full per-scope
  `PermissionPolicy::decide` integration for non-trusted profiles stays with the
  controller seam (safety-gates grant lifecycle), not the wire client.
- `AcpLiveAdapter` is wired into the single orchestration seam:
  `AgentAdapterHandle::Acp` + `AgentAdapterHandle::acp(..)`, routed through
  `as_adapter()`/`is_real()` so the loop can dispatch to it (gate-off -> blocked
  turn, no spawn).
- Reuse of the loop's ingestion route is now PROVEN, not self-attested: a new
  capo-controller test drives an `AcpLiveAdapter` transcript and feeds
  `transcript.events` through `apply_normalized_adapter_events_with_turn`,
  asserting the ACP tool call lands as a completed `adapter_native:acp` read-model
  row.
- New deterministic tests: reject-only and empty/no-selectable-option permission
  answers on the wire; non-trusted-local fail-closed; advertised fs/write routed +
  answered; un-advertised fs/write rejected with a JSON-RPC error; unknown request
  method-not-found; pump timeout (no hang); a capo-runtime `spawn_piped_process`
  line-protocol + shutdown/reap test; `AgentAdapterHandle::acp` dispatch.

DP1 gate fix (2026-06-01):

- Fixed a `clippy::large_enum_variant` failure at
  `crates/capo-adapters/src/adapter.rs`: the `AgentAdapterHandle::Acp` variant
  inlined the large `AcpLiveAdapter` (>= 624 bytes), bloating every handle. Boxed
  the large variant to `Acp(Box<AcpLiveAdapter>)`; the boxing is encapsulated in
  the `AgentAdapterHandle::acp(..)` constructor (`Box::new`) and the `as_adapter()`
  match derefs via `as_ref()`, so no callers changed.
- Gate re-run from `/Users/nicolas/devel/capo-wt/depth`: `cargo fmt --check` PASS;
  `cargo clippy --all-targets --all-features -- -D warnings` PASS (no warnings);
  `cargo test --workspace` PASS (0 failed; capo-adapters 52 passed / 2 ignored).
  `git diff --check` clean.

## DP2 - ACP session/load + session/resume And Raw-Update Reconciliation/Dedupe

Status: done.

Prerequisite: `real-turn-loop` + `tools-aci` (builds on DP1).

Acceptance:

- Implement `session/resume` as the default reconnect path when the agent
  advertises `sessionCapabilities.resume` and Capo already holds local history:
  emit `adapter.attach_started` -> `session/resume` -> store the response as raw
  metadata -> `adapter.attach_completed`, creating NO message/item replay events
  (per `acp-replay-dedupe.md`).
- Implement `session/load` only for foreign import, repair/reconciliation, or
  resume-less agents: open an `AcpReplayBatch`, persist every `session/update` as
  an `AcpRawUpdate` before normalization, stage candidate items in a
  non-projecting replay workspace, finalize on the load response, then reconcile.
- Implement the reconciliation rules: stable timeline-key match -> accepted
  update or duplicate marker; content-hash + anchor match -> duplicate
  observation; no match -> import missing item; ambiguous -> low-confidence import
  or quarantine; emit `adapter.replay_started` / `adapter.replay_completed` with
  imported/duplicate/ambiguous counts.
- Use the protocol-aware `AcpTimelineKey` shapes from `acp-replay-dedupe.md`
  (`acp:{session}:tool:{toolCallId}`, `acp:{session}:plan:current`, message keys
  with `message_boundary_confidence` when stable IDs are absent), and treat
  `tool_call_update` `content`/`locations` as replacement fields, ACP plans as
  full replacements.
- Add the `adapter_replay_batches` / `adapter_raw_updates` / `adapter_timeline_keys`
  tables and the `adapter.replay_*` / `adapter.attach_*` event kinds in
  `crates/capo-state` if not already present, each with an idempotency key and
  projection; raw updates never mutate read models directly.
- Keep Capo restart recovery and ACP load replay as SEPARATE phases: recovery
  establishes local event truth first, then ACP replay reconciles against it.

Verification:

- Deterministic replay tests covering the `acp-replay-dedupe.md` prototype
  fixtures: resume-after-restart adds no items; load replaying known history adds
  no duplicate UI items; foreign load imports once; repeated identical
  `tool_call_update` yields one read model with raw duplicates; ID-less
  consecutive same-type chunks record low boundary confidence.
- Restart/replay test proving reconciled projections rebuild identically.
- Focused `cargo test -p capo-adapters -p capo-state`.
- `cargo fmt`

Evidence (DP2 review fixes 2026-06-01):

- The prior DP2 commit landed ONLY the `capo-state` read-model scaffolding (event
  kinds, 3 tables, 3 projections, codec) with NO producer and NO tests, so the 8
  event kinds and 3 projections were dead code and the behavioral acceptance
  (resume/load/reconciliation) was unimplemented. This review fix lands the
  missing behavior + verification suite.
- New `crates/capo-adapters/src/acp_replay.rs`: the deterministic
  reconciliation REDUCER (`AcpReplayEngine`). Pure, storage-free: given the raw
  `session/load` frames (or the empty resume-attach stream) plus the fingerprints
  of items Capo already holds, it produces an `AcpReplayPlan` (batch summary,
  ordered `AcpRawUpdateRecord`s, derived `AcpTimelineKeyRecord`s, and per-candidate
  `AcpReconcileDecision`: imported / duplicate / ambiguous). Tool calls dedupe by
  stable `acp:{session}:tool:{id}`; `tool_call_update` content/locations are
  replacement fields (repeated identical updates collapse to ONE candidate while
  every raw frame is retained); plans use `acp:{session}:plan:current`; ID-less
  consecutive same-type message chunks finalize to a content hash with `low`
  boundary confidence and import as ambiguous.
- `crates/capo-adapters/src/acp_wire.rs`: real `session_resume` /
  `session_load` wire methods on `AcpWireClient` (resume returns metadata only and
  yields NO item events; load pumps the full history through the SAME
  `parse_acp_record` route). The transcript now retains the RAW `session/update`
  frames before normalization for the engine.
- `crates/capo-controller/src/acp_replay_ingest.rs`: the single orchestration
  PRODUCER (`ingest_acp_replay_plan`) that turns a plan into durable events +
  projections at the controller seam, mirroring the
  `PermissionApproval`/`CapabilityGrant` pattern: opens the batch
  (`adapter.replay_started`/`adapter.attach_started`), appends one
  `adapter.raw_update_observed` + `AdapterRawUpdate` per frame, records each
  `AdapterTimelineKey`, emits `adapter.replay_duplicate_detected` /
  `adapter.replay_ambiguous` markers (no item events) or imports missing items via
  the shared `apply_normalized_adapter_events` route, then finalizes
  (`adapter.replay_completed`/`adapter.attach_completed`) with the counts. Every
  event carries the design's `acp:{session}:{family}:{key}:{op}` idempotency key.
  `acp_existing_item_fingerprints` reads Capo's durable `adapter_timeline_keys`
  (Capo owns identity; the reducer never touches storage).
- `crates/capo-state/src/queries.rs`: `adapter_replay_batches_for_session`,
  `adapter_raw_updates_for_batch`, `adapter_timeline_keys_for_session`.
- Tests (all run under the gate): capo-adapters
  `dp2_session_resume_adds_no_items`, `dp2_session_load_pumps_raw_history_then_reconciles`,
  `repeated_tool_call_update_collapses_to_one_candidate_with_raw_duplicates`,
  `idless_consecutive_chunks_record_low_boundary_confidence`,
  `load_of_known_tool_history_is_duplicate_not_reimport`,
  `load_of_known_message_matches_by_content_hash`,
  `plan_updates_collapse_to_single_current_plan_candidate`; capo-state
  `dp2_acp_replay_event_kinds_round_trip` (8 kinds as_str/from_wire) and the three
  `dp2_adapter_*_projection_persists_and_rebuilds_identically`; capo-controller
  `dp2_session_resume_attach_adds_no_items_but_records_attach_batch`,
  `dp2_foreign_session_load_imports_each_item_once_then_rebuilds_identically`
  (restart/replay rebuild-identically proof for all 3 read models + imported
  items), `dp2_load_of_known_history_adds_no_duplicate_ui_items`.
- Gate re-run from `/Users/nicolas/devel/capo-wt/depth`: `cargo fmt --check` PASS;
  `cargo clippy --all-targets --all-features -- -D warnings` PASS (no warnings);
  `cargo test --workspace` PASS (0 failed across all crates + doc-tests). The new
  tests fail-for-the-right-reason without the producer/engine (the projections are
  unreachable and the counts/markers are absent); they pass with it.

## DP3 - ACP Raw-Update Storage, Provenance, And Cancel-While-Permission-Pending

Status: gate fixed 2026-06-01 (fmt/clippy green); behavioral acceptance still in
progress.

Prerequisite: `real-turn-loop` + `tools-aci` (builds on DP1/DP2).

Acceptance:

- Persist raw ACP updates as artifacts/rows with the
  `external_ref.adapter = "acp"` provenance block from `acp-replay-dedupe.md`
  (`external_session_id`, `acp_update_kind`, `acp_replay_batch_id`,
  `acp_raw_update_id`, `acp_timeline_key?`, `replay_source`) and the
  `acp:{capo_session_id}:{event_family}:{timeline_key}:{operation}:{operation_version}`
  idempotency-key shape on every normalized ACP event.
- Stage ID-less message chunks outside the append-only log, finalize them to a
  normalized `content_hash` + `chunk_count` at turn/load completion, and append a
  duplicate-observation event (not item events) when a finalized candidate matches
  an existing Capo item by role + content hash + surrounding anchors.
- Implement the cancel-while-permission-pending fixture path: after
  `session/cancel`, accept late `session/update`s, answer the pending ACP
  permission as `cancelled`, close the Capo permission queue, and finalize the
  turn with stop reason `cancelled`.
- Store large ACP payloads as artifact refs rather than inline JSON, and pass any
  stored payloads through the existing redaction/credential scan before persisting.
- Record `dedupe_confidence` (`stable` / `heuristic` / `none`) and
  `import_confidence` so low-confidence reconciliations are auditable rather than
  silently projected.

Verification:

- Deterministic test asserting the normalized idempotency-key shape and the
  `external_ref.adapter = "acp"` provenance on a scripted transcript.
- Deterministic cancel-while-permission-pending fixture proving the pending
  permission is answered `cancelled` and late updates are accepted before
  finalization.
- Focused `cargo test -p capo-adapters -p capo-state`.
- `cargo fmt`
- `git diff --check`

Evidence (DP3 gate fix 2026-06-01):

- The objective gate had FAILED on two stale-WIP issues: a `cargo fmt` multi-line
  chain at `crates/capo-adapters/src/event.rs:191` and a clippy dead-code error
  for the `AcpImportConfidence` enum + its `as_str`/`from_dedupe` methods in
  `crates/capo-adapters/src/acp_replay.rs` (introduced for DP3's
  `import_confidence` auditability requirement but never wired up).
- Fix (smallest correct change that also advances DP3's acceptance rather than
  deleting the required type):
  - `crates/capo-adapters/src/event.rs`: collapsed the `operation` assignment chain
    to a single line per rustfmt.
  - `crates/capo-adapters/src/acp_replay.rs`: added an `import_confidence:
    AcpImportConfidence` field to `AcpReconciledCandidate`, computed in `plan_load`
    via `AcpImportConfidence::from_dedupe(boundary_confidence)` (stable-keyed import
    -> `stable`, single inferred message group -> `heuristic`, ambiguous
    collapsed-chunk import -> `none`), so a low-confidence reconciliation is
    auditable rather than silently projected.
  - `crates/capo-adapters/src/lib.rs`: re-exported `AcpImportConfidence`.
  - `crates/capo-controller/src/acp_replay_ingest.rs`: the duplicate/ambiguous
    reconciliation markers now carry `candidate.import_confidence.as_str()` in their
    operation string so the confidence is recorded on the event, exercising the
    previously-dead `as_str`.
  - New deterministic test `acp_replay::tests::reconciled_candidates_record_auditable_import_confidence`
    asserting a stable tool import records `stable` and an ID-less collapsed-chunk
    ambiguous import records `none`.
- Gate re-run from `/Users/nicolas/devel/capo-wt/depth`: `cargo fmt --check` PASS;
  `cargo clippy --all-targets --all-features -- -D warnings` PASS (no warnings);
  `cargo test --workspace` PASS (0 failed across all crates + doc-tests;
  capo-adapters 64 passed / 2 ignored including the new test). `git diff --check`
  clean.
- Note: this commit makes the objective gate green and lands the
  `import_confidence` auditability slice of DP3; the remaining DP3 behavioral
  acceptance (raw-update artifact provenance block + `external_ref.adapter="acp"`
  on normalized events, ID-less staging/finalize at turn/load completion, the
  cancel-while-permission-pending fixture, artifact-ref + redaction for large
  payloads) remains to be completed.

## DP4 - Claude Workspace-Write Adapter As A Second Real Provider (Opt-In Gated)

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci`.

Acceptance:

- Implement `ClaudeCodeAdapter` as a second real `AgentAdapter`-trait
  implementation alongside Codex, lifting Claude out of the current no-tools plan
  mode into a workspace-write profile, validating the trait against a second
  provider (breadth, not a re-architecture of the loop).
- Launch Claude through `RuntimeRunner` using the observed `protocol-provider.md`
  surface: `claude -p --output-format stream-json --verbose`, scrubbing
  unrelated `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` per the connector env
  policy, and routing through the live-provider preflight
  (`crates/capo-server/src/live_provider.rs`, `claude` is already a supported
  preflight kind at `:70` and `util.rs:39`).
- Parse Claude `stream-json` records into `NormalizedAdapterEvent`s (items, turns,
  tool calls, usage), mapping Claude `session-id` to `external_session_ref` and
  Claude tool/permission events to Capo tool calls / permission requests; use
  content hashes + ordinal anchors with low confidence where stream JSON lacks
  stable IDs.
- Gate live Claude writes behind an explicit opt-in env gate mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE` (e.g. `CAPO_SERVER_RUN_CLAUDE_LIVE`) AND the
  `real-turn-loop` confinement/checkpoint/ceiling safety floor; dry-run/diff
  preview remains default; if native tool-result delivery is unsupported, record
  observed-only tool results.
- Add a deterministic mock-output test using the
  `mock_provider_output_jsonl`-style fixture so the Claude write round-trip is
  fully testable without a live provider, asserting normalized-event shape parity
  with the Codex adapter at the trait seam.

Verification:

- Deterministic `stream-json` fixture test for the Claude write round-trip in
  `crates/capo-adapters`.
- Focused `cargo test -p capo-adapters -p capo-server`.
- `cargo fmt`
- Live Claude smoke deferred to DP11 behind the opt-in gate with secrets stripped.
- `git diff --check`

## DP5 - Real Memory Packet Path: MarkdownMemoryBackend + FTS5 Retrieval

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci`.

Acceptance:

- Add `MarkdownMemoryBackend` and `SqliteFtsMemoryBackend` variants to the
  `MemoryBackend` enum in `crates/capo-memory/src/lib.rs` (today only
  `Fake(FakeMemoryBackend)`), keeping the existing
  `build_source_linked_packet` provenance/inclusion-reason/exclusion semantics.
- Build the live packet from REAL retrieved sources, killing the hardcoded packet
  strings: `MarkdownMemoryBackend` supplies workpad/source pointers with content
  hashes; `SqliteFtsMemoryBackend` provides FTS5 search/ranking over
  `memory_records`, selected artifacts, and workpad sections per
  `memory-architecture.md`.
- Wire the strict eligibility filter that is currently dead in production:
  `SqliteQueries::packet_eligible_memory_records`
  (`crates/capo-state/src/queries.rs:846`) and
  `MemoryRecordProjection::is_packet_eligible` (`projections.rs:394`) must gate
  packet candidates so invalidated/rejected/superseded/secret/unreviewed records
  are excluded, and the candidate set feeds the loop's turn-context packet rather
  than literals.
- Implement the `MemoryBackend` `search(MemoryQuery, MemoryBudget)` contract from
  `memory-architecture.md` over FTS5 (filtering out invalidated/rejected/
  superseded/unauthorized/redacted records unless explicit scope), with a real
  token-budget selection that retains the existing per-item inclusion reasons and
  excluded-reason decisions.
- Add `memory.index_updated` / `memory.packet_built` / `memory.packet_attached`
  events (and the `memory_index_entries` / `memory_packets` tables) where absent,
  and keep the packet artifact replayable: the attached packet must reconstruct
  exactly from its `packet_artifact_id`.
- Keep vector/embeddings/graph backends deferred: no vector DB is required for
  this first retrieval path.

Verification:

- Deterministic FTS5 retrieval tests proving search ranking, eligibility-filter
  exclusion of secret/unreviewed/superseded records, and budget-bounded selection.
- Restart/replay test proving rebuilt FTS indexes return the same searchable
  record IDs and the attached packet replays byte-for-byte from its artifact.
- Focused `cargo test -p capo-memory -p capo-state`.
- `cargo fmt`
- `git diff --check`

## DP6 - Memory Extraction + Staleness MemoryJob And Index The Working Repo

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci` (builds on DP5).

Acceptance:

- Implement the `extract_facts`, `index_fts`, `invalidate`, and `rebuild`
  `MemoryJob` kinds from `memory-architecture.md`, each attaching at least one
  `MemorySource` provenance edge (`ingest`/`extract` must never create a record
  without a source), with `memory.job_requested` / `memory.job_completed` events.
- Index the working repo's markdown sources (`workpads/**/knowledge.md`, source
  docs) into `memory_records` + `memory_sources` with content hashes, reusing the
  existing non-destructive markdown indexer rather than overwriting human truth.
- Implement staleness detection: when an indexed source's `source_content_hash`
  drifts, emit `memory.record_invalidated` / `memory.record_superseded` and
  exclude stale records from packets by default (matching `valid_until` /
  `revoked_by` exclusion in `build_source_linked_packet`).
- Keep generated records untrusted: extracted facts land in `review_state =
  generated` and cannot supersede a reviewed workpad decision without
  `memory.record_promoted`; secrets, credentials, subscription sessions, and raw
  voice transcripts are rejected as memory sources by default.
- Make extraction/index/rebuild idempotent with source-range idempotency keys so
  re-running a job over the same range produces the same record IDs.

Verification:

- Deterministic tests: a record cannot be ingested without provenance; a drifted
  source hash invalidates/supersedes and is excluded from the next packet;
  credential/voice-transcript sources are rejected.
- Rebuild test proving re-indexing the repo from source ranges yields identical
  searchable record IDs.
- Focused `cargo test -p capo-memory -p capo-state`.
- `cargo fmt`

## DP7 - First OS Sandbox Tier (Seatbelt / Landlock+Bwrap) Behind The Runtime Boundary

Status: done (2026-06-01).

Prerequisite: `safety-gates` (checkpoint/recovery + enforced `PermissionPolicy`).

Acceptance:

- Add a real OS sandbox tier as a swappable option behind the `RuntimeRunner`
  boundary (the `runtime-tunnel.md` enum), enforcing filesystem/network confinement
  through actual OS mechanisms rather than delegating to the provider CLI's own
  `--sandbox` flag; Capo only claims hard sandboxing where the OS actually enforces
  it and a test proves it.
- Ship the first tier as macOS seatbelt for the dev box AND a linux
  landlock+bwrap path for CI, recording in `knowledge.md` which gates first
  (open question DP-OQ2); model after the codex `sandboxing` crate
  (seatbelt `.sbpl` base/network policies; landlock + bwrap launcher) referenced
  in `references.md`.
- Wire the sandbox decision to the `safety-gates` `PermissionPolicy`/capability
  scopes so a confined run's filesystem-write and network-egress scopes match the
  granted capability profile; an un-granted critical scope is denied before the
  sandbox launches.
- Implement deterministic REFUSAL-mode tests: a write outside the confined root is
  refused by the sandbox (not just by Capo's path-prefix check), and a network
  egress attempt is refused when the profile forbids it; the refusal is recorded
  as an event, not a silent failure.
- Keep the sandbox composable with the `real-turn-loop` path confinement and
  pre-write checkpoint: the sandbox is an ADDITIONAL enforcement layer, and a
  successful confined run still produces a reversible checkpoint.

Verification:

- Deterministic sandbox refusal-mode tests (out-of-root write refused, egress
  refused) behind a platform gate so they run on the supported OS only.
- Focused `cargo test -p capo-runtime`.
- `cargo fmt`
- Live sandboxed-run smoke deferred to DP11 behind the opt-in gate.
- `git diff --check`

Must not do:

- Do not claim sandboxing on a platform where Capo cannot enforce it; record the
  platform limitation instead.

Evidence (DP7 landed 2026-06-01):

- New `crates/capo-runtime/src/sandbox.rs`: a first OS sandbox tier as a SWAPPABLE
  option behind the `RuntimeRunner` boundary (`OsSandbox` + `SandboxTier::{None,
  MacosSeatbelt, LinuxLandlockBwrap}`), enforcing filesystem/network confinement
  through actual OS mechanisms rather than the provider CLI's `--sandbox` flag.
  macOS seatbelt is launched via `/usr/bin/sandbox-exec -f <generated .sbpl>`
  (`(deny default)` + `(allow file-read*)` + `(allow file-write* (subpath ...))`
  per confined root + `(deny network*)`/`(allow network*)` per profile), modeled
  after the codex `sandboxing` seatbelt base/network policy. The linux path
  rewrites the request to launch through `bwrap` (read-only `/` bind, read-write
  bind per confined root, `--unshare-net` when egress is forbidden), modeled after
  the codex `linux-sandbox`/`bwrap` crates.
- `SandboxTier::is_enforced_here()` gates the claim: seatbelt enforces only on
  `cfg!(target_os = "macos")`, landlock+bwrap only on linux. On a non-supporting
  platform the plan reports `SandboxEnforcement::Unenforced` and emits a
  `sandbox.unenforced` event -- Capo does NOT claim sandboxing where the OS cannot
  enforce it (the DP7 "Must not do").
- The sandbox decision is wired to the `safety-gates` capability scopes through
  `SandboxProfile` (confined `writable_roots` = `filesystem:write:workspace`,
  `allow_network_egress` = the `network:connect:*` critical scope). `OsSandbox::plan`
  refuses an un-granted critical scope BEFORE the sandbox launches: a run that
  requires egress under a network-forbidding profile, or a cwd outside the confined
  writable roots, yields `SandboxEnforcement::Refused` + a `sandbox.launch_refused`
  event with NO planned process (an event, not a silent failure).
- Composes with the `real-turn-loop` path confinement: `OsSandbox::run` rewrites
  the request to launch under the OS launcher and runs it through the SAME
  `LocalProcessRunner`, reusing env-scrub, `ensure_cwd_allowed`, redaction, and
  artifact capture; a successful confined run still produces the runner's normal
  artifact/exit-status surface (an ADDITIONAL enforcement layer, not a replacement).
- Deterministic REFUSAL-mode tests (platform-gated to macOS so they run on the
  supported OS only): `seatbelt_refuses_out_of_root_write_at_the_os_layer` (a shell
  write to a path OUTSIDE the confined root fails with non-zero exit and the file
  is absent -- refused by the seatbelt sandbox itself, not just Capo's path-prefix
  check), `seatbelt_refuses_network_egress_at_the_os_layer` (an `nc` connect is
  blocked at the OS layer with the pre-launch gate intentionally bypassed),
  `seatbelt_allows_in_root_write` (the confinement is additional, not blanket
  denial), `seatbelt_policy_denies_network_and_confines_writes` (policy shape).
  Platform-independent: pre-launch refusals (egress-forbidden, write-outside-root)
  and the enforcement-claim/host-default gates.
- DP-OQ2 resolved in `knowledge.md`: macOS seatbelt gates first on the dev box;
  linux landlock+bwrap is the CI tier.
- Gate (run from `/Users/nicolas/devel/capo-wt/depth`): `cargo fmt --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo test --workspace` all pass. Live sandboxed-run smoke remains deferred to
  DP11.

## DP8 - Git Worktree Isolation Per Session/Goal

Status: pending.

Prerequisite: `safety-gates` (checkpoint/recovery); worktree-PER-GOAL additionally
depends on `goal-autonomy`.

Acceptance:

- Add git worktree isolation as a swappable option behind the runtime boundary so
  a session's workspace-write run executes in a dedicated `git worktree` rather
  than the operator's live working tree, keeping the `real-turn-loop` workspace
  confinement scoped to that worktree root.
- Implement worktree lifecycle as events: create-on-session-start,
  reconcile/merge-back point, and teardown, recorded so a worktree can be
  reconstructed/inspected after restart and never silently abandoned.
- Compose worktree isolation with the `safety-gates` single-writer workspace
  lock and pre-write checkpoint: each worktree holds its own write lease, and a
  rollback restores the worktree to its pre-write checkpoint.
- For the worktree-PER-GOAL slice (gated on `goal-autonomy`), bind a worktree to a
  `Goal`/`GoalAttempt` so a continued goal reattaches to its existing worktree and
  parent/child subgoals can be isolated; do NOT change the goal model itself
  (that is `goal-autonomy`).
- Add a deterministic test proving two concurrent sessions write to distinct
  worktrees with no cross-contamination, and a rollback test restoring one
  worktree without disturbing the other.

Verification:

- Deterministic worktree-isolation and rollback tests (two sessions, distinct
  worktrees, independent restore).
- Restart/replay test proving worktree lifecycle events rebuild and a worktree is
  reattachable, not orphaned.
- Focused `cargo test -p capo-runtime -p capo-server`.
- `cargo fmt`
- `git diff --check`

## DP9 - Optional OTel Exporter For Spans/Timing Across Loop/Tools/Runtime

Status: pending.

Prerequisite: the subject surfaces (`real-turn-loop` loop, `tools-aci`
instrumentation, `capo-runtime`) exist; OTel is additive observability.

Acceptance:

- Add an optional OpenTelemetry exporter, OFF by default, that emits spans/timing
  across the controller turn loop, tool invocations, and runtime process
  lifecycle, configured behind an explicit env/config flag (no spans leave the
  process unless enabled); model after the codex `otel` crate (config / otlp /
  provider / trace_context) referenced in `references.md`.
- Add real WALL-CLOCK timing alongside the existing event-sequence-delta duration
  in `crates/capo-eval` (`duration_sequence_span`, `lib.rs:123`), so outcome
  reports carry both sequence span and elapsed wall-clock; spans correlate to
  `run_id` / `turn_id` / `tool_call_id`.
- Apply the existing redaction guard to span attributes before export: a known
  secret must never appear in an exported span, mirroring the redaction-on-emit
  discipline used on the streaming path.
- Keep OTel strictly optional and non-authoritative: disabling it changes nothing
  about event-sourced truth or read models; spans are observability, not state.
- Add a deterministic test asserting that with OTel disabled no exporter is
  constructed and no span data is emitted, and with a fake/in-memory exporter
  enabled the expected loop/tool/runtime spans appear with timing.

Verification:

- Deterministic test with a fake/in-memory span exporter asserting span presence,
  parentage, and wall-clock timing; and a disabled-by-default test.
- Redaction test asserting a known secret never appears in an exported span.
- Focused `cargo test -p capo-eval` (and the crate hosting the exporter).
- `cargo fmt`
- `git diff --check`

## DP10 - Deterministic Fake/Replay Tests For ACP, Memory Retrieval, And Sandbox Refusal Modes

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci` (consolidates DP1-DP9 determinism).

Acceptance:

- Consolidate the deterministic suite that must pass with NO live provider and NO
  real OS sandbox network: ACP transcript replay (DP1-DP3), FTS5 memory retrieval
  + eligibility filtering (DP5-DP6), and sandbox refusal-mode fixtures (DP7) where
  the platform supports a deterministic refusal.
- Assert the ACP replay/dedupe invariants end-to-end: resume adds no items, load
  imports once, repeated tool-call updates yield one read model, ID-less chunks
  record boundary confidence, and cancel-while-permission-pending finalizes
  `cancelled`.
- Assert the memory invariants end-to-end: hardcoded packet strings are gone (the
  packet derives from retrieved + eligibility-filtered sources), secret/unreviewed
  records are excluded, and packets replay exactly from their artifacts.
- Assert sandbox refusal invariants where deterministic: out-of-root write and
  forbidden egress are refused by the sandbox layer and recorded as events.
- Make every assertion replay-stable: a restart/rebuild reproduces identical
  projected state for the ACP, memory, and sandbox-event paths.

Verification:

- Restart/replay tests across the ACP, memory, and sandbox-event paths proving
  identical rebuilds.
- Focused `cargo test -p capo-adapters -p capo-memory -p capo-state -p capo-runtime`,
  widening to `cargo test` if shared state behavior changes broadly.
- `cargo fmt`
- `git diff --check`

## DP11 - Live Opt-In ACP + Sandbox Smoke (Secrets Stripped) Paired With Deterministic Assertions And E2E Gate

Status: pending.

Prerequisite: DP1-DP9 landed with their deterministic suites green.

Acceptance:

- Add a live opt-in ACP smoke behind an explicit env gate (mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE` / `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`),
  separate from ordinary test runs, that runs one real
  initialize/session.new/prompt/update/request_permission/cancel flow against a
  real ACP-compatible agent, plus an optional live Claude write smoke (DP4) and a
  live sandboxed-run smoke (DP7) on the supported OS.
- Pair every live smoke with a deterministic assertion: the same scripted/mock
  fixture asserts the identical normalized-event, artifact, and refusal-mode shape
  so completion is never solely operator-attested.
- Strip secrets from all smoke evidence: artifacts pass the existing credential
  scan (`scan_artifacts_for_sensitive_markers`), and any `unknown` /
  `contains_sensitive` artifact is quarantined or dropped per the artifact privacy
  contract; ACP raw payloads and Claude stream JSON are redacted before retention.
- Confirm the cross-cutting safety floors engage on the live paths: ACP/Claude
  writes run inside the `real-turn-loop` confinement/checkpoint/ceiling and the
  `safety-gates` `PermissionPolicy`; sandbox refusal is enforced by the OS layer.
- Run the focused E2E gate: deterministic ACP replay + memory retrieval + sandbox
  refusal suite, then the gated live smokes, with review notes on architecture fit
  (ACP stays an adapter; sandbox/worktree stay behind the runtime boundary; OTel
  stays optional), provider lock-in, and whether to deepen further or close
  `depth`.

Verification:

- `cargo fmt`
- Focused `cargo test -p capo-adapters -p capo-server -p capo-runtime -p capo-memory`,
  widening to `cargo test` if shared behavior changes broadly.
- Live ACP + optional Claude + sandbox smokes behind explicit opt-in env gates,
  with secrets stripped, each paired with its deterministic fixture assertion.
- `git diff --check`
