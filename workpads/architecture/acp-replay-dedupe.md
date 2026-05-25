# ACP Replay And Dedupe

## Objective

Define how Capo should ingest ACP `session/update` streams, `session/load` history replay, `session/resume` reconnects, partial streaming updates, tool-call updates, and Capo restart recovery without duplicating dashboard/read-model state.

This is the A2a architecture artifact. It refines the generic event identity model in `state-model.md` for ACP specifically.

## Sources Observed

Observed 2026-05-25 from the official ACP repository clone at `workpads/references/repos/agent-client-protocol`, commit `ec66afe2f0f9fce4e3348b38f8007b5583e4b20f`.

Primary source paths:

- `README.md`
- `docs/protocol/initialization.mdx`
- `docs/protocol/session-setup.mdx`
- `docs/protocol/prompt-turn.mdx`
- `docs/protocol/tool-calls.mdx`
- `docs/protocol/agent-plan.mdx`
- `docs/announcements/session-resume-stabilized.mdx`
- `docs/rfds/message-id.mdx`
- `schema/schema.json`

## ACP Facts That Matter

- ACP wire compatibility is negotiated through integer `protocolVersion`; the current stable protocol version observed is `1`.
- Capabilities are optional. Omitted capabilities mean unsupported.
- Baseline agents support `session/new`, `session/prompt`, `session/cancel`, and `session/update`.
- `session/load` is only available when the agent advertises `loadSession`.
- `session/load` must replay the entire conversation to the client through `session/update` notifications, then respond to the original request.
- `session/resume` reconnects to an existing session without replaying conversation history when `sessionCapabilities.resume` is advertised.
- `session/update` variants in stable schema include message chunks, thought chunks, tool calls, tool-call updates, plan updates, available command updates, current mode updates, config option updates, and session info updates.
- Stable message chunk updates do not have a stable `messageId` field in `schema/schema.json`.
- The ACP Message ID RFD explains that missing message IDs make consecutive same-type messages ambiguous and make load/session dedupe difficult. The Rust schema currently gates message IDs behind `unstable_message_id`.
- Even in the Message ID RFD, agents may preserve original message IDs across `session/load` or generate new ones; stability across loads is not guaranteed.
- `toolCallId` is stable within an ACP session and required for `tool_call` and `tool_call_update`.
- `tool_call_update` fields are partial replacements. `content` and `locations` replace collections, not append.
- ACP plan updates are complete replacements of the current plan.
- After `session/cancel`, agents may still send pending updates before responding to the original prompt with stop reason `cancelled`.

## Design Decision

Capo must not make ACP replayed `session/update` notifications directly authoritative for UI state.

Instead:

1. Persist every raw ACP update in a replay/live ingest batch.
2. Normalize raw ACP updates into Capo events through idempotent mappers.
3. Project UI/read models only from Capo event sequences.
4. Prefer `session/resume` over `session/load` when Capo already has complete local history and the agent supports resume.
5. Use `session/load` for foreign session import, explicit repair/reconciliation, or agents that lack resume.
6. Treat `session/load` updates as replay input that may import missing history, reconcile current state, or be recorded as duplicate observations.

## New Vocabulary

### AcpReplayBatch

One bounded stream of ACP updates observed by Capo.

Fields:

- `acp_replay_batch_id`
- `capo_session_id`
- `external_session_ref`
- `source`: `live_prompt`, `session_load`, `session_resume_attach`, `restart_recovery`, `foreign_import`
- `started_at`
- `completed_at?`
- `load_request_id?`
- `prompt_request_id?`
- `recovery_attempt_id?`
- `raw_update_count`
- `normalized_event_range?`
- `status`: `open`, `completed`, `failed`, `superseded`

### AcpRawUpdate

One raw `session/update` notification or related ACP response observed during a batch.

Fields:

- `acp_raw_update_id`
- `acp_replay_batch_id`
- `external_session_ref`
- `batch_index`
- `jsonrpc_method`
- `session_update_kind?`
- `external_item_ref?`
- `payload_hash`
- `payload_artifact_id`
- `observed_at`
- `dedupe_confidence`: `stable`, `heuristic`, `none`

### AcpTimelineKey

Protocol-aware key used by normalization and projection to map ACP updates to Capo items.

Fields:

- `external_session_ref`
- `kind`
- `stable_ref?`
- `synthetic_ref?`
- `confidence`

Examples:

- Tool call: `acp:{session}:tool:{toolCallId}`
- Plan: `acp:{session}:plan:current`
- Session info: `acp:{session}:session_info`
- Message with negotiated stable schema ID: `acp:{session}:message:{messageId}`
- Message without ID: `acp:{session}:message:{role}:{batch_order_group}:{content_hash_window}` with low confidence

## Raw Storage Rules

- Store ACP raw updates before normalization.
- `batch_index` is monotonically increasing within a batch.
- Raw update identity is `(acp_replay_batch_id, batch_index)`.
- Raw update dedupe across batches is advisory only and uses payload hash plus protocol-aware keys.
- Raw update rows may point at artifacts instead of storing large JSON inline.
- Raw updates never mutate read models directly.

## Normalized Event Rules

Every normalized event from an ACP update includes:

- `external_ref.adapter = "acp"`
- `external_ref.external_session_id`
- `external_ref.acp_update_kind`
- `external_ref.acp_replay_batch_id`
- `external_ref.acp_raw_update_id`
- `external_ref.acp_timeline_key?`
- `external_ref.replay_source`
- `idempotency_key`

Idempotency key shape:

```text
acp:{capo_session_id}:{event_family}:{timeline_key}:{operation}:{operation_version}
```

Rules:

- For stable external IDs, `timeline_key` uses that ID.
- For message chunks without stable IDs, Capo stages candidate items outside the append-only event log and finalizes them to content hashes at turn/load completion.
- If a replayed candidate item has the same role, normalized content hash, and surrounding stable anchors as an existing finalized Capo item, Capo appends a duplicate-observation event instead of item events.
- Duplicate raw updates can still be retained as raw observations.
- Normalized Capo event uniqueness remains stronger than raw ACP identity.

Replay staging:

- `adapter.raw_update_observed` can be committed immediately because it is a raw observation.
- Candidate `item.started`, `item.delta`, `item.completed`, and tool/plan update mappings are staged in adapter-local memory or a non-projecting staging table during `session/load`.
- Staged candidates do not advance read-model watermarks and are not replayed as product state.
- After reconciliation, Capo appends only accepted events: import/update events for missing state, duplicate/ambiguous replay marker events, or attach/replay completion events.
- Event rebuilds therefore never re-create duplicate UI items from rejected provisional candidates.

## Message Chunk Handling

ACP stable schema gives message chunks as `user_message_chunk`, `agent_message_chunk`, and `agent_thought_chunk` without stable IDs.

Capo rules:

1. Create a Capo `item.started` when a message chunk begins a new message group.
2. Append chunks with `item.delta`.
3. Keep `stream_state = open` until Capo sees one of:
   - a different update family that closes the current message group,
   - a role/type change,
   - a prompt response stop reason,
   - `session/load` completion,
   - cancellation finalization.
4. Emit `item.completed` with a normalized `content_hash` and `chunk_count`.
5. Store `message_boundary_confidence`:
   - `stable` when a supported `messageId` exists,
   - `heuristic` when inferred from ACP update ordering,
   - `low` when consecutive same-type chunks may represent one or many messages.

Replay behavior:

- If a future stable schema includes negotiated `messageId` support, use it as the message timeline key, but still tolerate changed IDs across `session/load`.
- If an adapter exposes `_meta.messageId`, treat it as adapter-specific and opt-in. It may improve a heuristic timeline key for that adapter, but it is not generic ACP identity and must not be trusted as stable unless the concrete adapter documents and tests that convention.
- Without message IDs, `session/load` import builds provisional items, finalizes them, then compares finalized role/content hashes against existing Capo items for that external session and turn/import window.
- Low-confidence matches are recorded as `adapter.replay_duplicate_detected` and not projected as UI duplicates.
- Low-confidence non-matches are imported but marked `import_confidence = low`.

## Tool Call Handling

ACP tool calls have stable `toolCallId` within a session.

Capo rules:

- `tool_call` maps to `tool.call_requested` or updates an existing tool call with the same timeline key.
- `tool_call_update` maps to one or more Capo tool events depending on changed fields:
  - status `in_progress` -> `tool.call_started`
  - status `completed` -> `tool.call_completed`
  - status `failed` -> `tool.call_failed`
  - content/rawOutput changes -> `tool.output_observed`
- `content`, `locations`, `rawInput`, and `rawOutput` are replacement fields for the ACP view. Capo stores each raw replacement and projects the latest state for the read model.
- Repeated identical updates for a tool timeline key are ignored by projection but retained as raw observations.
- Cancellation can create internal Capo `tool.call_canceled`/canceled state even though stable ACP `ToolCallStatus` only lists pending, in-progress, completed, and failed.

## Plan Handling

ACP plan updates are complete replacements.

Capo rules:

- Use one timeline key per ACP session plan: `acp:{session}:plan:current`.
- Each ACP plan update maps to `item.started` if no plan item exists, then `item.delta` or `item.completed` with replacement semantics.
- Read models render only the latest plan state.
- Event history keeps prior plan states for audit and evaluation.

## Session Info, Modes, Commands, And Config Updates

ACP session info, mode, command, and config update variants are metadata updates.

Capo rules:

- Store them as adapter metadata events, not as Capo product modes.
- `session_info_update` can update session title/metadata read models when the field is present and safe.
- `current_mode_update` is agent-reported adapter state only. Capo does not gain user-facing modes.
- Available commands and config options are adapter capabilities/metadata and should not bypass Capo command envelopes or permission policy.

## `session/load` Strategy

Use `session/load` in three cases:

1. Foreign session import where Capo has no local history.
2. Repair/reconciliation when Capo local history is incomplete.
3. Agents that lack `session/resume` but can load.

Do not use `session/load` as the default restart path when Capo already has local history and the agent supports `session/resume`.

Load algorithm:

1. Emit `adapter.replay_started` with source `session_load`.
2. Open an `AcpReplayBatch`.
3. Store every `session/update` notification as `AcpRawUpdate`.
4. Stage candidate normalized records in a non-projecting replay workspace.
5. On `session/load` response, finalize open message/tool/plan candidates.
6. Run replay reconciliation:
   - stable timeline-key match -> append an accepted update event or duplicate marker,
   - content-hash match with surrounding anchors -> duplicate observation,
   - no match -> import missing historical item,
   - ambiguous match -> import with low confidence or quarantine for review depending on risk.
7. Emit `adapter.replay_completed` with imported/duplicate/ambiguous counts.
8. Only after replay completion should UI clients see the reconciled read-model watermark.

## `session/resume` Strategy

Use `session/resume` when:

- Capo has persisted local history for the external session.
- The agent advertises `sessionCapabilities.resume`.
- Capo only needs to reconnect to the agent's context, not replay history.

Resume algorithm:

1. Emit `adapter.attach_started`.
2. Call `session/resume`.
3. Store the response as raw metadata.
4. Emit `adapter.attach_completed`.
5. Do not create message/item replay events.
6. Continue live prompt batches from the next Capo turn.

## Capo Restart Recovery Interaction

On Capo restart:

1. Rebuild read models from Capo events first.
2. For active ACP sessions, probe adapter/runtime health.
3. Prefer `session/resume` when supported.
4. If only `session/load` is available, open a replay batch and reconcile instead of streaming replayed updates directly to UI.
5. If neither is available, keep the session inspectable from Capo state and mark it non-resumable.

This keeps Capo restart recovery and ACP load replay as separate phases:

- Capo recovery establishes local event/read-model truth.
- ACP replay is adapter input that may reconcile with or extend that truth.

## UI Projection Rules

- UI clients consume Capo `sequence` and read-model watermarks.
- UI clients never render raw ACP replay batches directly.
- `adapter.replay_started` may show a session as reconciling.
- During replay, projections can update in a hidden transaction or behind a replay watermark.
- On replay completion, UI sees one coherent state: imported missing items plus duplicate counts, not duplicated message bubbles.

## Proposed State Model Additions

Add tables:

```text
adapter_replay_batches(acp_replay_batch_id, session_id, external_session_ref_json, source, started_at, completed_at, load_request_id, prompt_request_id, recovery_attempt_id, raw_update_count, normalized_sequence_start, normalized_sequence_end, status, summary_json)
adapter_raw_updates(acp_raw_update_id, acp_replay_batch_id, external_session_ref_json, batch_index, jsonrpc_method, session_update_kind, external_item_ref_json, payload_hash, payload_artifact_id, observed_at, dedupe_confidence)
adapter_timeline_keys(adapter_timeline_key_id, session_id, external_session_ref_json, kind, stable_ref, synthetic_ref, confidence, first_sequence, last_sequence)
```

Add event kinds:

- `adapter.replay_started`
- `adapter.raw_update_observed`
- `adapter.replay_duplicate_detected`
- `adapter.replay_ambiguous`
- `adapter.replay_completed`
- `adapter.attach_started`
- `adapter.attach_completed`
- `adapter.attach_failed`

Add item fields:

- `stream_state`
- `content_hash`
- `chunk_count`
- `message_boundary_confidence`
- `adapter_timeline_key_id?`
- `import_confidence?`

## Prototype Fixture Requirements

A prototype ACP adapter test should include fixtures for:

1. Live prompt stream with agent message chunks, a plan, a tool call, tool updates, and stop reason.
2. Capo restart followed by `session/resume`; assert no replayed items are added.
3. Capo restart followed by `session/load` replaying the same history; assert no duplicate UI items.
4. Foreign `session/load`; assert imported user/agent chunks and tool calls become inspectable once.
5. Repeated identical `tool_call_update`; assert one tool call read model with raw duplicate observations.
6. Consecutive same-type `agent_message_chunk` updates without message IDs; assert low boundary confidence is recorded.
7. Plan replacement; assert latest plan renders while prior plan events remain auditable.
8. Cancel while permission is pending; assert pending ACP permission is answered `cancelled`, Capo permission queue closes, and late tool updates are accepted before turn finalization.

## Recommendation

Implement ACP replay support as a conservative adapter ingestion layer:

- Capo owns durable event identity.
- `session/resume` is the default reconnect path when available.
- `session/load` is a replay/import/reconciliation operation, not a UI stream.
- Tool calls are easy to dedupe by `toolCallId`.
- Plans and metadata are replacement projections.
- Message chunks remain the hard case because stable ACP v1 lacks message IDs; Capo must finalize content hashes and record boundary confidence.
- A future stable `messageId` should be supported opportunistically, but Capo cannot depend on it for correctness.

Confidence: medium-high for ACP facts and the high-level design. Confidence is medium for message chunk dedupe without stable IDs; the prototype must keep low-confidence markers and fixtures until real adapter traces prove the heuristics.
