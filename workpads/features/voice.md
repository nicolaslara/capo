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

### V4 - Dogfood Readiness Conversation

Status: completed

Acceptance:

- Recognize simple dogfood-readiness questions as a read-only voice intent.
- Answer from the shared project dashboard/query contract rather than duplicating readiness logic in voice code.
- Render readiness status, component readiness booleans, blockers, and next actions.
- Preserve raw transcript non-retention and avoid mutating state.

Evidence:

- `VoiceIntentKind::DogfoodReadiness` and `VoiceReadScope::ProjectDogfoodReadiness` in `../../crates/capo-voice/src/lib.rs`.
- CLI voice dogfood-readiness rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-voice dogfood_readiness -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dogfood_readiness -- --nocapture`: passed.

Decision:

- Keep dogfood-readiness voice handling as a read-only query over `ProjectDashboard::dogfood_readiness()`. Voice does not run providers, inspect credentials, edit workpads, or create readiness artifacts.

### V5 - Recent Work Conversation

Status: completed

Acceptance:

- Recognize simple project-level recent-work questions such as "What have my agents done?"
- Recognize simple agent-level recent-work questions such as "What has fake-codex done?"
- Answer from the shared dashboard/query contract rather than adding voice-specific state reads.
- Render agents, active sessions, latest summaries, evidence refs, recent-event counts, and project evidence counts without retaining raw transcripts or mutating state.

Evidence:

- `VoiceIntentKind::RecentWork` and `VoiceReadScope::ProjectRecentWork` in `../../crates/capo-voice/src/lib.rs`.
- CLI voice recent-work rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-voice recent_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_recent_work -- --nocapture`: passed.

Decision:

- Keep recent-work voice handling read-only over `ProjectDashboard`. Voice summarizes shared read-model facts and does not infer hidden agent activity, run providers, inspect credentials, edit workpads, or retain the raw transcript.

### V6 - Review Needs Conversation

Status: completed

Acceptance:

- Recognize simple review/outcome questions such as "What needs review?"
- Answer from shared dashboard/query review findings and task outcome reports.
- Render review-finding counts, open blockers, outcome-report counts, reports with findings, latest review outcome, and linked finding/report rows.
- Preserve raw transcript non-retention and avoid mutating state.

Evidence:

- `VoiceIntentKind::ReviewNeeds` and `VoiceReadScope::ProjectReviewNeeds` in `../../crates/capo-voice/src/lib.rs`.
- CLI voice review-needs rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-voice review_needs -- --nocapture`: passed.
- `cargo test -p capo-cli voice_review_needs -- --nocapture`: passed.

Decision:

- Keep review-needs voice handling read-only over `ProjectDashboard`. Voice summarizes persisted review findings and task outcome reports, but does not create review state, infer hidden blockers, run providers, inspect credentials, edit workpads, or retain the raw transcript.

### V7 - Next Work Conversation

Status: completed

Acceptance:

- Recognize simple next-work questions such as "What should we do next?"
- Answer from shared dashboard/query workpad task rows.
- Reuse the same workpad priority semantics as the operator next-work path: actionable observed markdown status, `observed_only` Capo execution status, then deterministic path/anchor/task ordering.
- Render candidate count, selected workpad task, source ref, title, observed markdown status, Capo execution status, and default Capo task ID.
- Preserve raw transcript non-retention and avoid mutating state.

Evidence:

- `ProjectDashboard::next_workpad_task()` and `ProjectDashboard::next_workpad_candidate_count()` in `../../crates/capo-query/src/lib.rs`.
- `VoiceIntentKind::NextWork` and `VoiceReadScope::ProjectNextWork` in `../../crates/capo-voice/src/lib.rs`.
- CLI voice next-work rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query next_actionable_workpad -- --nocapture`: passed.
- `cargo test -p capo-voice next_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_next_work -- --nocapture`: passed.

Decision:

- Keep next-work voice handling read-only over `ProjectDashboard`. Voice can tell the operator what Capo sees as next, but it does not import tasks, start agents, edit workpads, run providers, or retain the raw transcript.

### V8 - Confirmed Start Next Work Conversation

Status: completed

Acceptance:

- Recognize simple start-next commands such as "Start next task with fake-codex."
- Require visible confirmation before importing a workpad task or dispatching work.
- Reuse the existing `workpad start-next` semantics after confirmation: select the next observed-only workpad task, import it, and dispatch it through the fake/local controller path to the named registered agent.
- Audit the voice-origin approval and once-scoped grant before mutation.
- Preserve raw transcript non-retention and avoid provider CLI execution.

Evidence:

- `VoiceIntentKind::StartNextWork` in `../../crates/capo-voice/src/lib.rs`.
- Confirmed voice start-next execution path in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-voice start_next_work -- --nocapture`: passed.
- `cargo test -p capo-cli voice_confirmed_start_next_work -- --nocapture`: passed.

Decision:

- Treat voice start-next as privileged because it mutates Capo task/session state. It requires `--confirm`, uses the same voice approval queue as stop/interrupt, and remains fake/local until real provider opt-in evidence exists.

### V9 - Dispatch Status Conversation

Status: completed

Acceptance:

- Recognize simple dispatch-chain status questions such as "What is dispatch status for DISPATCH_PLAN_ID?"
- Answer from `ProjectDashboard::adapter_dispatch_status(...)` rather than duplicating plan/gate/replay/execution lookup in voice code.
- Render plan metadata, dogfood gate status, latest gate/replay/execution status, provider execution flags, credential scan status, and next action.
- Preserve raw transcript non-retention, avoid mutating state, and avoid provider CLI execution.

Evidence:

- `VoiceIntentKind::DispatchStatus` and `VoiceReadScope::ProjectDispatchStatus` in `../../crates/capo-voice/src/lib.rs`.
- CLI voice dispatch-status rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-voice dispatch_status -- --nocapture`: passed.
- `cargo test -p capo-cli voice_dispatch_status -- --nocapture`: passed.

Decision:

- Keep dispatch-status voice handling read-only over `ProjectDashboard`. Voice can explain what Capo knows about a recorded dispatch chain, but it does not rerun gates, rematerialize prompts, launch providers, inspect credentials, edit workpads, or retain the raw transcript.
