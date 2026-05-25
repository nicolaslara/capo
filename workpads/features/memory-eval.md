# Memory And Evaluation Feature

## Objective

Evolve prototype memory packets and local evidence into source-linked memory records and performance/review reports that can guide future agent work.

## Prototype Inputs

- P9 built source-linked packets with inclusion/exclusion reasons.
- P11/P12 export packet and tool/evidence refs into markdown.
- `capo-eval` remains a local scaffold.

## Dependencies

- Memory records require source refs, review state, sensitivity, and provenance.
- Evaluation reports must be derived from events/evidence, not free-floating summaries.

## Tasks

### ME1 - Memory Record Read Models

Status: in_progress

Acceptance:

- Promote memory candidates/records into typed read models beyond packet artifacts.
- Track source hash, source anchor, review state, sensitivity, and invalidation.
- Keep packet building replayable from selected records.

### ME2 - Task Outcome Report

Status: pending

Acceptance:

- Generate a report for completed/interrupted tasks with duration, actions, tool calls, evidence, confidence, blockers, and review outcome.
- Export the report as markdown evidence.
- Record report refs in state.

### ME3 - Review Feedback Loop

Status: pending

Acceptance:

- Capture human/subagent review findings as durable evidence.
- Link findings to sessions, tasks, tools, and follow-up workpad items.
