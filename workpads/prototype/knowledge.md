# Prototype Knowledge

## Objective

Record what the prototype proves, what it fails to prove, and whether it is reliable enough to become the harness for Capo's own work.

## Status

Prototype gate not passed.

Architecture gate passed 2026-05-25. Prototype P0 is ready to start.

## Initial Direction

- Build the smallest e2e loop that can actually orchestrate one coding agent.
- Persist state before adding many input surfaces.
- Keep workpads as the fallback human-readable state until dogfooding is proven.
- Follow `../architecture/prototype-plan.md`: fake boundary e2e first, then CLI, local runtime, Codex/Claude fixture adapters, opt-in real local adapter smoke, tools, memory packet, recovery, and evidence export.
- Use `../architecture/gate-review.md` for residual risks that prototype tasks must prove rather than reopen during scaffold setup.

## P0 - Workspace Scaffold And Toolchain

Status: completed on 2026-05-25.

Decisions:

- Use a Rust-first Cargo workspace for the durable prototype controller.
- Keep Python out of the P0 scaffold. Python remains available later for voice, local-model, memory-system, or research sidecars when a task proves ecosystem leverage.
- Start dependency-free. The `capo --help` skeleton is handwritten so P0 does not force a CLI dependency choice before the command model is clearer.
- Do not declare a crate license until project license files and policy are chosen.
- Use Rust 1.94.1 / Cargo 1.94.1 locally, edition 2024, resolver 3.

Workspace layout:

- `crates/capo-cli`: command-line control surface; currently provides `capo --help` and `capo version`.
- `crates/capo-core`: product vocabulary and future domain/controller types.
- `crates/capo-state`: state store and projection scaffold.
- `crates/capo-adapters`: fake, Codex, Claude Code, and ACP adapter scaffold.
- `crates/capo-runtime`: fake/local runtime runner scaffold.
- `crates/capo-tools`: Capo-owned tool list and future instrumentation.
- `crates/capo-memory`: fake packet-builder memory scaffold.
- `crates/capo-eval`: local evidence/evaluation scaffold.
- `tests/e2e`: reserved for CLI/controller/state smoke tests.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- `cargo run -p capo-cli -- --help`: passed and did not read provider credentials, start agents, or create state.

Follow-up:

- P1 should replace scaffold constants with typed IDs, command envelopes, lifecycle records, and static dispatch boundary enums.
- P0 intentionally did not add Clap, SQLite, async runtime, or serialization dependencies. Add dependencies only when the implementing task needs them and after recording current release/license context.

## P1 - Core Domain And Boundary Skeleton

Status: completed on 2026-05-25.

Decisions:

- `capo-core` owns typed IDs, command envelopes, lifecycle/status vocabulary, core records, boundary binding metadata, and the persistence-free `CapoController` preview.
- Boundary crates own their static dispatch enums and fake variants:
  - `capo-adapters`: `AgentAdapter::Fake` and `ProviderConnector::Fake`
  - `capo-runtime`: `RuntimeRunner::Fake` and `ConnectivityTunnel::Fake`
  - `capo-state`: `StateStore::Fake`
  - `capo-tools`: `ToolExposure::Fake` and `PermissionPolicy::Fake`
  - `capo-memory`: `MemoryBackend::Fake`
  - `capo-eval`: `EvaluationLayer::Fake`
- `capo-core` does not depend on boundary crates. `capo-cli` depends on all boundary crates and owns the cross-boundary fake wiring test to avoid dependency cycles.
- The P1 controller is deliberately a preview, not a real orchestrator. It proves command target validation and required boundary presence without persistence or side effects.
- No new third-party dependencies were added.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed, including `fake_boundaries_wire_through_controller_without_persistence`.
- `cargo run -p capo-cli -- --help`: passed and remains credential-free.

Follow-up:

- P2 should replace preview-only state with append-only event/store abstractions and projection records.
- P3 should turn the fake boundary wiring into a real fake e2e loop through the controller/state store.

## P2 - SQLite Event Store And Projections

Status: completed on 2026-05-25.

Decisions:

- Add `rusqlite 0.39.0` with the `bundled` feature to `capo-state` for deterministic local prototype tests. `cargo info rusqlite` reported license MIT.
- Use append-only `events` plus a replayable `projection_records` table. This lets P2 rebuild read models deterministically without pretending final event JSON parsing exists yet.
- Keep artifacts as explicit rows with redaction state, URI, hash, size, and owner refs. P2 records artifact metadata but does not write artifact file contents yet. Normal artifact persistence is fail-closed to `safe` or `redacted`; `unknown` and `contains_sensitive` rows are rejected until a quarantine path exists.
- Implement projection tables for projects, tasks, agents, sessions, runs, capability grants, tool calls, memory packet refs, and evidence.
- Implement restart recovery shape with `recovery_attempts`, `begin_recovery`, and `complete_recovery` without mutating existing events.
- Define the projection watermark as the latest event sequence considered by rebuild, including events with no projection records.
- Store `idempotency_key` on events for later replay/dedupe work, but P2 does not enforce idempotent append yet.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- State tests cover event/projection persistence, artifact/tool/grant/memory/evidence persistence and projection rebuild, projection watermark recovery after events with no projection records, fail-closed malformed projection decode, safe/redacted artifact persistence policy, and recovery attempt bookkeeping.

Follow-up:

- P3 should drive these state APIs through the fake controller/adapter/runtime/tool/memory loop.
- P4 should expose read models through CLI commands instead of tests only.
- P10 should either enforce event idempotency with a partial unique index or route ACP replay dedupe through explicit lookup behavior before broad restart/replay claims.
- Future hardening should replace generic projection rows with typed event payload parsing once command semantics stabilize.

## P3 - Fake Boundary E2E

Status: completed on 2026-05-25.

Decisions:

- Add `capo-controller` as the orchestration crate. `capo-core` remains shared domain vocabulary; concrete boundary calls stay in their owning crates.
- Implement `FakeBoundaryController` as a real fake-only loop over `FakeAdapter`, `FakeRuntimeRunner`, fake provider metadata, `AllowTrustedLocalProfilePolicy`, fake Capo tools, fake memory packets, and SQLite state.
- Keep trusted-local permissions explicit and non-fake. The policy allows broadly for the prototype but still emits a durable capability grant projection.
- Persist separate events for session start, capability grant, tool request, tool completion, memory packet build, evidence record, and interrupt so recent-event inspection proves boundary flow instead of hiding work under one event.
- Reuse the fake runtime process ref and adapter session ref returned by `send_task` when interrupting. P3 does not yet persist those refs in dedicated read-model columns; P10/P5 will harden runtime recovery.
- Controller observations are read from SQLite projections and `recent_events_for_session`, not from live fake objects.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- `fake_boundaries_drive_controller_state_and_interrupt_from_read_models` covers send/observe/interrupt, read-model status/summary/confidence, separate permission/tool/memory/evidence event kinds, and reopening SQLite state after the fake run.

Review:

- A focused review found misleading interrupt behavior, over-combined events, wrong registration event naming, and dropped session title. All were fixed before completion.

Follow-up:

- P4 should wire this controller loop into CLI commands instead of test-only APIs.
- P5/P10 should persist adapter session refs and runtime process refs as first-class read models before local process recovery claims.

## P4 - First CLI Control Surface

Status: completed on 2026-05-25.

Decisions:

- Keep the CLI dependency-free for P4. The command grammar is handwritten and intentionally narrow while the Capo command model is still stabilizing.
- Wire write commands through `CommandEnvelope`-taking controller methods. The CLI still renders directly, but it does not own orchestration state.
- Support `init`, `agent register`, `agent spawn`, `agent list`, `task send`, `session status`, `session interrupt`, `session stop`, `recover`, and `evidence export`.
- Treat `agent spawn` honestly in P4: it creates the fake agent identity and records that the fake runtime starts on `task send`. Real runtime spawn semantics are deferred to P5.
- Make `session stop` distinct from interrupt: stop emits `session.stopped`, sets the session to `completed`, and sets the run to `exited`.
- Route `recover` through controller recovery bookkeeping with `begin_recovery`, projection rebuild, and `complete_recovery`.
- Add `latest_blocker` to the session read model so CLI status renders blocker state from SQLite instead of fabricating it.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Manual CLI smoke passed with temporary state and evidence directories: init, spawn, task send, status, recover, evidence export, and stop.

Review:

- A focused review found envelope handling, spawn naming, stop behavior, recover behavior, and blocker rendering gaps. All were fixed before completion.

Follow-up:

- P5 should replace fake spawn semantics with local runtime process lifecycle.
- P10 should harden repeated command idempotency and recovery behavior for multiple repeated CLI invocations in the same store.

## P5 - Local Process Runtime

Status: completed on 2026-05-25.

Decisions:

- Add `LocalProcessRunner` as the first real runtime boundary while keeping connectivity as a separate `ConnectivityTunnel` enum. `LocalLoopbackTunnel` is only a connectivity binding, not process execution.
- Keep the local process API explicit: request records carry `program`, `argv`, `cwd`, and request environment overrides; config records carry workspace roots, artifact root, environment allowlist, redaction rules, and output byte limit.
- Reject process working directories outside configured workspace roots before spawn.
- Clear the child environment by default, then restore only configured allowlisted host variables and request overrides that are also allowlisted.
- Capture stdout/stderr as bounded artifacts with deterministic content hashes and redaction metadata. Rule-based redaction marks artifacts as `redacted`; otherwise they are `safe`.
- Emit normalized runtime events using the runtime architecture vocabulary: `runtime.start_requested`, `runtime.process_started`, `runtime.output_delta`, `runtime.output_artifact_recorded`, `runtime.process_exited`, and control events for interrupt, terminate, and kill.
- Support both synchronous command execution and live child handles for health checks, kill, wait, and artifact collection. This is enough for P5; controller/state persistence of runtime refs remains a P10 hardening area.
- Preserve captured artifact directories during cleanup and write a cleanup marker instead of deleting durable evidence.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Runtime tests cover redacted stdout/stderr artifact capture, normalized output artifact events, interrupt/terminate/kill control events, live child kill and wait, cwd rejection, env override rejection, health, cleanup, and orphan recovery behavior.

Review:

- A focused review found issues in an earlier draft: no live child handle, no external PID, misleading redaction metadata, request env allowlist bypass, non-architecture event names, artifact deletion during cleanup, and unstable content hashing. These were fixed before completion.

Follow-up:

- P10 should persist local runtime process refs as first-class read models when restart recovery and replay are implemented.
- P7 should reuse the same runtime boundary for safe opt-in Codex and Claude Code local adapter smoke tests.

## P6 - Adapter Fixture Parsers

Status: completed on 2026-05-25.

Decisions:

- Add fixture parsers to `capo-adapters` for Codex JSONL, Claude Code stream JSON, and ACP replay JSONL before attempting real local subscription-backed smoke tests.
- Add `serde 1.0.228` and `serde_json 1.0.150` only to `capo-adapters`. `cargo info` reported both as `MIT OR Apache-2.0`.
- Parse provider streams with `serde_json::Value` for the prototype. The point of P6 is normalization boundaries and replay evidence, not claiming complete vendor schemas.
- Normalize all fixture records into `NormalizedAdapterEvent` records carrying adapter kind, normalized kind, external refs, timeline key, timeline confidence, content/tool/status/usage fields, raw event hash, and optional idempotency key.
- Keep provider-specific event kinds in `provider_event_kind`; controller code should consume normalized event fields rather than Codex, Claude, or ACP raw fields.
- Treat Codex item/tool IDs and Claude message/tool IDs as stable fixture timeline keys.
- Treat ACP `toolCallId` as stable, and ACP message chunks as heuristic because stable ACP v1 message chunks do not provide a message ID.
- Add a dedupe helper keyed by normalized idempotency keys and prove duplicate ACP tool updates collapse while raw fixture observations remain countable.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Adapter tests cover Codex session/message/tool/usage normalization, Claude session/message/tool/usage normalization, ACP session/message/plan/tool normalization, heuristic ACP message timeline confidence, stable ACP tool timeline confidence, and duplicate ACP tool update dedupe by idempotency key.

Follow-up:

- P7 should use these parser types as the safety gate before any opt-in real Codex or Claude Code local smoke.
- P10 should move ACP replay batches/raw updates/idempotency enforcement into state so dedupe is durable across restart, not just parser-local.

## P7 - Real Local Adapter Smoke

Status: waiting on opt-in as of 2026-05-25.

Decisions:

- Add the local adapter smoke harness before executing any subscription-backed CLI. Real Codex and Claude Code calls must be explicitly opted in with environment flags.
- Keep Codex launch defaults restrictive: `codex exec --json --sandbox read-only --ephemeral --ignore-user-config --ignore-rules --cd <isolated workspace>`.
- Add a Claude Code smoke plan, but do not claim the real smoke is safe until the installed CLI's restricted permission/tool arguments are verified. The current planned shape uses stream JSON, plan permission mode, disallowed tools, and strict empty MCP config.
- Route smoke execution through `LocalProcessRunner` rather than spawning provider CLIs directly from adapter code.
- Scan stdout/stderr artifacts after execution and fail if unredacted credential/session markers appear. Redacted markers are allowed only when the artifact contains `[REDACTED]`.
- Do not mark P7 complete until the real Codex opt-in smoke has been run and recorded. The current commit proves harness safety and local runtime integration with `/bin/echo` only.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Tests cover restrictive Codex smoke-plan arguments, restricted Claude smoke-plan arguments, explicit opt-in skip behavior, runtime-boundary execution with `/bin/echo`, and artifact secret-marker scanning.

Skipped verification:

- Real Codex smoke was not run because `CAPO_RUN_CODEX_LOCAL_SMOKE=1` was not set by the user.
- Real Claude Code smoke was not run because restricted tool/permission arguments still need a local CLI verification pass before enabling it.

## P8 - Capo Tools And Permission Audit

Status: completed on 2026-05-25.

Decisions:

- Add `CapoToolRegistry` as the first non-fake `ToolExposure` variant while preserving `PermissionPolicy` as a separate boundary.
- Register the first six Capo-owned tools from `tool-exposure.md`: `capo.task_status`, `capo.agent_status`, `capo.session_summary`, `capo.workpad_read`, `capo.evidence_record`, and `capo.capability_request`.
- Keep first tool handlers simple and context-driven. They read supplied task/agent/session/workpad/evidence/capability context and return deterministic output; controller/state integration can replace those context inputs with live read-model queries in later slices.
- Each tool definition records origin, handler kind, schema JSON, required scopes, risk, exposure, instrumentation level, status, and whether it mutates state.
- Trusted-local permission still allows the call, but authorization emits permission request/decision audit events and returns a capability grant.
- Tool invocation emits the full lifecycle required by the architecture: tool request, permission request, permission decision, grant use, invocation start, output artifact recorded, output observed, call completed, and result delivered.
- Extend the fake controller e2e path so these lifecycle events are durable Capo events visible from session read-model inspection.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Tool tests cover the six definitions, required scopes, context output for all six tools, trusted-local permission allow, and audit lifecycle event ordering.
- Controller tests cover durable session events for permission decision, grant creation/use, tool request, invocation, output artifact, output observation, completion, result delivery, memory packet, and evidence.

Follow-up:

- Later P8/P10 hardening should add typed `tool_invocations`, `tool_definitions`, and `tool_observations` read models instead of carrying the extra lifecycle only as event rows.
- P9 should use the tool/evidence outputs as memory packet provenance inputs.

## P9 - Memory Packet And Context Provenance

Status: completed on 2026-05-25.

Decisions:

- Add source-linked packet building to `MemoryBackend::Fake` rather than adding a second memory backend before local provenance is proven.
- Memory packet candidates carry title, body, source kind/ref/anchor/hash, review state, sensitivity, estimated tokens, and inclusion reason.
- Packet selection includes reviewed non-secret candidates within budget and excludes generated, rejected/superseded/invalidated, secret, and over-budget candidates with explicit reasons.
- Packet artifacts and explanation artifacts are rendered as markdown strings. The packet artifact is replayable prompt-input evidence; the explanation artifact records why candidates were included or excluded.
- Fake controller task execution now builds a source-linked packet from current goal, tool output summary, and prototype workpad pointer, while excluding an unreviewed generated scratch note.
- The controller records both packet and explanation artifact metadata and attaches the packet to the run/turn through the existing memory packet projection.
- Add `memory_packets_for_session` to SQLite state so the attached packet can be inspected from read models.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- Memory tests cover reviewed source inclusion, source refs and inclusion reasons, generated/secret/over-budget exclusion reasons, and no secret content in packet markdown.
- Controller/state tests cover attached memory packet projection, run/turn refs, packet artifact ID, explanation artifact ID in the memory event, and included/excluded counts.

Follow-up:

- P10 should make memory packet attachment replay/idempotency robust across restart.
- Future memory work should add typed memory record/source/read models rather than relying only on packet projection plus artifact metadata.

## P10 - Restart Recovery And Replay

Status: completed on 2026-05-25.

Decisions:

- Enforce durable replay idempotency in SQLite with project-scoped `(project_id, idempotency_key)` duplicate lookup plus a partial unique index for non-null project/idempotency pairs.
- Duplicate appends return the original event sequence and do not write projection records again. This keeps `session/load` or adapter replay from producing duplicate read-model rows when stable normalized idempotency keys are present.
- Keep idempotency project-scoped. Null-project events are not deduped by the partial index and should be used only for unscoped internal records.
- Add `tool_calls_for_session` so replay and later dashboard surfaces can inspect adapter-native tool-call read models.
- Restart recovery now rebuilds projections first, then marks active-looking runs for the current project as `exited_unknown` with durable `run.exited` events.
- Active-looking run recovery is scoped through the run's session project rather than all runs in the SQLite store.
- ACP compatibility remains fixture-level only. The P10 test proves stable ACP tool updates are durable and deduped through state, not that Capo is a full ACP server/client yet.

Verification:

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test`: passed.
- State tests cover project-scoped idempotency lookup, projection non-duplication, active-looking run exit-marking, and idempotent repeated recovery.
- Controller tests replay the ACP fixture's stable `toolCallId` updates through SQLite twice, including a duplicate completed update, then rebuild projections and verify there are three durable events and one completed tool-call read model.

Follow-up:

- P11 can export evidence after interrupted or recovered runs now that exit-marked active runs survive restart as durable state.
- Future ACP work should add raw update batch storage and explicit adapter replay start/completed events if Capo needs full `session/load` transcript replay rather than only normalized read-model dedupe.

## Prototype Gate

Status: not passed.

Required evidence:

- Spawn/register an agent.
- Send and interrupt work.
- Inspect status, goal, events, latest summary, and blocker.
- Persist and recover state.
- Record evidence in workpad-like artifact.

## Open Questions

- Whether the first non-fake real adapter smoke should be Codex only or Codex and Claude Code in the same task.
- Whether the first dashboard/TUI slice must precede dogfood or can follow the first file-workpad dogfood migration.
- How much ACP implementation should ship in the prototype after fixture replay tests, versus remaining compatibility-only until a concrete ACP agent integration is needed.
