# Goal Autonomy Tasks

## Objective

Implement Capo's autonomy differentiator on now-real prerequisites, in two
milestones. Milestone A implements the goal/requirement/evidence event model and
projections and the lifecycle/server/read commands. Milestone B adds the
safe-boundary continuation scheduler, the evidence-gated completion auditor where
agents propose and the auditor decides, continuation context assembly,
reattach-after-compaction, and parent/child plus provider-native delegation as
observed-not-authoritative.

This workpad USES `goal-orchestration` (GO1-GO14) as the authoritative design and
schema source; it implements that design on real prerequisites and does not
re-specify it.

## Status

Planned. Phase 5 of the daily-driver roadmap. Milestone A depends on
`real-turn-loop` and `tools-aci`; Milestone B additionally depends on
`safety-gates`. `GA0` records routing, scope, and the explicit
goal-orchestration reconciliation. All implementation tasks remain pending.

## Feature Set

- Goal/requirement/evidence/review/validation/continuation event model and
  projections, implementing goal-orchestration GO1/GO3 on the real loop.
- Typed server lifecycle/read commands and operator read surfaces for goals,
  story, evidence, review, validation (GO4-GO6, GO10).
- Sourced continuation context packet and continuation prompt assembly (GO7).
- Safe-boundary continuation scheduler as a pure opt-in state machine consuming
  the `safety-gates` workspace lock, with no-progress/spin/budget guards (GO8).
- Evidence-gated completion auditor: agents propose, the auditor decides
  goal-complete on observed evidence (GO9).
- Reattach-after-compaction that re-injects objective and audit contract on
  restart (GO13).
- Parent/child subgoal reporting and provider-native goal delegation recorded as
  observed evidence, never authoritative Capo completion (GO11/GO12).

## GA0 - Workpad, Routing, Scope, And Goal-Orchestration Reconciliation

Status: pending.

Acceptance:

- Record the scope decision: this workpad IMPLEMENTS `goal-orchestration`
  (GO1-GO14) on real prerequisites; it cites that design and does not duplicate
  or re-author its prose.
- State the two-milestone split and its prerequisites explicitly: Milestone A
  (goal/requirement/evidence model + projections + lifecycle/server/read
  commands) depends only on `real-turn-loop` + `tools-aci`; Milestone B
  (continuation scheduler + auditor + context assembly + reattach +
  parent/child + provider delegation) additionally depends on `safety-gates`.
- Map each `GA<N>` task to the `goal-orchestration` task(s) it realizes
  (for example GA1 -> GO1/GO3, GA2 -> GO4/GO5/GO6/GO10, GA3 -> GO7, GA4 -> GO8,
  GA5 -> GO9, GA6 -> GO13, GA7 -> GO11/GO12) so design and code converge.
- Declare the cross-workpad seams: `tools-aci` pre-lands the GO2 reporting tool
  surface (emission + fakes) and the `source=agent_reported` vs observed tagging;
  `safety-gates` owns checkpoint/rollback, verification scoring, permission
  enforcement, and the single-writer workspace lock that GA4 consumes.
- Record the workpad invariant: no task completes on operator self-attestation
  alone; every manual smoke is paired with a deterministic assertion (wire
  snapshot, exit status, or replay), matching `goal-orchestration` discipline.
- List the open questions (continuation budget as RTL ceiling extension vs a
  separate composing `GoalBudget`; whether Milestone A ships and gates
  independently before Milestone B or shares one close-out gate).

Verification:

- Workpad files reviewed against `goal-orchestration/tasks.md` and `knowledge.md`
  for one-to-one task mapping and zero design duplication.
- `git diff --check`.

Must not do:

- Do not edit shared files (`TASKS.md`, `WORKPADS.md`, `AGENTS.md`,
  `WORKING.md`).
- Do not re-design the goal/report/evidence domain model owned by
  `goal-orchestration`.

## GA1 - Milestone A: Goal/Requirement/Evidence Event Model And Projections

Status: done. Goal lifecycle/requirement/continuation/delegated-provider events
(`event.rs`) and the five `ProjectionRecord` variants + structs
(`projections.rs`) are wired end to end: schema tables (`schema.rs`), the
encode/decode round-trip (`codec_encode.rs`/`codec.rs`), the in-transaction
projection apply (`apply.rs`), and typed read methods (`queries.rs`). Agent
reports persist as `source=agent_reported` with confidence distinct from observed
evidence (`runtime_output`/`adapter_event`); duplicate report submissions dedupe
on idempotency key; a full rebuild reproduces the goal/requirement/report
projections identically.

Evidence:

- `cargo fmt --check` -> clean (exit 0).
- `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0 (the prior
  non-exhaustive-match failures in `apply.rs`/`codec_encode.rs` for the new
  `ProjectionRecord` variants are resolved).
- `cargo test -p capo-state goal` -> 2 passed
  (`goal_projections_are_persisted_and_rebuild_identically`,
  `duplicate_goal_report_submission_is_idempotent`).
- `cargo test --workspace` -> exit 0, 0 failed across all crates (capo-state lib:
  64 passed).
- `git diff --check` -> clean.

Acceptance:

- Add append-only `EventKind` variants in `crates/capo-state/src/event.rs` for
  the goal lifecycle (goal created/updated/paused/resumed/blocked/cleared),
  requirement status, continuation decisions, and delegated-provider goal state,
  realizing `goal-orchestration` GO1/GO3 schema; reuse the existing
  `EvidenceRecorded`/`ReviewFindingRecorded` events and the `tools-aci`
  agent-report events rather than redefining evidence.
- Add `ProjectionRecord` variants and structs in
  `crates/capo-state/src/projections.rs` for active goals (`GoalProjection`),
  per-requirement status (`RequirementLedgerProjection`), agent story, evidence
  ledger, review ledger, validation ledger, confidence/risk summary, and current
  blocker state, projected in-transaction like the existing ~30 projections.
- Persist agent reports and provider-native completion as
  `source=agent_reported` (with confidence) distinct from observed evidence
  (`source=runtime_output`/`adapter_event`), so completion is never reachable by
  agent assertion alone; cite `tools-aci` for the tagged report events.
- Preserve raw adapter/provider data as inputs/artifacts, not authoritative
  read-model truth, per `state-model.md`.
- Define idempotency keys and external refs for every new event so duplicate
  report submissions and replay are deduped.
- New goal/continuation events reconcile with the existing dispatch run-exit and
  execution-status semantics (`append_dispatch_run_exit`,
  `AdapterDispatchExecutionProjection`); a goal attempt references dispatch run
  identity rather than introducing a second run-completion notion.

Verification:

- Focused Rust tests for event append, projection, and rebuild:
  `cargo test -p capo-state goal`.
- Idempotency test for duplicate report submission and a full projection rebuild
  test proving goal/requirement/evidence projections rebuild identically.
- `cargo fmt`.
- `git diff --check`.

Must not do:

- Do not store agent-reported claims indistinguishably from observed evidence.
- Do not fork a second run/turn completion notion separate from the existing
  dispatch execution-status projections.

## GA2 - Milestone A: Server Lifecycle/Read Commands And Operator Read Surfaces

Status: in progress (server side done and gate-green; CLI operator_control goal
read surfaces still pending). The typed `crates/capo-server` goal lifecycle
mutations (`SetGoal`/`PauseGoal`/`ResumeGoal`/`BlockGoal`/`ClearGoal`/
`SetRequirementStatus`/`RecordGoalReport`) and read commands (`ListGoals`/
`ViewGoal`/`GoalStory`/`GoalTimeline`/`GoalEvidence`/`GoalValidations`/
`GoalReviews`/`GoalRisks`/`GoalReport`) are wired end to end through the
server/controller boundary: typed requests + responses (`types.rs`), the
encode/decode round-trip and missing goal codec helpers (`transport/codec.rs`),
the published JSON-RPC schema enums and regenerated `contract/jsonrpc-schema.json`
snapshot, and the wire error mapping for the four GA2 `ServerError` kinds
(`unknown_goal`, `goal_complete_not_a_lifecycle_command`,
`illegal_goal_status_transition`, `unclassifiable_report_source`). Goal-complete
is rejected by construction (the GA5 auditor is the only path); a `validated`/
`reviewed` requirement on an agent claim alone is rejected; historical reports
render as markdown and JSON without inlining raw artifact bodies. NOT yet done:
the `crates/capo-cli` operator_control goal read surfaces (`goals`, `goal [GOAL]`,
`story`, `timeline`, `evidence`, `validations`, `reviews`, `risks`) and the
`cargo test -p capo-cli --test server_transport goal` control-through-server
tests; those remain for a follow-up before GA2 closes.

Evidence:

- Root cause of the failed gate: the GA2 goal commands/payloads/error variants
  were added to `types.rs` but the dependent code lagged -- `transport/codec.rs`
  called 9 goal encode/decode helpers that did not exist, `transport/wire.rs` and
  `transport/contract.rs` did not map/publish the 4 new error kinds, the
  `tests/contract.rs` exhaustive matches did not cover the new command/payload/
  error variants, and a borrow error (`cannot move out of goal because it is
  borrowed`) sat in `goal_commands.rs::handle_goal_lifecycle`. Fixed all of these
  with the smallest correct change and regenerated the checked-in JSON-RPC schema
  snapshot via `CAPO_REGENERATE_WIRE_SNAPSHOTS=1`.
- Added deterministic server-boundary tests in `crates/capo-server/src/tests/
  goal.rs` (7 tests): lifecycle mutations drive the read model; direct
  mark-complete is rejected; an unknown-goal lifecycle command is rejected; a
  `validated`-on-`agent_reported` requirement is rejected while `supported`-on-
  observed is accepted; an unclassifiable report source is rejected; story vs
  evidence surfaces separate claims from observed evidence; markdown + JSON
  historical reports render without leaking raw bodies.
- `cargo fmt --check` -> exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.
- `cargo test -p capo-server goal` -> 9 passed (7 new GA2 + 2 pre-existing).
- `cargo test --workspace` -> exit 0, 0 failed across all crates (capo-server lib:
  103 passed, 3 ignored).
- `git diff --check` -> clean.

Acceptance:

- Add typed server requests in `crates/capo-server` for goal lifecycle
  (create/set, view, pause/resume, clear/cancel, mark-blocked-with-reason) and
  for recording each report/evidence/review/validation event, realizing
  `goal-orchestration` GO4/GO6; all mutations flow through the
  server/controller boundary, not a client.
- Goal-complete is NOT an ordinary lifecycle command: the only path to a
  Capo goal-complete transition is the GA5 auditor; add a test asserting a
  direct "mark complete" server request is rejected.
- Link goals to project, task, agent, session, parent goal, and requirements,
  and store success criteria, constraints, verification surface, budget, and
  stop conditions as structured state per GO6.
- Add typed query commands for goal status, agent reports, story, timeline,
  evidence, review, validation, and historical report projections (GO4/GO10).
- Add operator control read surfaces in `crates/capo-cli` operator_control
  (`goals`, `goal [GOAL]`, `story [AGENT|GOAL]`, `timeline`, `evidence`,
  `validations`, `reviews`, `risks`) realizing GO5; keep normal output concise
  and human-readable with raw metadata behind `details`, reusing the existing
  static-dispatch renderer boundary.
- Generate historical execution reports (GO10) rebuildable from events,
  projections, and artifacts, exportable as markdown and JSON, degrading clearly
  when artifacts are missing or redacted.

Verification:

- Server request/response tests with fake agents for each mutation and query
  path: `cargo test -p capo-server goal`.
- Control-through-server tests with scripted stdin and mocked data:
  `cargo test -p capo-cli --test server_transport goal`.
- Snapshot/golden test for markdown and JSON historical report rendering.
- A test asserting illegal lifecycle transitions and direct mark-complete are
  rejected.
- `cargo fmt`.

Must not do:

- Do not let the CLI or any client own goal lifecycle or scheduler state.
- Do not expose raw provider transcripts in default operator output.

## GA3 - Milestone B: Sourced Continuation Context Packet And Prompt Assembly

Status: done. The sourced continuation context packet + continuation prompt
assembly (GO7) is implemented controller-side in
`crates/capo-controller/src/continuation_context.rs` as a pure, read-only view
over persisted goal state. `FakeBoundaryController::continuation_context_packet`
(and `_with_limits`) reconstruct the active objective + audit contract (objective,
status, success criteria, constraints, verification surface, stop conditions,
current blocker, and the per-requirement ledger with observed-vs-reported
provenance) STRICTLY from the GA1 `goals`/`requirement_ledgers` projections -- no
transcript -- then fold the bounded newest reports, observed evidence, review
findings, memory packets, continuation decisions, delegated-provider observations,
and the goal's workpad/task ref into sourced `ContinuationContextFragment`s. Every
fragment carries a `source_ref` and an FNV-1a `content_hash`; a referenced
report/evidence/memory body is named by artifact id with the artifact's content
hash and redaction state (new `SqliteStateStore::artifact_by_id` query), never
inlined, and a non-`safe` or missing artifact is carried as a redacted reference.
Assembly is bounded by explicit `ContinuationContextLimits` (newest-N selection +
per-fragment summary char cap with an explicit ellipsis), and `render_prompt`
leads with the reconstructed objective + audit contract. The packet is a return
value (loop input), never persisted as authoritative read-model state.

Evidence:

- New module `crates/capo-controller/src/continuation_context.rs` (types
  `ContinuationContextPacket`/`ContinuationAuditContract`/
  `ContinuationContextFragment`/`ContinuationRequirement`/
  `ContinuationSourceKind`/`ContinuationContextLimits`, all re-exported from the
  controller crate root) plus a new read query
  `SqliteStateStore::artifact_by_id` in `crates/capo-state/src/queries.rs` for the
  referenced-body content hash + redaction lookup.
- 4 deterministic controller tests (scripted/seeded goal state, no live provider):
  `continuation_context_packet_selects_bounded_sourced_fragments` (selection +
  source refs + observed-vs-reported tagging + content hashes + prompt),
  `continuation_context_is_bounded_and_does_not_dump_whole_bodies` (selection and
  summary-size limits enforced),
  `continuation_context_preserves_artifact_content_hash_and_redacts_unsafe_bodies`
  (provenance + redaction, including a missing-artifact degrade), and
  `continuation_objective_and_audit_contract_survive_server_restart_and_rebuild`
  (re-open over the same state root + `rebuild_projections` -> the objective +
  audit contract + whole packet rebuild byte-for-byte identically).
- `cargo test -p capo-controller continuation_context` -> 4 passed.
- `cargo fmt --check` -> exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.
- `cargo test --workspace` -> exit 0, 0 failed across all crates (capo-controller
  lib: 122 passed, 2 ignored; capo-state lib: 62 passed).
- `git diff --check` -> clean.

Acceptance:

- Build a sourced continuation context packet from goal state, requirements,
  latest reports, evidence, blockers, validation, review state, memory packets,
  and relevant workpad/source refs, realizing `goal-orchestration` GO7.
- Define the continuation prompt/context shape that survives server restart,
  compaction, adapter restart, and provider transcript loss; the active
  objective and audit contract are reconstructed from persisted goal state, not
  from a model transcript.
- Preserve source refs and content hashes for every injected context fragment so
  the packet is explainable and provenance is queryable, consistent with
  `memory-architecture.md`.
- Keep packet assembly bounded (explicit selection and size limits) and
  explainable; do not dump whole files or raw transcripts.
- Assemble the packet on the controller/server side as input to the real turn
  loop; it does not become authoritative read-model state.

Verification:

- Tests for packet selection, source refs, content hashes, and redaction:
  `cargo test -p capo-controller continuation_context`.
- Recovery test showing the active objective and audit contract survive a server
  restart and rebuild.
- `cargo fmt`.

Must not do:

- Do not require semantic/vector memory for the first continuation packet.
- Do not reconstruct the objective from provider transcript text.

## GA4 - Milestone B: Safe-Boundary Continuation Scheduler

Status: done. The safe-boundary continuation scheduler (GO8) is implemented in
`crates/capo-controller/src/continuation_scheduler.rs` as a PURE state machine:
`ContinuationScheduler::decide(&SchedulerInputs) -> ContinuationOutcome` performs
no I/O, appends no event, and is deterministic given its inputs, producing one of
`continue | pause | block | budget-limit | no-progress-suppress` with a stable
machine reason code. It is opt-in only: `SchedulerInputs::enabled` (an explicit
operator/config flag) is required for `continue`; `enabled = false` short-circuits
to `pause` (`not_enabled`), so automatic continuation is never on by default.
Continue is reachable only at a safe boundary -- goal active, runtime + session
idle, no queued user input, no pending permission, capability profile valid,
budget available, no recent no-progress suppression, AND no conflicting
`safety-gates` workspace lock (consuming the SG5 single-writer write lease via
`workspace_lease_holder`: a held lease owned by a DIFFERENT session is a conflict;
the same session is not). A source-writing next step requires BOTH a checkpoint
boundary and the verification runner present, else it pauses
(`writes_source_without_checkpoint` / `writes_source_without_verification`) -- the
scheduler refuses to continue a goal that would write source without a checkpoint
boundary. No-progress/spin guard: a prior `no-progress-suppress` continuation
(read from the goal's continuation ledger) forces `no-progress-suppress` until
strategy changes. Budget exhaustion is a terminal `budget-limit`; the recording
path `evaluate_and_record_continuation` durably records the decision through the
GA1 `goal.continuation_decision_recorded` event + `GoalContinuationProjection`
(idempotent on `(goal, continuation_id)`) and pairs `budget-limit` with the
existing RTL7 `abort_run_for_ceiling` `run.aborted` abort. GA0 open question
resolved: `GoalBudget` COMPOSES the RTL7 per-run `RunResourceCeiling` (reusing its
ceiling/usage types) rather than replacing it; the run-level floor still fires in
the loop. All scheduler policy lives controller-side; no client surface holds it.

Evidence:

- New module `crates/capo-controller/src/continuation_scheduler.rs` (types
  `ContinuationScheduler`/`ContinuationDecision`/`ContinuationOutcome`/
  `SchedulerInputs`/`ContinuationConditions`/`GoalBudget`, all re-exported from the
  controller crate root) plus controller methods `evaluate_continuation` (pure,
  read-only) and `evaluate_and_record_continuation` (records the decision + aborts
  on budget-limit). No new event/projection types were needed: the GA1
  `ContinuationDecisionRecorded` event + `GoalContinuationProjection` and the RTL7
  `abort_run_for_ceiling` path are reused.
- 15 deterministic tests (mocked/seeded goal state, no live provider): 8 pure
  state-machine branch tests (continue at safe boundary; never continue when not
  enabled; pause on each unsafe-boundary condition; refuse source write without
  checkpoint/verification; block outranks every other signal; budget-limit
  outranks soft pause; no-progress suppression until strategy changes) and 7
  controller-wiring tests (records a continue decision; refuses to continue when
  another writer holds the workspace lock and the same session does not conflict;
  budget-limit aborts the run durably; no-progress suppression blocks the next
  continuation; blocks a blocked goal; recording idempotent on continuation_id;
  continuation decisions survive restart + projection rebuild).
- `cargo test -p capo-controller continuation_scheduler` -> 15 passed.
- `cargo fmt --check` -> exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings` -> exit 0.
- `cargo test --workspace` -> exit 0, 0 failed across all crates (capo-controller
  lib: 139 passed, 2 ignored; capo-state lib: 62 passed; capo-server lib: 108
  passed, 3 ignored).
- `git diff --check` -> clean.

Acceptance:

- Add a server/controller continuation scheduler as a PURE state machine
  (deterministic given inputs) realizing `goal-orchestration` GO8; it produces a
  `continue | pause | block | budget-limit | no-progress-suppress` decision and
  is opt-in only (explicit operator/config enablement).
- The scheduler may continue an active goal only at safe boundaries: runtime and
  session idle, no queued user input, no pending permission, budget available,
  capability profile still valid, no recent no-progress suppression, AND no
  conflicting workspace lock acquired through the `safety-gates` single-writer
  workspace lock/write lease.
- Continuation requires the `safety-gates` checkpoint/rollback and verification
  runner to be present; the scheduler refuses to continue a goal whose next step
  would write source without a checkpoint boundary.
- Add no-progress and spin guards: a continuation that makes no material
  progress suppresses the next automatic continuation until strategy changes.
- Add budget-limited and blocked transitions and a `run.aborted`/continuation
  decision event on budget exhaustion; resolve whether `GoalBudget` extends the
  RTL per-run resource ceiling or composes it (carry the open question from
  GA0).
- Start with deterministic mocked agents; live continuation stays behind an
  explicit opt-in gate, never on by default.

Verification:

- Scheduler state-machine tests for each decision branch:
  `cargo test -p capo-controller continuation_scheduler`.
- A test proving the scheduler refuses to continue when the workspace lock is
  held by another writer and when budget is exhausted.
- A test proving no-progress suppression blocks the next automatic continuation.
- `cargo fmt`.

Must not do:

- Do not continue a goal that would write source without a checkpoint boundary.
- Do not enable automatic continuation by default.
- Do not place scheduler policy in any client surface.

## GA5 - Milestone B: Evidence-Gated Completion Auditor

Status: pending.

Acceptance:

- Build a requirement-by-requirement completion auditor (`goal-orchestration`
  GO9) that decides goal-complete using observed evidence, validation, review,
  blocker, and confidence records; agent completion claims are PROPOSALS only.
- The Capo goal-complete transition is reachable ONLY through the auditor; an
  agent `capo.complete_requirement`/`capo.complete_subtask` report and a
  provider-native completion are recorded as `source=agent_reported`/observed
  evidence and never directly flip goal state.
- Require concrete observed evidence (from `tools-aci` test/check tool output and
  runtime/adapter observations) before marking a requirement or goal complete;
  reject completion backed only by agent prose.
- Distinguish requirement states: supported, validated, reviewed, blocked,
  contradicted, unverified; record skipped or weak validation explicitly.
- Emit an auditor decision event and projection so the "why is this (not)
  complete?" answer is a derived read model, not hand-written prose.

Verification:

- Auditor tests with complete, partial, weak-evidence, contradicted, blocked,
  and overclaimed scenarios: `cargo test -p capo-controller completion_auditor`.
- A test proving an agent-reported completion claim alone does NOT transition the
  goal to complete (premature-completion-blocked).
- A test proving a requirement with concrete observed evidence and validation
  does transition.
- `cargo fmt`.

Must not do:

- Do not let global/aggregate confidence substitute for requirement-level
  evidence.
- Do not allow provider-native completion to authoritatively complete a Capo
  goal.

## GA6 - Milestone B: Reattach-After-Compaction

Status: pending.

Acceptance:

- On server restart and on adapter/provider session restart, re-inject the
  active objective, success criteria, and audit contract from persisted goal
  state into the continuation context, realizing `goal-orchestration` GO13.
- Ensure goals, requirements, evidence, validations, reviews, continuation
  decisions, and historical reports survive server restart and projection
  rebuild.
- Fix or avoid provider artifact overwrite patterns so earlier live replies and
  per-turn evidence remain recoverable for the auditor and historical reports;
  key artifacts so multiple provider turns do not overwrite prior evidence (the
  observed `stdout.txt` reuse pattern must not destroy earlier turn evidence).
- Define a retention policy for raw provider output, redacted summaries, hashes,
  and exported reports, consistent with `state-model.md` artifact rules.
- After restart, the auditor and scheduler operate on the rebuilt goal state
  without depending on any in-memory transcript.

Verification:

- Restart/replay test proving goal + continuation state rebuilds identically:
  `cargo test -p capo-state goal_replay` and
  `cargo test -p capo-controller reattach`.
- Artifact-retention test proving multiple provider turns do not overwrite the
  historical evidence needed for the auditor or report.
- `cargo fmt`.

Must not do:

- Do not depend on live model transcript memory to reconstruct the objective.

## GA7 - Milestone B: Parent/Child Subgoals And Provider-Native Delegation

Status: pending.

Acceptance:

- Model parent/child goals and sessions (`goal-orchestration` GO11): a child
  agent publishes progress, evidence, blockers, and completion claims to its own
  session and to the parent Capo goal; child completion claims do not
  automatically satisfy parent requirements.
- Define merge/review points before child work can satisfy a parent requirement;
  add subgoal result contracts and a parent-visible story projection; keep child
  reports scoped by capability profile, workspace/checkpoint, and evidence refs.
- Feature-probe provider-native goal support rather than assuming it (GO12);
  define a delegated mode for Codex `/goal` where Capo mirrors objective and
  success criteria, dispatches to the provider-native goal mode when available,
  observes events, and audits completion through GA5.
- Add fallback behavior when provider-native goal commands are unavailable, and
  record provider-native goal state, command surface, and limitations with dated
  evidence.
- Keep provider-native completion as evidence only (`source=agent_reported`/
  observed), never as authoritative Capo completion.

Verification:

- Mocked multi-agent test with parent goal, child subgoal, child evidence,
  parent review, and parent story report:
  `cargo test -p capo-controller parent_child`.
- Deterministic fake delegated-provider test proving provider completion is
  recorded as evidence and audited, not auto-completed.
- Optional live Codex `/goal` smoke behind explicit opt-in (mirroring
  `CAPO_SERVER_RUN_CODEX_LIVE`), with secrets stripped and paired with a
  deterministic wire/exit assertion.
- `cargo fmt`.

Must not do:

- Do not make Codex `/goal` the Capo goal model.
- Do not let child completion claims auto-satisfy parent requirements.

## GA8 - Mocked End-To-End Continuation And Completion Paths

Status: pending.

Acceptance:

- Run a full mocked e2e covering each scheduler/auditor branch with mocked
  agents and no live provider:
  - continue at a safe boundary;
  - pause when input is queued or a boundary is unsafe;
  - block on a raised blocker;
  - budget-limit on budget exhaustion;
  - no-progress suppression after a no-material-progress continuation;
  - premature-completion-blocked when only an agent claim exists;
  - complete-with-evidence when concrete observed evidence and validation are
    present.
- Each branch asserts the resulting event sequence and projection state, not
  console text.
- The e2e composes the real turn loop, the GA4 scheduler, and the GA5 auditor
  through the server/controller boundary; the workspace lock and checkpoint
  boundaries from `safety-gates` are exercised in the continue path.
- Generate a historical report at the end of the run and snapshot it.

Verification:

- `cargo test -p capo-controller goal_autonomy_e2e` covering all seven branches.
- Wire/event-sequence snapshot per branch.
- `cargo fmt`.

Must not do:

- Do not assert completion through console output alone; assert events and
  projections.
- Do not invoke a live provider in the deterministic e2e.

## GA9 - Restart/Replay, E2E Gate, And Goal-Orchestration Close-Out

Status: pending.

Acceptance:

- Prove goal + continuation + auditor + report state survives server restart and
  full projection rebuild end to end, including reattach-after-compaction from
  GA6.
- Run focused verification across all changed crates (`capo-state`,
  `capo-server`, `capo-controller`, `capo-cli`) and widen to `cargo test` if
  shared controller/state behavior changes broadly.
- Add review notes covering architecture fit (one orchestration path, no second
  controller), safety/privacy (redaction on the report and continuation packet,
  no agent-asserted completion), test adequacy, provider lock-in, and product
  fit.
- Mark each realized `goal-orchestration` task (GO1/GO3/GO4-GO10/GO13/GO11/GO12)
  as design-realized with a pointer to the implementing `GA<N>` task, closing the
  design-vs-code gap.
- Decide whether Milestone A gated independently before Milestone B or both share
  this close-out gate (resolve the GA0 open question with evidence).

Verification:

- `cargo fmt`.
- Restart/replay test: `cargo test -p capo-state goal_replay` plus the GA8 e2e.
- Focused `cargo test` for changed crates, widening to `cargo test` if needed.
- `git diff --check`.

Must not do:

- Do not close the workpad on self-attestation; close on deterministic e2e plus
  restart/replay evidence.
