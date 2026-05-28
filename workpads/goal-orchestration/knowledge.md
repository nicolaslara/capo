# Goal Orchestration Knowledge

## Objective

Capture decisions for Capo-owned goal orchestration: durable objectives,
agent-native reporting, evidence-backed validation, continuation scheduling, and
historical execution reports.

## Scope Decision

Create a new `goal-orchestration` workpad.

This should not live inside `operator-control`. Operator-control is a client
surface: it renders state, accepts user commands, and lowers those commands into
server requests. Goal orchestration is controller/server behavior: state,
events, reports, scheduling, continuation, validation, provider delegation, and
history. Mixing them would move orchestration pressure into the client and
contradict Capo's boundary model.

This should not reopen `harness-research`. The prior research is complete and
now serves as input. The next useful step is implementation-shaped product
planning, not more comparison.

## Core Design

Capo should own the outer loop:

```text
CapoGoal
  -> continuation scheduler
  -> sourced context packet
  -> agent adapter/runtime
  -> normalized events and reports
  -> evidence/review/validation ledgers
  -> completion auditor
  -> continue | pause | block | budget-limit | complete
```

Provider-native loops such as Codex `/goal` may run inside this outer loop, but
they do not become authoritative. Capo mirrors the objective, observes provider
activity, records evidence, and decides completion through its own auditor.

## Agent Reporting Principle

Agents should publish structured operational data to Capo while they work. The
model transcript is useful context, but it is not enough for orchestration,
history, or audit.

The first reporting surface should capture:

- Intent: what the agent is trying to do now, why, and what success looks like.
- Progress: what changed since the last report.
- Evidence: files, commands, tests, logs, screenshots, citations, commits,
  artifacts, or other refs.
- Confidence: scoped confidence tied to a claim or requirement.
- Assumptions: facts the agent is relying on that are not fully proven.
- Blockers: what stopped progress, what was tried, and what decision/capability
  is needed.
- Review: reviewer, criteria, findings, accepted/rejected status, and follow-up.
- Validation: check command, result, coverage, skipped reason, or weakness.
- Completion: requirement-level completion claim with evidence refs.

Reports are claims, not proof by themselves. Capo should correlate reports with
observed tool calls, runtime events, artifacts, validation outputs, and reviews.

## Evidence And Confidence Semantics

Confidence must be attached to a scoped claim:

- `high`: direct evidence covers the claim and no known contradictions remain.
- `medium`: evidence supports the claim but coverage is partial, indirect, or
  environment-dependent.
- `low`: evidence is weak, missing, stale, contradicted, or mostly inferred.

Completion should not use global confidence alone. It should require
requirement-level evidence and validation. A goal can have high progress and
still remain incomplete if a requirement is unverified.

Evidence status should distinguish:

- `observed`: Capo saw a tool/runtime/provider event or artifact.
- `reported`: an agent claimed something happened.
- `validated`: a check or reviewer verified the claim.
- `reviewed`: a human or review agent assessed the claim.
- `contradicted`: later evidence conflicts with the claim.
- `stale`: source changed after the evidence was recorded.
- `redacted`: evidence exists but cannot be shown directly.

## Story Projections

Capo should be able to answer:

- What was the goal?
- What was each agent trying to do?
- Why did it choose that path?
- What changed?
- What evidence supports those changes?
- What was validated?
- What was reviewed?
- What is still uncertain?
- Why did Capo continue, pause, block, or complete?

The story should be a derived read model, not hand-written prose. Human-readable
summaries and historical reports are renderings over events, projections, and
artifacts.

First projections:

- `GoalReadModel`: lifecycle, owner, requirements, budget, current state.
- `RequirementLedger`: per-requirement status, evidence, validation, review,
  blocker, confidence, and stale flags.
- `AgentStory`: intent/progress reports plus observed actions and artifacts.
- `EvidenceLedger`: evidence refs, source kind, redaction, support strength.
- `ReviewLedger`: review requests, findings, decisions, unresolved items.
- `ValidationLedger`: checks, outputs, coverage, skipped reasons, failures.
- `HistoricalExecutionReport`: rebuildable report over goal/run/session state.

## Reporting Cadence

Agents should report at meaningful boundaries:

- at the start of a goal or subgoal;
- before a risky or broad change;
- after material progress;
- before and after validation;
- when assumptions or blockers appear;
- before requesting review;
- when proposing requirement or goal completion.

The scheduler should not require reports after every token or minor observation.
Reports should be useful control-plane facts, not noisy telemetry.

## Parent And Child Agents

Parent/child agent work needs explicit contracts:

- A parent goal may create child subgoals.
- Child agents publish reports to their own session and to the parent goal.
- Child completion claims do not automatically satisfy parent requirements.
- Parent requirements become satisfied only after merge/review/evidence rules
  are met.
- Child story and evidence remain inspectable separately, but parent reports
  should summarize the child contribution and remaining risk.

## Stop And Safety Policy

Automatic continuation should wait until the state model is strong enough:

- active Capo goal;
- clear success criteria and stop conditions;
- no queued user input;
- no pending permission;
- runtime/session idle;
- capability profile still valid;
- budget available;
- no recent no-progress suppression;
- continuation context assembled from sourced state;
- evidence ledger available for completion audit.

If a continuation makes no material progress, Capo should suppress the next
automatic continuation until the user or planner changes strategy.

## Historical Reports

Historical execution reports should support:

- operator handoff;
- review and audit;
- debugging provider behavior;
- project memory extraction;
- regression analysis of agent quality;
- proof that a goal was validated or why it remains blocked.

Reports should be exportable as markdown and JSON. They should degrade clearly
when raw artifacts are missing or redacted, instead of implying stronger
evidence than Capo has.

## Non-Goals

- Do not make Codex `/goal` the Capo goal model.
- Do not treat agent prose as authoritative state.
- Do not implement broad unattended source-writing behavior before checkpoint
  and rollback semantics are strong enough.
- Do not require a semantic/vector memory system for the first goal loop.
- Do not expose raw provider transcripts by default just to make reports richer.
- Do not put scheduler policy in `capo control` or any other client.

## Open Questions

- Which reports should be agent-visible tools versus internal Capo commands?
- Should confidence be user-visible by default, or mostly a detail/report field?
- How should Capo detect stale evidence after files change?
- What is the minimum checkpoint/rollback feature needed before automatic
  continuation can write source unattended?
- Should historical reports become memory records automatically, or only after
  review/promotion?
- How much raw provider text should be retained for live Codex/Claude turns?
