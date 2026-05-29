# Goal Autonomy References

## Objective

Record the local and external sources that shape the goal-autonomy workpad.
Dated claims reflect 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/state-model.md`
  - Key facts: SQLite events are the source of operational truth; read models
    rebuild from events plus artifacts; session/run/turn lifecycle, evidence,
    review, memory, and artifacts already belong in the state model; raw
    adapter/provider data is preserved as inputs/artifacts, not authoritative
    read-model truth. Goal, requirement, continuation, and auditor events must
    extend this append-only model and rebuild deterministically.
- `workpads/harness-research/daily-driver-review.md`
  - Key facts: the goal-lifecycle/continuation dimension scores 1.0 - the
    best-articulated design among peers with essentially zero implementation
    (zero `Goal`/`continuation`/`scheduler`/`auditor` symbols verified in code).
    The review's Phase 3 ("make it autonomous") names the exact pieces this
    workpad builds: goal model + projections with idempotency/rebuild test, a
    pure opt-in safe-boundary scheduler, an evidence-gated auditor as the ONLY
    path to goal-complete (agents propose, never assert), and
    reattach-after-compaction. It places these AFTER the real loop, tools, and
    safety substrate land - the dependency structure this workpad follows.

## Local Product And Implementation Sources

- `workpads/goal-orchestration/knowledge.md`
  - The authoritative DESIGN SOURCE, cited not duplicated.
  - Key facts: Capo owns the outer loop (continuation scheduler -> sourced
    context packet -> adapter -> normalized events/reports ->
    evidence/review/validation ledgers -> completion auditor -> decision);
    confidence levels (`high`/`medium`/`low`) and evidence statuses
    (`observed`/`reported`/`validated`/`reviewed`/`contradicted`/`stale`/
    `redacted`); completion requires requirement-level evidence, not global
    confidence; the stop policy enumerates the safe-boundary preconditions GA4
    consumes; child completion claims do not auto-satisfy parent requirements;
    provider-native loops run inside the outer loop but are never authoritative.
- `workpads/goal-orchestration/tasks.md`
  - The design task source this workpad realizes one-to-one.
  - Key facts: GO1/GO3 (domain model + event/projection plumbing) -> GA1;
    GO4/GO6/GO5/GO10 (server commands, lifecycle, operator read surfaces,
    historical reports) -> GA2; GO7 (context packet) -> GA3; GO8 (scheduler) ->
    GA4; GO9 (auditor) -> GA5; GO13 (recovery/retention/replay) -> GA6;
    GO11/GO12 (parent/child + provider-native delegation) -> GA7. GO2 (reporting
    tool contract) is realized in `tools-aci`, not here.
- `crates/capo-state` (`src/event.rs`, `src/projections.rs`)
  - Key facts: a single append-only `EventKind` enum with idempotency-key dedupe
    and ~30 in-transaction projections rebuilt from `ProjectionRecord` variants;
    `EvidenceRecorded` and `ReviewFindingRecorded` event kinds and `Evidence`/
    `ReviewFinding` projections already exist and are reused rather than
    redefined; only `CapabilityGrantCreated`/`CapabilityGrantUsed` grant kinds
    exist today. New goal/requirement/continuation/auditor event kinds and
    projections extend this model and must rebuild identically on replay.
- `crates/capo-controller`
  - Key facts: the controller is `FakeBoundaryController` over an `AgentAdapter`
    enum (Fake/ScriptedMock) today; `real-turn-loop` replaces it with a real
    observe->decide->emit loop that drives the existing
    `AdapterDispatchExecution` semantics. The GA scheduler, auditor, context
    packet, and reattach are controller/server behavior; a goal attempt
    references dispatch run identity rather than forking a second
    run-completion notion. Scheduler/auditor policy never moves into a client.

## External Sources

- https://developers.openai.com/codex/use-cases/follow-goals
  - Observed 2026-05-29.
  - Key facts: `/goal` gives Codex a durable objective with a set/view/pause/
    resume/clear lifecycle; good goals need an explicit stopping condition and a
    validation loop. Informs the GA2 lifecycle command surface and GA7 delegated
    mode, where Capo mirrors the objective but audits completion itself.
- https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex
  - Observed 2026-05-29.
  - Key facts: Codex Goals are persisted thread state with lifecycle, budget,
    progress accounting, event-driven continuation at safe boundaries, and
    evidence-based completion audit - the same shape Capo implements, but with
    completion gated by Capo's own evidence ledger rather than provider state.
- https://github.com/openai/codex/issues/19910
  - Observed 2026-05-29. Public issue report, not stable documentation.
  - Key facts: reports compaction-related goal-continuation/audit loss and argues
    for reinjecting goal context from persisted active goal state - the direct
    motivation for GA6 reattach-after-compaction from durable state.

Note: these external sources are inherited via
`workpads/harness-research/references.md` and
`workpads/goal-orchestration/references.md`, where they were first observed
(2026-05-28). They are re-dated here as confirmed for this workpad on 2026-05-29
and are not independently re-derived.
