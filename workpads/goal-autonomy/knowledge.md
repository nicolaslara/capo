# Goal Autonomy Knowledge

## Objective

Capture the implementation decisions for Capo's autonomy differentiator: the
durable goal/requirement/evidence model and projections, the safe-boundary
continuation scheduler, the evidence-gated completion auditor, and
reattach-after-compaction. This workpad turns the `goal-orchestration` design
into running code on the now-real loop/tools/safety substrate. It does not
re-author that design.

## Scope Decision

`goal-orchestration` REMAINS the authoritative DESIGN SOURCE. It owns the goal
domain model, the GO2 agent-reporting semantics, the evidence/review/validation
ledger semantics, the story projections, the completion-audit philosophy
(requirement-level evidence over global confidence), the parent/child contracts,
and the provider-native delegation policy. Its `knowledge.md` and `tasks.md` are
cited, not duplicated, by this workpad. Where a design question is already
answered there (confidence levels, evidence statuses, reporting cadence, stop
policy), this workpad references it rather than restating it.

`goal-autonomy` IMPLEMENTS the runtime on the now-real substrate and does NOT
re-specify the design. The prerequisite workpads made the substrate real: a
genuine observe->decide->emit turn loop driving the dispatch primitives
(`real-turn-loop`), a real ACI with the GO2 reporting/evidence tool surface and
observed-vs-reported tagging (`tools-aci`), and checkpoint/rollback, a real
verification runner, permission enforcement, and a single-writer workspace lock
(`safety-gates`). Before these landed, GO1-GO13 implicitly assumed substrate
that did not exist, which is why autonomy could not land first.

The GO2 reporting TOOL surface is NOT implemented here. It is an ACI/tool-registry
concern implemented in `tools-aci`: each reporting tool is registered in the
typed registry, and agent reports are persisted as a distinct
`source=agent_reported` event class (with confidence), separate from observed
evidence (`source=runtime_output`/`adapter_event`). `goal-autonomy` consumes
those tagged events; it does not redefine evidence or re-register the tools.

This workpad carries TWO milestones, split by dependency, not by workpad
(consistent with the daily-driver roadmap's Phase 3 "make it autonomous"):

- Milestone A: the goal/requirement/evidence event model + projections (GO1/GO3),
  lifecycle/server/read commands (GO4-GO6, GO10). Depends only on
  `real-turn-loop` + `tools-aci`. The goal model and read surfaces do not need
  checkpoint/rollback, so they land earlier.
- Milestone B: the continuation context packet (GO7), the safe-boundary
  continuation scheduler (GO8), the evidence-gated completion auditor (GO9),
  reattach-after-compaction (GO13), and parent/child plus provider-native
  delegation as observed-not-authoritative (GO11/GO12). HARD-GATED on
  `safety-gates` checkpoint/rollback + verification + the single-writer workspace
  lock. Autonomy is unsafe before this substrate exists.

After this workpad closes, `goal-orchestration`'s remaining design-only tasks are
marked satisfied-by/folded as references to the realizing `GA<N>` tasks, and
`goal-orchestration` is closed as "design realized in `goal-autonomy` +
`tools-aci`." Net: one design brain (`goal-orchestration`), one implementation
(`goal-autonomy` + the GO2 surface in `tools-aci`), and no competing goal
designs.

## Event-Driven Safe-Boundary Scheduler

The continuation scheduler (GA4, realizing GO8) is a PURE state machine:
deterministic given its inputs, producing a `continue | pause | block |
budget-limit | no-progress-suppress` decision and no side effects of its own. It
is opt-in only; automatic continuation is never on by default. It may continue an
active goal only at a safe boundary, reusing the stop policy defined in
`goal-orchestration/knowledge.md`: runtime and session idle, no queued user
input, no pending permission, budget available, capability profile still valid,
no recent no-progress suppression, AND no conflicting workspace lock acquired
through the `safety-gates` single-writer write lease.

Continuation requires the `safety-gates` substrate to be present. The scheduler
refuses to continue a goal whose next step would write source without a
checkpoint boundary, and it relies on the verification runner's OBSERVED evidence
rather than agent prose. This keeps the first unattended writes confined,
reversible, and bounded, which the design's Non-Goals explicitly require before
broad unattended source-writing.

## Completion Auditor As The Only Path To Goal-Complete

The evidence-gated completion auditor (GA5, realizing GO9) is the ONLY path to a
Capo goal-complete transition. Agents PROPOSE completion; they never ASSERT it. A
`capo.complete_requirement`/`capo.complete_subtask` report and any provider-native
completion are recorded as `source=agent_reported`/observed evidence and never
directly flip goal state. Goal-complete is therefore not an ordinary lifecycle
command; a direct "mark complete" server request is rejected by construction.

The auditor decides requirement-by-requirement on observed evidence, validation,
review, blocker, and confidence records, distinguishing supported, validated,
reviewed, blocked, contradicted, and unverified requirements and recording
skipped or weak validation explicitly. Per the design, requirement-level evidence
is required; global/aggregate confidence cannot substitute for it. The auditor
emits a decision event and projection so "why is this (not) complete?" is a
derived read model rather than hand-written prose.

## No-Progress And Spin Suppression

A continuation that makes no material progress suppresses the next automatic
continuation until strategy changes (operator or planner intervention),
implementing the design's no-progress guard. This prevents the scheduler from
burning budget on a spinning loop. Budget exhaustion produces a `budget-limit`
decision and a continuation/`run.aborted` event rather than silent termination.
Whether the `GoalBudget` strictly extends the `real-turn-loop` per-run resource
ceiling or composes it is carried as an open question into GA0/GA4.

## Reattach Objective After Compaction

On server restart and on adapter/provider session restart, the active objective,
success criteria, and audit contract are re-injected into the continuation
context from PERSISTED goal state, not from a model transcript (GA6, realizing
GO13). Goals, requirements, evidence, validations, reviews, continuation
decisions, and historical reports survive restart and projection rebuild. This
directly addresses the observed compaction-related goal/continuation loss in the
codex `/goal` issue: the objective lives in the event log and projections, so the
auditor and scheduler operate on rebuilt state without any in-memory transcript.
Per-turn artifacts must be keyed so multiple provider turns do not overwrite
earlier evidence (the observed `stdout.txt` reuse pattern must not destroy prior
turn evidence the auditor and historical report depend on).

GA6 implementation decisions (realized, deterministic, no live provider):

- The objective + success criteria + audit contract re-inject through the GA3
  continuation context packet, which reconstructs them STRICTLY from the
  `goals` / `requirement_ledgers` projections; nothing GA6-specific was added
  for that path. The auditor (`audit_goal_completion`) and scheduler
  (`evaluate_continuation`) read the same persisted projections, so both operate
  on rebuilt state with no in-memory transcript. Cross-attempt observed evidence
  is read by the goal's stable TASK id, so a fresh attempt session (an
  adapter/provider session restart) does not drop prior-attempt evidence.
- The artifact-overwrite gap was concrete: the adapter-replay
  `adapter.turn_completed` evidence row keyed its `evidence_id` only by
  `(adapter_kind, session_id)`, so successive provider turns in one session
  collapsed onto a single row and the next turn's `ON CONFLICT(evidence_id) DO
  UPDATE` destroyed the prior turn's observed evidence. The fix keys that row PER
  TURN (the explicit turn id when the loop drives a turn, else the event's
  timeline/item key), mirroring how the adapter-replay events themselves are
  already disambiguated. Re-replaying the SAME turn stays idempotent on one row.

Retention policy for raw provider output (consistent with
`workpads/architecture/state-model.md` artifact rules): raw provider/runtime
output is NOT inlined into event payloads or projected read models -- it is stored
as an `Artifact` (`raw_adapter_event` / `runtime_log` / `tool_output` / `evidence`
kind) and referenced by `artifact_id` + `content_hash` + `redaction_state`. The
continuation packet and historical report carry only bounded summaries plus those
references; a non-`safe` artifact is carried as a redacted reference (named, never
inlined), and a missing artifact degrades to a redacted reference rather than
inventing content. Because per-turn evidence rows and their backing artifacts are
keyed per turn, retaining one turn's raw output never overwrites another's, so the
auditor and exported reports keep every turn's evidence recoverable after restart
+ rebuild.

## Non-Goals

- Do not duplicate the `goal-orchestration` design prose (domain model, reporting
  semantics, ledger semantics, completion-audit philosophy).
- Do not re-implement the GO2 reporting/evidence TOOL surface; it lives in
  `tools-aci`.
- Do not store agent-reported claims indistinguishably from observed evidence.
- Do not let global/aggregate confidence substitute for requirement-level
  evidence at completion.
- Do not allow agent prose or provider-native completion to authoritatively
  complete a Capo goal.
- Do not enable automatic continuation by default, and do not place scheduler or
  auditor policy in any client surface.
- Do not continue a goal that would write source without a checkpoint boundary.
- Do not require a semantic/vector memory system for the first continuation
  packet (depth concern).
- Do not fork a second run/turn completion notion separate from the existing
  dispatch execution-status projections.
- No web client.

## Open Questions

- Is the continuation budget a strict extension of the `real-turn-loop` per-run
  resource ceiling, or a separate `GoalBudget` that composes it?
- Does Milestone A ship and gate independently before Milestone B starts, or do
  both share one close-out gate at workpad close?
- How should stale evidence be detected after files change (inherited open
  question from `goal-orchestration`; relevant to auditor freshness)?
- How much raw provider text should be retained per live turn for the auditor and
  historical reports, given the artifact-retention and redaction rules?
