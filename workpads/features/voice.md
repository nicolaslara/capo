# Voice Feature

## Objective

Turn the P14 dummy transcript contract into a conversational Capo control loop that can ask about agent status, summarize progress, and steer sessions through the same controller/query/permission boundaries as CLI and dashboard.

## Prototype Inputs

- P14 defines `capo-voice` command/read-model planning for dummy transcripts.
- Raw transcripts are not retained by default; redaction is required before any durable voice-derived record.

## Dependencies

- Use dummy transcripts until controller/query/permission integration is proven.
- Real audio capture, ASR, streaming, and mobile voice are deferred.

## Tasks

### V1 - Controller Integration

Status: completed

Acceptance:

- Route `VoiceCommandPlan` commands through controller command handlers.
- Render spoken-response data from the shared query surface.
- Unknown transcripts do not mutate state.

Evidence:

- `capo voice submit --transcript TEXT [--voice-session SESSION_ID] [--actor ACTOR] [--confirm]` in `../../crates/capo-cli/src/main.rs`.
- Voice status/dashboard plans render from `capo-query::project_dashboard`.
- Voice redirect plans execute through `FakeBoundaryController::redirect_command`.
- Unknown and unconfirmed privileged plans return without appending state events.
- `cargo test -p capo-voice`: passed.
- `cargo test -p capo-cli voice -- --nocapture`: passed.

### V2 - Voice Permission Confirmation

Status: completed

Acceptance:

- Require visible confirmation for stop/interrupt and other privileged voice actions.
- Audit voice-origin permission requests and decisions.
- Preserve raw-transcript non-retention.

Evidence:

- Unconfirmed privileged `capo voice submit` queues a `voice-control` permission approval and does not stop or interrupt the session.
- Confirmed privileged voice stop/interrupt records `permission.approval_queued`, `permission.decided`, and a once-scoped voice confirmation grant before controller mutation.
- Confirmed voice stop/interrupt dispatches a generic durable reason instead of persisting the raw transcript or the voice-derived "because ..." phrase.
- `cargo test -p capo-voice`: passed.
- `cargo test -p capo-cli voice -- --nocapture`: passed.

### V3 - Retention And Redaction Smoke

Status: completed

Acceptance:

- Prove retained summaries are reviewed and redacted before memory ingestion.
- Confirm raw transcripts are absent from state and evidence artifacts.

Evidence:

- `capo voice submit --redacted-summary TEXT --reviewed-summary` writes a reviewed/redacted `MemoryRecord` plus replayable `MemorySource`.
- `--redacted-summary` without `--reviewed-summary` fails before memory ingestion.
- Regression coverage scans the test state tree and memory/event projections for the raw voice phrase.
- `cargo test -p capo-cli voice -- --nocapture`: passed.
- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
