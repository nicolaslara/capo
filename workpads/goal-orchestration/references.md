# Goal Orchestration References

## Objective

Record the local and external sources that shape the goal-orchestration workpad.
Dated claims reflect the observed state on 2026-05-28.

## Local Architecture Sources

- `workpads/architecture/boundaries.md`
  - Key facts: Capo is the controller; input surfaces submit commands and render
    read models; the event log is authoritative; raw external streams are
    inputs; UI/voice/mobile must not own orchestration state.
- `workpads/architecture/state-model.md`
  - Key facts: SQLite events are the source of operational truth; read models
    rebuild from events plus artifacts; session/run/turn lifecycle, evidence,
    review, memory, and artifacts already belong in the state model.
- `workpads/architecture/tool-exposure.md`
  - Key facts: Capo tools are controller capabilities; every visible tool call
    should become durable events; `capo.evidence_record` is already identified
    as an early state-mutating Capo tool; agent-native/provider-native tools are
    observed-only unless Capo executes or receives structured lifecycle data.
- `workpads/architecture/memory-architecture.md`
  - Key facts: memory is derived context with provenance and confidence;
    operational truth remains in SQLite events, projections, and artifacts;
    memory packets are fractional and sourced.
- `workpads/architecture/prototype-plan.md`
  - Key facts: the prototype spine already expects controller-owned state,
    recent events, current goal, blockers, confidence, tool observations,
    evidence refs, memory packets, and evidence export.

## Local Product And Implementation Sources

- `workpads/harness-research/knowledge.md`
  - Key facts: ACP is an adapter/protocol boundary, not a controller; Capo should
    own runtime, permissions, tool instrumentation, checkpoints, memory,
    evaluation, observability, multi-client state, and the outer goal loop.
- `workpads/harness-research/references.md`
  - Key facts: dated source links for Codex Goals, Codex safety, ACP, OpenCode,
    Claude Code, Cursor, OpenHands, SWE-agent, Aider, Cline, Gemini CLI, Goose,
    and Roo Code.
- `workpads/operator-control/knowledge.md`
  - Key facts: operator-control is an input/client surface; current control UI
    can inspect agents, show evidence/reviews, start Codex with opt-in, and has
    an identified live-provider artifact retention gap for historical replies.
- `workpads/server/knowledge.md`
  - Key facts: server/control-plane work completed enough for CLI-through-server
    command paths, mocked agents, Codex proof, and manual smoke evidence.

## External Sources Already Reflected In Harness Research

- https://developers.openai.com/codex/use-cases/follow-goals
  - Observed 2026-05-28 in harness research.
  - Key facts: `/goal` gives Codex a durable objective; command lifecycle
    includes set/view/pause/resume/clear; good goals need a stopping condition
    and validation loop.
- https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex
  - Observed 2026-05-28 in harness research.
  - Key facts: Codex Goals are persisted thread state with lifecycle, budget,
    progress accounting, event-driven continuation at safe boundaries, and
    evidence-based completion audit.
- https://github.com/openai/codex/issues/19910
  - Observed 2026-05-28 in harness research.
  - Confidence: public issue report, not stable documentation.
  - Key facts: reports compaction-related goal-continuation/audit loss and
    argues for reinjecting goal context from persisted active goal state.

## Follow-Up Sources To Verify When Implementing Parent/Child Reporting

- Claude Code Agent Teams and Subagents docs for task lists, communication, and
  result handoff patterns.
- OpenCode agents/subagents docs and source for parent/child session structure.
- Goose subagent docs for visible subagent tool-call traces.
- Roo Code Boomerang Tasks docs for subtask completion handoff.

These follow-up sources should be rechecked before implementation because
agent-product surfaces change quickly.
