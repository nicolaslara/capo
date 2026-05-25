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

Status: pending

Acceptance:

- Require visible confirmation for stop/interrupt and other privileged voice actions.
- Audit voice-origin permission requests and decisions.
- Preserve raw-transcript non-retention.

### V3 - Retention And Redaction Smoke

Status: pending

Acceptance:

- Prove retained summaries are reviewed and redacted before memory ingestion.
- Confirm raw transcripts are absent from state and evidence artifacts.
