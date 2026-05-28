# Goal Orchestration Tasks

## Objective

Build Capo's controller-owned goal loop: durable goals, structured agent
reporting, evidence/review/validation ledgers, event-driven continuation,
completion audit, parent/child agent reporting, provider-native goal delegation,
and historical execution reports.

The workpad exists because this is larger than `operator-control`. The control
REPL is a client/input surface. Goal orchestration changes the server,
controller, state model, tool/reporting surface, read models, validation layer,
and provider-adapter policy.

## Status

Planned after `operator-control` closes. `GO0` created this workpad and routing.
All implementation tasks remain pending.

## Feature Set

- Capo-owned `Goal` lifecycle and requirement model.
- Agent-native publish/report tools for intent, progress, evidence, confidence,
  assumptions, blockers, reviews, validation, and completion claims.
- Event-log and read-model projections that let Capo tell the execution story.
- Evidence, review, validation, and confidence ledgers tied to requirements.
- Server-side continuation scheduler with safe-boundary and no-progress guards.
- Context packet and continuation prompt assembly from sourced state.
- Historical execution reports in human-readable and machine-readable forms.
- Parent/child agent reporting contracts for subgoals and delegated work.
- Optional provider-native goal delegation, starting with Codex `/goal`, without
  making provider state authoritative.

## GO0 - Workpad And Routing

Status: completed on 2026-05-28.

Acceptance:

- Decide whether the design belongs in an existing workpad or a new one.
- Create a full workpad covering all goal-orchestration feature slices, not only
  agent reporting.
- Add the workpad to the project queue after `operator-control`.
- Add the workpad load list and rules to `workpads/WORKPADS.md`.
- Record the scope decision and implementation principles.

Evidence:

- `TASKS.md`
- `workpads/WORKPADS.md`
- `workpads/goal-orchestration/tasks.md`
- `workpads/goal-orchestration/knowledge.md`
- `workpads/goal-orchestration/references.md`

## GO1 - Domain Model And Architecture Delta

Status: pending.

Acceptance:

- Define `Goal`, `GoalRequirement`, `GoalAttempt`, `GoalContinuation`,
  `GoalBudget`, `AgentReport`, `EvidenceRecord`, `ReviewRecord`,
  `ValidationRecord`, `ConfidenceRecord`, and `HistoricalExecutionReport`.
- Define lifecycle states for goals, requirements, reports, reviews,
  validations, continuations, and delegated provider subgoals.
- Define event names, idempotency keys, external refs, artifact refs, and
  projection ownership.
- Update the relevant architecture docs or add a focused design doc under this
  workpad if the delta is too detailed for existing architecture files.
- Include failure modes: stale reports, fabricated evidence, duplicate reports,
  conflicting confidence, provider transcript loss, compaction loss, restart,
  and partial delegation.

Verification:

- Architecture doc/delta reviewed against `boundaries.md`, `state-model.md`,
  `tool-exposure.md`, `memory-architecture.md`, and harness research.
- `git diff --check`.

## GO2 - Agent Reporting Tool Contract

Status: pending.

Acceptance:

- Define the first reporting tool surface:
  - `capo.report_intent`
  - `capo.report_progress`
  - `capo.record_evidence`
  - `capo.report_confidence`
  - `capo.record_assumption`
  - `capo.raise_blocker`
  - `capo.request_review`
  - `capo.record_review`
  - `capo.record_validation`
  - `capo.complete_requirement`
  - `capo.complete_subtask`
- Specify schemas, required scopes, risk levels, redaction policy, and whether
  each tool mutates Capo state.
- Define which tools are agent-visible, parent-agent-visible, input-surface
  visible, or internal-only.
- Decide how reports relate to observed tool/runtime/provider events: reports
  may explain intent and confidence, but they do not replace observed evidence.
- Add a fake tool implementation plan for deterministic tests.

Verification:

- Tool contract aligns with `tool-exposure.md`.
- Secret/redaction policy reviewed.
- `git diff --check`.

## GO3 - Event Store And Read-Model Plumbing

Status: pending.

Acceptance:

- Add append-only events for goal lifecycle, report publication, evidence,
  confidence, assumptions, blockers, reviews, validations, requirement status,
  continuation decisions, and delegated-provider goal state.
- Add projections for:
  - active goals;
  - requirement status;
  - agent report timeline;
  - evidence ledger;
  - review ledger;
  - validation ledger;
  - confidence/risk summary;
  - current blocker state;
  - historical execution story.
- Preserve raw adapter/provider data as inputs/artifacts, not authoritative
  read-model truth.
- Include idempotency tests for duplicate report submissions and replay.

Verification:

- Focused Rust tests for event/projection behavior.
- `cargo fmt`
- Focused `cargo test` commands for state/server crates.

## GO4 - Server Commands For Goals And Reports

Status: pending.

Acceptance:

- Add typed server requests for creating/viewing/pausing/resuming/clearing/
  canceling/blocking/completing goals.
- Add typed server requests for recording each report/evidence/review/
  validation event.
- Ensure all mutations flow through the server/controller boundary.
- Add query commands for goal status, agent reports, story, evidence, review,
  validation, and historical report projections.
- Preserve deterministic mocked-agent tests before integrating live providers.

Verification:

- Server request/response tests with fake agents.
- CLI-through-server or direct server-client tests for each mutation/query path.

## GO5 - Operator Control Read Surfaces

Status: pending.

Acceptance:

- Add human-facing control commands for goal/story/report inspection, such as:
  - `goals`
  - `goal [GOAL]`
  - `story [AGENT|GOAL]`
  - `timeline [AGENT|GOAL]`
  - `evidence [AGENT|GOAL]`
  - `validations [AGENT|GOAL]`
  - `reviews [AGENT|GOAL]`
  - `risks [AGENT|GOAL]`
- Keep normal output concise and readable; keep raw event/projection metadata
  behind `details`/debug-style commands.
- Make "what happened?", "is this validated?", and "has this been reviewed?"
  answerable from read models.
- Preserve scripted stdin tests and interactive behavior from operator-control.

Verification:

- Deterministic control tests with mocked data.
- Manual transcript showing an execution story rather than debug output.

## GO6 - Goal Lifecycle Commands

Status: pending.

Acceptance:

- Add user-facing goal commands to create and manage Capo-owned goals:
  - start/set goal;
  - inspect goal;
  - pause/resume;
  - clear/cancel;
  - mark blocked with reason;
  - mark complete only through the completion/audit path.
- Link goals to project, task, agent, session, parent goal, and requirements.
- Define how goals differ from one-off `send`/`steer` messages.
- Store success criteria, constraints, verification surface, budget, and stop
  conditions as structured state.

Verification:

- Unit tests for lifecycle transitions.
- Server/control tests that reject illegal transitions.

## GO7 - Context Packet And Continuation Prompt Assembly

Status: pending.

Acceptance:

- Build sourced continuation context from goal state, requirements, latest
  reports, evidence, blockers, validation, review state, memory packets, and
  relevant workpad/source refs.
- Define prompt/context shape for Capo-owned continuation that survives restart,
  compaction, adapter restart, and provider transcript loss.
- Preserve source refs and content hashes for injected context.
- Keep prompt assembly bounded and explainable.

Verification:

- Tests for packet selection, source refs, and redaction.
- Recovery test showing the active objective and audit contract survive restart.

## GO8 - Event-Driven Continuation Scheduler

Status: pending.

Acceptance:

- Add a server/controller scheduler that can continue active goals only at safe
  boundaries:
  - runtime/session idle;
  - no queued user input;
  - no pending permission;
  - no conflicting workspace lock;
  - budget available;
  - no recent no-progress suppression;
  - capability profile still valid.
- Add no-progress and spin guards.
- Add budget-limited and blocked transitions.
- Start with deterministic mocked agents and explicit opt-in for automatic
  continuation.

Verification:

- Scheduler state-machine tests.
- Mocked e2e proving continue, pause, blocked, budget-limited, no-progress
  suppression, and completion paths.

## GO9 - Evidence-Backed Completion Auditor

Status: pending.

Acceptance:

- Build requirement-by-requirement completion checks using evidence, validation,
  review, blocker, and confidence records.
- Require concrete evidence before marking a requirement or goal complete.
- Allow agents to propose completion, but keep the final Capo goal-complete
  transition guarded by the auditor.
- Distinguish supported, validated, reviewed, blocked, contradicted, and
  unverified requirements.
- Record skipped or weak validation explicitly.

Verification:

- Auditor tests with complete, partial, weak-evidence, contradicted, blocked,
  and overclaimed scenarios.
- Manual report showing why a goal is or is not complete.

## GO10 - Historical Execution Reports

Status: pending.

Acceptance:

- Generate reports that tell the story of a goal or agent run:
  - objective and success criteria;
  - agent/session/run timeline;
  - intent changes and rationale;
  - actions and tool/runtime/provider observations;
  - evidence and artifacts;
  - assumptions and confidence;
  - blockers and decisions;
  - review and validation status;
  - final outcome and remaining risk.
- Support markdown for humans and JSON for machine consumers.
- Ensure reports are rebuildable from events, projections, and artifacts.
- Include redaction and missing-artifact behavior.

Verification:

- Snapshot/golden tests for report rendering.
- Manual generated report from a mocked goal run.

## GO11 - Parent/Child Agent Reporting And Subgoals

Status: pending.

Acceptance:

- Model parent/child goals and sessions.
- Define how a child agent publishes progress, evidence, blockers, and
  completion claims to the parent Capo goal.
- Define merge/review points before child work can satisfy parent requirements.
- Add subgoal result contracts and parent-visible story projection.
- Keep child reports scoped by capability profile, workspace/checkpoint, and
  evidence refs.

Verification:

- Mocked multi-agent test with parent goal, child subgoal, child evidence,
  parent review, and parent story report.

## GO12 - Provider-Native Goal Delegation

Status: pending.

Acceptance:

- Feature-probe provider-native goal support instead of assuming it exists.
- Define delegated mode for Codex `/goal`: Capo mirrors the objective and
  success criteria, dispatches to the provider-native goal mode when available,
  observes events, and audits completion externally.
- Add fallback behavior when provider-native goal commands are unavailable.
- Keep provider-native completion as evidence, not as authoritative Capo
  completion.
- Record provider-native goal state, command surface, and limitations with
  dated evidence.

Verification:

- Deterministic fake delegated-provider tests.
- Optional live Codex smoke behind explicit opt-in, separate from ordinary test
  runs.

## GO13 - Recovery, Artifact Retention, And Replay

Status: pending.

Acceptance:

- Ensure goals, reports, evidence, validations, reviews, continuation decisions,
  and historical reports survive server restart and projection rebuild.
- Fix or avoid provider artifact overwrite patterns that make earlier live
  replies unrecoverable.
- Add per-turn artifact refs or bounded redacted display snapshots where needed
  for historical reporting.
- Define retention policy for raw provider output, redacted summaries, hashes,
  and exported reports.

Verification:

- Restart/replay tests.
- Artifact retention test proving multiple provider turns do not overwrite the
  historical evidence needed for reports.

## GO14 - E2E Gate And Review

Status: pending.

Acceptance:

- Run a full mocked e2e path:
  - create goal;
  - agent reports intent/progress/evidence/confidence;
  - validation and review are recorded;
  - scheduler continues once;
  - auditor blocks premature completion;
  - requirement is later completed with evidence;
  - historical report is generated.
- Run focused verification commands for changed crates.
- Add review notes covering architecture fit, safety/privacy, test adequacy,
  provider lock-in, and product fit.
- Decide whether the next work should deepen goal orchestration or move to
  checkpoint/rollback/autonomy hardening.

Verification:

- `cargo fmt`
- Focused `cargo test` commands, widening to `cargo test` if shared controller
  or state behavior changes broadly.
- `git diff --check`
