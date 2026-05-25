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

Status: completed

Acceptance:

- Promote memory candidates/records into typed read models beyond packet artifacts.
- Track source hash, source anchor, review state, sensitivity, and invalidation.
- Keep packet building replayable from selected records.

Evidence:

- `crates/capo-state/src/lib.rs`
- `cargo test -p capo-state memory_record`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused memory read-model review: replayable-source filtering and fail-closed projection decode blockers found and fixed.

### ME2 - Task Outcome Report

Status: completed

Acceptance:

- Generate a report for completed/interrupted tasks with duration, actions, tool calls, evidence, confidence, blockers, and review outcome.
- Export the report as markdown evidence.
- Record report refs in state.

Evidence:

- `crates/capo-eval/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-eval`
- `cargo test -p capo-state task_outcome`
- `cargo test -p capo-cli cli_drives_fake_controller_and_exports_evidence`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused ME2 reviews: self-referential reruns, overwrite safety, review-outcome derivation, terminal-status guard, and report/artifact/event identity blockers found and fixed; final focused review found no blockers.

### ME3 - Review Feedback Loop

Status: pending

Acceptance:

- Capture human/subagent review findings as durable evidence.
- Link findings to sessions, tasks, tools, and follow-up workpad items.
