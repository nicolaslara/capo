# Feature References

Record source links and implementation references for post-prototype feature work.

## Prototype Gate Inputs

Observed 2026-05-25.

- Prototype gate decision and feature split: `../prototype/knowledge.md`
- Prototype spec and dogfood criteria: `../prototype/spec.md`
- Prototype task evidence: `../prototype/tasks.md`
- Prototype architecture plan: `../architecture/prototype-plan.md`
- Tool and permission architecture: `../architecture/tool-exposure.md`, `../architecture/capability-permissions.md`
- Runtime and connector architecture: `../architecture/runtime-tunnel.md`, `../architecture/protocol-provider.md`
- Memory architecture: `../architecture/memory-architecture.md`
- Voice contract spike: `../../crates/capo-voice/src/lib.rs`
- P12/P13 CLI smoke and dashboard tests: `../../crates/capo-cli/src/main.rs`

## Feature Workpad Files

Observed 2026-05-25.

- Real local agent connectors: `agent-connectors.md`
- Dogfood bridge: `dogfood-bridge.md`
- Dashboard/query surface: `dashboard.md`
- Permissions and tools: `permissions-tools.md`
- Memory and evaluation: `memory-eval.md`
- Voice: `voice.md`
- Remote runtime and tunnel: `remote-runtime.md`

## F2/DB1 Workpad Index

Observed 2026-05-25.

- Workpad scanner crate: `../../crates/capo-workpads/src/lib.rs`
- Workpad scanner manifest: `../../crates/capo-workpads/Cargo.toml`
- Workspace manifest and lockfile: `../../Cargo.toml`, `../../Cargo.lock`
- SQLite workpad projections and event kind: `../../crates/capo-state/src/lib.rs`
- CLI index command and tests: `../../crates/capo-cli/src/main.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB1.

## F2/DB2 Capo Task Import

Observed 2026-05-25.

- Command intent for workpad import: `../../crates/capo-core/src/lib.rs`
- SQLite workpad lookup helpers and import event kind: `../../crates/capo-state/src/lib.rs`
- CLI import command, source-hash drift check, idempotency key, and regression test: `../../crates/capo-cli/src/main.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB2.

## F2/DB3 Reviewed Workpad Artifacts

Observed 2026-05-25.

- Command intents for proposal/apply surfaces: `../../crates/capo-core/src/lib.rs`
- Proposal event kind and artifact/evidence projections: `../../crates/capo-state/src/lib.rs`
- CLI proposal generation, guarded apply command, overwrite guards, and regression test: `../../crates/capo-cli/src/main.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB3.

## F2/DB4 Next Workpad Selection

Observed 2026-05-25.

- Read-only next workpad selection command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Workpad task read model query used by selection: `../../crates/capo-state/src/lib.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB4.

## F2/DB5 Start Next Workpad Task

Observed 2026-05-25.

- `workpad start-next` command, CLI regression coverage, and explicit task ID command envelope use: `../../crates/capo-cli/src/main.rs`
- Optional explicit task ID support in fake-controller send-task handling: `../../crates/capo-controller/src/lib.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB5.

## F2/DB6 Dogfood Readiness Surface

Observed 2026-05-26.

- Shared readiness query contract: `../../crates/capo-query/src/lib.rs`
- CLI `dogfood readiness` rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- Agent connector facts consumed by the readiness query: `agent-connectors.md`
- No new third-party dependencies were added for DB6.

## F2/DB7 Dogfood Readiness Evidence Export

Observed 2026-05-26.

- CLI `dogfood readiness --out DIR` artifact export and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared readiness query contract reused by the export: `../../crates/capo-query/src/lib.rs`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DB7.

## F3/DS5 Project Evidence Visibility

Observed 2026-05-26.

- Project-level evidence query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard project evidence contract: `../../crates/capo-query/src/lib.rs`
- CLI dashboard project evidence rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- No new third-party dependencies were added for DS5.

## F3/DS6 Dogfood Readiness Dashboard Summary

Observed 2026-05-26.

- Shared dashboard readiness computation: `../../crates/capo-query/src/lib.rs`
- CLI dashboard readiness rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- No new third-party dependencies were added for DS6.

## F6/V4 Dogfood Readiness Conversation

Observed 2026-05-26.

- Voice dogfood-readiness planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice read-contract rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared readiness query consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V4.

## F6/V5 Recent Work Conversation

Observed 2026-05-26.

- Voice recent-work planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice read-contract rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dashboard query consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V5.

## F1/AC1-AC2 Local Connector Preflight

Observed 2026-05-25.

- Codex CLI path/version: `/Users/nicolas/.nvm/versions/node/v24.10.0/bin/codex`, `codex-cli 0.133.0`.
- Codex help checked with `codex exec --help`; planned safe-smoke flags are present: `--json`, `--sandbox read-only`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, and `--cd`.
- Claude Code path/version: `/Applications/cmux.app/Contents/Resources/bin/claude`, `2.1.150 (Claude Code)`.
- Claude help checked with `claude --help`; restricted smoke flags are present: `-p`, `--output-format stream-json`, `--permission-mode plan`, `--tools`, `--disallowedTools`, `--mcp-config`, `--strict-mcp-config`, `--no-session-persistence`, and `--disable-slash-commands`.
- Local adapter smoke plan and safety scanner implementation: `../../crates/capo-adapters/src/lib.rs`.
- Agent connectors source workpad: `agent-connectors.md`.
- No new third-party dependencies were added for AC1/AC2 preflight.

## F1/AC9 Local Adapter Launch Contract

Observed 2026-05-25.

- Local subscription launch-plan contract and Codex/Claude builders: `../../crates/capo-adapters/src/lib.rs`
- Runtime request/config target types used by launch plans: `../../crates/capo-runtime/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC9.

## F1/AC10 Controller Dispatch Planning

Observed 2026-05-25.

- Controller dispatch planner and regression tests: `../../crates/capo-controller/src/lib.rs`
- CLI `adapter plan-launch` operator surface and regression tests: `../../crates/capo-cli/src/main.rs`
- Local adapter launch-plan contract reused by controller: `../../crates/capo-adapters/src/lib.rs`
- Runtime request/config target types: `../../crates/capo-runtime/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC10.

## F1/AC11 Durable Dispatch Plan Read Model

Observed 2026-05-25.

- Adapter dispatch plan projection, event kind, SQLite table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard dispatch-plan query surface: `../../crates/capo-query/src/lib.rs`
- CLI `adapter plan-launch --record` and dashboard rendering: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC11.

## F1/AC12 Workpad Next Adapter Plan

Observed 2026-05-25.

- CLI `workpad plan-next` command, dispatch-plan composition, prompt-redaction regression coverage: `../../crates/capo-cli/src/main.rs`
- Workpad scanner/read models selected by `plan-next`: `../../crates/capo-workpads/src/lib.rs`, `../../crates/capo-state/src/lib.rs`
- Adapter dispatch-plan projection reused by `plan-next`: `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC12.

## F1/AC13 Dispatch Execution Gate

Observed 2026-05-25.

- CLI `adapter dispatch-gate` command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared real-agent dogfood gate reused for dispatch gating: `../../crates/capo-query/src/lib.rs`
- Recorded dispatch plans consumed by the gate: `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC13.

## F1/AC14 Dispatch Gate Audit Trail

Observed 2026-05-25.

- Dispatch-gate projection, event kind, SQLite table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for recorded dispatch gates: `../../crates/capo-query/src/lib.rs`
- CLI `adapter dispatch-gate --record` and dashboard rendering: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC14.

## F1/AC15 Dispatch Fixture Replay

Observed 2026-05-25.

- CLI `adapter replay-dispatch` command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Controller normalized adapter event replay path reused by dispatch replay: `../../crates/capo-controller/src/lib.rs`
- Fixture parser and test fixture inputs: `../../crates/capo-adapters/src/lib.rs`, `../../crates/capo-adapters/fixtures/codex-exec.jsonl`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC15.

## F1/AC16 Dispatch Replay Read Model

Observed 2026-05-25.

- Dispatch-replay projection, event kind, SQLite table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for recorded dispatch replays: `../../crates/capo-query/src/lib.rs`
- CLI `adapter replay-dispatch` replay recording and dashboard rendering: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC16.

## F1/AC17 Dispatch Chain Status

Observed 2026-05-25.

- CLI `capo adapter dispatch-status` command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dispatch plan, gate, replay, and dogfood gate read models consumed through `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC17.

## F1/AC18 Dispatch Execution Request Audit

Observed 2026-05-25.

- CLI `capo adapter execution-request` command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dispatch execution request projection, event kind, table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for execution request rows: `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC18.

## F1/AC19 Dispatch Prompt Source Contract

Observed 2026-05-25.

- Dispatch prompt-source projection, event kind, table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for prompt source rows: `../../crates/capo-query/src/lib.rs`
- CLI plan-launch/workpad plan-next prompt-source recording and dashboard rendering: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC19.

## F1/AC20 Dispatch Prompt Materialization Dry Run

Observed 2026-05-26.

- CLI `capo adapter materialize-prompt` command and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dispatch prompt materialization projection, event kind, table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for prompt materialization rows: `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC20.

## F3/DS1 Query Surface Extraction

Observed 2026-05-25.

- Query aggregation crate: `../../crates/capo-query/src/lib.rs`
- Query crate manifest: `../../crates/capo-query/Cargo.toml`
- Workspace manifest and lockfile: `../../Cargo.toml`, `../../Cargo.lock`
- CLI dashboard rendering through query contract: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- No new third-party dependencies were added for DS1.

## F3/DS2 Operator Dashboard View

Observed 2026-05-25.

- Shared dashboard query contract with tool-call and memory-packet refs: `../../crates/capo-query/src/lib.rs`
- CLI dashboard filters and text rendering: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- No new third-party dependencies were added for DS2.

## F3/DS3 Workpad Queue Visibility

Observed 2026-05-25.

- Shared dashboard workpad task rows and query regression test: `../../crates/capo-query/src/lib.rs`
- CLI dashboard workpad task rendering and dogfood bridge regression coverage: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DS3.

## F3/DS4 Workpad Queue Filters

Observed 2026-05-25.

- Shared workpad filter query fields and regression tests: `../../crates/capo-query/src/lib.rs`
- CLI dashboard `--workpad-path` / `--workpad-status` filters and regression tests: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- Dogfood bridge source workpad: `dogfood-bridge.md`
- No new third-party dependencies were added for DS4.

## F4/PT1 Static Policy Variant

Observed 2026-05-25.

- Static permission policy, structured scope parsing, scoped grant IDs, and policy tests: `../../crates/capo-tools/src/lib.rs`
- Capability grant projection metadata and migration/readback: `../../crates/capo-state/src/lib.rs`
- Controller denied-permission stop path and scoped permission event IDs: `../../crates/capo-controller/src/lib.rs`
- Tools manifest dependency: `../../crates/capo-tools/Cargo.toml`
- Workspace lockfile: `../../Cargo.lock`
- Dependency check: `cargo info serde_json` reported version `1.0.150`, license `MIT OR Apache-2.0`, rust-version `1.71`, repository `https://github.com/serde-rs/json`.
- Permissions/tools source workpad: `permissions-tools.md`

## F4/PT2 User Approval Queue

Observed 2026-05-25.

- Approval queue read model, transactional pending-status decision guard, grant-created event persistence, and replay JSON validation: `../../crates/capo-state/src/lib.rs`
- CLI approval commands and tests: `../../crates/capo-cli/src/main.rs`
- Permission approval command intents: `../../crates/capo-core/src/lib.rs`
- CLI/state manifests with `serde_json` for JSON validation: `../../crates/capo-cli/Cargo.toml`, `../../crates/capo-state/Cargo.toml`
- Workspace lockfile: `../../Cargo.lock`
- Permission lifecycle and ACP allow/reject mapping source: `../architecture/capability-permissions.md`
- Permissions/tools source workpad: `permissions-tools.md`

## F4/PT3 Tool Wrapper Expansion

Observed 2026-05-25.

- Runtime wrapper registry, execution boundary, permission binding, artifact metadata, and wrapper tests: `../../crates/capo-tools/src/lib.rs`
- Runtime wrapper dependency: `../../crates/capo-tools/Cargo.toml`
- Local process runner used by shell/git wrappers: `../../crates/capo-runtime/src/lib.rs`
- Workspace lockfile: `../../Cargo.lock`
- Tool exposure and wrapper architecture source: `../architecture/tool-exposure.md`
- Runtime execution architecture source: `../architecture/runtime-tunnel.md`
- Capability and permission source: `../architecture/capability-permissions.md`
- Permissions/tools source workpad: `permissions-tools.md`

## F5/ME1 Memory Record Read Models

Observed 2026-05-25.

- Memory record/source projections, replayable packet eligibility query, projection replay encoding/decoding, event kinds, and regression tests: `../../crates/capo-state/src/lib.rs`
- Memory architecture source for record/source fields and packet provenance rules: `../architecture/memory-architecture.md`
- Memory/evaluation source workpad: `memory-eval.md`
- Existing source-linked packet builder retained as packet artifact evidence path: `../../crates/capo-memory/src/lib.rs`
- No new third-party dependencies were added for ME1.

## F5/ME2 Task Outcome Report

Observed 2026-05-25.

- Task outcome report builder, markdown rendering, terminal-run guard, and report derivation tests: `../../crates/capo-eval/src/lib.rs`
- Task outcome report projection, event kind, rebuild decode/encode, read query, and rebuild test: `../../crates/capo-state/src/lib.rs`
- CLI export command, report artifact persistence, idempotency, review-outcome derivation, overwrite guards, and e2e regression coverage: `../../crates/capo-cli/src/main.rs`
- Eval crate dependency on state read models: `../../crates/capo-eval/Cargo.toml`, `../../Cargo.lock`
- Memory/evaluation source workpad: `memory-eval.md`
- No new third-party dependencies were added for ME2.

## F5/ME3 Review Feedback Loop

Observed 2026-05-25.

- Review finding command intent: `../../crates/capo-core/src/lib.rs`
- Review finding projection, event kind, rebuild encode/decode, read query, and rebuild test: `../../crates/capo-state/src/lib.rs`
- CLI review recording command, guarded review artifacts, tool/workpad link validation, review outcome derivation, and e2e regression coverage: `../../crates/capo-cli/src/main.rs`
- Memory/evaluation source workpad: `memory-eval.md`
- No new third-party dependencies were added for ME3.

## F5/ME4 Review Finding Dashboard Visibility

Observed 2026-05-26.

- Project-scoped review finding state lookup: `../../crates/capo-state/src/lib.rs`
- Shared dashboard review finding contract: `../../crates/capo-query/src/lib.rs`
- CLI dashboard review finding rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Memory/evaluation source workpad: `memory-eval.md`
- No new third-party dependencies were added for ME4.

## F5/ME5 Task Outcome Dashboard Visibility

Observed 2026-05-26.

- Project/session task outcome report state lookups: `../../crates/capo-state/src/lib.rs`
- Shared dashboard task outcome report contract: `../../crates/capo-query/src/lib.rs`
- CLI dashboard task outcome rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Memory/evaluation source workpad: `memory-eval.md`
- No new third-party dependencies were added for ME5.

## F6/V1 Voice Controller Integration

Observed 2026-05-25.

- Dummy transcript planning contract: `../../crates/capo-voice/src/lib.rs`
- CLI voice submit route, controller dispatch, shared query rendering, and regression tests: `../../crates/capo-cli/src/main.rs`
- Shared voice/dashboard read contract: `../../crates/capo-query/src/lib.rs`
- Controller redirect/stop command handlers used by voice plans: `../../crates/capo-controller/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V1; `capo-cli` now depends on the existing workspace crate `capo-voice`.

## F6/V2 Voice Permission Confirmation

Observed 2026-05-25.

- Voice stop/interrupt planning and confirmation-required contract tests: `../../crates/capo-voice/src/lib.rs`
- Voice confirmation queue/decision implementation and regression tests: `../../crates/capo-cli/src/main.rs`
- Permission approval and capability grant projections reused by voice confirmation: `../../crates/capo-state/src/lib.rs`
- Voice permission scopes and raw transcript policy source: `../architecture/capability-permissions.md`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V2.

## F6/V3 Voice Retention And Redaction Smoke

Observed 2026-05-25.

- Reviewed/redacted voice summary ingestion and regression tests: `../../crates/capo-cli/src/main.rs`
- Memory record/source projection model reused by voice retention smoke: `../../crates/capo-state/src/lib.rs`
- Voice retention policy contract: `../../crates/capo-voice/src/lib.rs`
- Memory architecture source for reviewed/redacted voice summary policy: `../architecture/memory-architecture.md`
- Capability model source for raw voice transcript exclusion: `../architecture/capability-permissions.md`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V3.

## F1/AC3a Deterministic Adapter Replay

Observed 2026-05-25.

- Controller replay seam and Codex/Claude replay regressions: `../../crates/capo-controller/src/lib.rs`
- Normalized adapter event parsers and fixtures: `../../crates/capo-adapters/src/lib.rs`, `../../crates/capo-adapters/fixtures/codex-exec.jsonl`, `../../crates/capo-adapters/fixtures/claude-code-stream.jsonl`
- State projections used by replay: `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC3a.

## F1/AC3b Adapter Fixture Replay CLI

Observed 2026-05-25.

- CLI adapter fixture replay command and evidence export regression: `../../crates/capo-cli/src/main.rs`
- Controller replay seam reused by CLI: `../../crates/capo-controller/src/lib.rs`
- Normalized adapter fixtures: `../../crates/capo-adapters/fixtures/codex-exec.jsonl`, `../../crates/capo-adapters/fixtures/claude-code-stream.jsonl`, `../../crates/capo-adapters/fixtures/acp-replay.jsonl`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC3b.

## F1/AC4 Connector Readiness Surface

Observed 2026-05-25.

- CLI readiness command, help text, and regression tests: `../../crates/capo-cli/src/main.rs`
- Smoke-plan source metadata rendered by readiness command: `../../crates/capo-adapters/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC4.

## F1/AC5 Durable Connector Readiness State

Observed 2026-05-25.

- Adapter readiness projection, event kind, migration, rebuild codec, read query, and state regression test: `../../crates/capo-state/src/lib.rs`
- `capo adapter readiness --record`, dashboard rendering, and CLI regression test: `../../crates/capo-cli/src/main.rs`
- Shared dashboard adapter-readiness query field: `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC5.

## F1/AC6 Real Smoke Evidence Contract

Observed 2026-05-25.

- Adapter smoke report projection, event kind, migration, rebuild codec, read query, and state regression test: `../../crates/capo-state/src/lib.rs`
- `capo adapter smoke-report record`, dashboard rendering, and CLI regression test: `../../crates/capo-cli/src/main.rs`
- Shared dashboard smoke-report query field: `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC6.

## F1/AC7 Dogfood Readiness Gate

Observed 2026-05-25.

- Shared adapter dogfood gate query contract and regression test: `../../crates/capo-query/src/lib.rs`
- `capo adapter dogfood-gate`, dashboard gate rendering, and CLI regression test: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC7.

## F1/AC8 Smoke Artifact Scan Enforcement

Observed 2026-05-25.

- Smoke artifact scan command, passed-report scan enforcement, and CLI regression tests: `../../crates/capo-cli/src/main.rs`
- Shared sensitive marker scanner reused by CLI enforcement: `../../crates/capo-adapters/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC8.

## F7/RR1 Loopback Remote Runtime Contract

Observed 2026-05-25.

- Remote process runner, loopback config, lifecycle events, and contract test: `../../crates/capo-runtime/src/lib.rs`
- Runtime/tunnel separation architecture: `../architecture/runtime-tunnel.md`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR1.

## F7/RR2 Tunnel Adapter Stub

Observed 2026-05-25.

- Tunnel endpoint stub, endpoint config, resolved endpoint, connectivity health, exposure report, and contract tests: `../../crates/capo-runtime/src/lib.rs`
- Runtime/tunnel separation and connectivity event model architecture: `../architecture/runtime-tunnel.md`
- Capability permission scopes for private/public network exposure: `../architecture/capability-permissions.md`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR2.

## F7/RR3 Explicit Exposure Policy

Observed 2026-05-25.

- Connectivity exposure projection, event kinds, migration, rebuild codec, read query, and regression test: `../../crates/capo-state/src/lib.rs`
- Runtime/tunnel exposure event model architecture: `../architecture/runtime-tunnel.md`
- Durable grant/revocation architecture: `../architecture/capability-permissions.md`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR3.

## F7/RR4 Dashboard Exposure Visibility

Observed 2026-05-25.

- Shared dashboard query exposure rows and regression test: `../../crates/capo-query/src/lib.rs`
- CLI dashboard exposure rendering and regression test: `../../crates/capo-cli/src/main.rs`
- Connectivity exposure projection source: `../../crates/capo-state/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR4.

## F7/RR5 Connectivity Exposure Operator Surface

Observed 2026-05-26.

- CLI `connectivity expose-stub` command, endpoint parsing, exposure projection recording, and regression test: `../../crates/capo-cli/src/main.rs`
- Connectivity endpoint stub and exposure scope contracts reused by the command: `../../crates/capo-runtime/src/lib.rs`
- Connectivity exposure projection source: `../../crates/capo-state/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR5.

## F7/RR6 Connectivity Exposure Approval Bridge

Observed 2026-05-26.

- CLI `connectivity request-approval` and `connectivity activate-exposure` commands plus blocked -> approval -> grant -> active regression test: `../../crates/capo-cli/src/main.rs`
- Existing permission approval and capability grant projections reused by the bridge: `../../crates/capo-state/src/lib.rs`
- Shared dashboard exposure rendering used to verify active state: `../../crates/capo-cli/src/main.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR6.

## F7/RR7 Connectivity Exposure Revocation Surface

Observed 2026-05-26.

- CLI `connectivity revoke-exposure` command and active -> revoked dashboard regression path: `../../crates/capo-cli/src/main.rs`
- Connectivity exposure revocation event kind and projection fields reused from state layer: `../../crates/capo-state/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR7.

## F1/AC21 Real Dispatch Runner Preflight

Observed 2026-05-26.

- CLI `adapter run-preflight` command, preflight composition, and regression coverage: `../../crates/capo-cli/src/main.rs`
- Dispatch plan, execution request, and prompt materialization read models consumed through shared query surface: `../../crates/capo-query/src/lib.rs`, `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC21.

## F1/AC22 Guarded Local Dispatch Runner Surface

Observed 2026-05-26.

- CLI `adapter run-local` command, fail-closed preflight consumption, workpad prompt reconstruction, and regression coverage: `../../crates/capo-cli/src/main.rs`
- Local provider launch plans reused by the runner: `../../crates/capo-adapters/src/lib.rs`
- Process execution boundary used only after explicit opt-in: `../../crates/capo-runtime/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC22.

## F1/AC23 Dispatch Execution Outcome Read Model

Observed 2026-05-26.

- Dispatch execution projection, event kind, SQLite table, rebuild codec, and read query: `../../crates/capo-state/src/lib.rs`
- Shared dashboard query field for local dispatch executions: `../../crates/capo-query/src/lib.rs`
- CLI `run-local --record` outcome recording and dashboard rendering: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC23.

## F1/AC24 Dispatch Status Execution Introspection

Observed 2026-05-26.

- CLI `dispatch-status` execution outcome rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dashboard execution rows consumed by status rendering: `../../crates/capo-query/src/lib.rs`
- Dispatch execution projection source: `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC24.

## F1/AC25 Dispatch Chain Evidence Export

Observed 2026-05-26.

- CLI `dispatch-evidence` command, markdown renderer, guarded writer, and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dashboard projections consumed by the export: `../../crates/capo-query/src/lib.rs`
- Evidence and artifact projection types reused by the export: `../../crates/capo-state/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC25.

## F1/AC26 Dispatch Status Query Contract

Observed 2026-05-26.

- Shared dispatch status summary contract and regression coverage: `../../crates/capo-query/src/lib.rs`
- CLI `dispatch-status` rendering over the shared summary: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC26.

## F1/AC27 Latest Dispatch Status Selection

Observed 2026-05-26.

- Shared latest dispatch status selector and regression coverage: `../../crates/capo-query/src/lib.rs`
- CLI `dispatch-status --latest [--agent NAME]` rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC27.

## F1/AC28 Latest Dispatch Evidence Export

Observed 2026-05-26.

- CLI latest dispatch-evidence selector and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared latest dispatch selector consumed by the export: `../../crates/capo-query/src/lib.rs`
- Agent connectors source workpad: `agent-connectors.md`
- No new third-party dependencies were added for AC28.

## F6/V6 Review Needs Conversation

Observed 2026-05-26.

- Voice review-needs planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice read-contract rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dashboard review/outcome projections consumed by voice: `../../crates/capo-query/src/lib.rs`
- Review finding and task outcome source projections: `../../crates/capo-state/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V6.

## F6/V7 Next Work Conversation

Observed 2026-05-26.

- Shared next-workpad selection helper and unit coverage: `../../crates/capo-query/src/lib.rs`
- Voice next-work planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice read-contract rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Workpad task source projections: `../../crates/capo-state/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V7.

## F6/V8 Confirmed Start Next Work Conversation

Observed 2026-05-26.

- Voice start-next planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI confirmed voice start-next execution and regression coverage: `../../crates/capo-cli/src/main.rs`
- Workpad import/start-next semantics reused by the voice path: `../../crates/capo-cli/src/main.rs`
- Voice approval and capability grant projections reused from `../../crates/capo-state/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V8.

## F6/V9 Dispatch Status Conversation

Observed 2026-05-26.

- Voice dispatch-status planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice dispatch-status rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dispatch-status query consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V9.

## F6/V10 Latest Dispatch Status Conversation

Observed 2026-05-26.

- Voice latest dispatch-status planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice latest dispatch-status rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared latest dispatch-status query consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- No new third-party dependencies were added for V10.

## F3/DS7 Shared Next Workpad Selection

Observed 2026-05-26.

- Shared workpad next selector: `../../crates/capo-query/src/lib.rs`
- CLI workpad next/plan-next/start-next routing through the shared selector: `../../crates/capo-cli/src/main.rs`
- Dashboard source workpad: `dashboard.md`
- No new third-party dependencies were added for DS7.

## F7/RR8 Connectivity Exposure Evidence Export

Observed 2026-05-26.

- CLI `connectivity exposure-evidence` command, markdown renderer, guarded writer, and regression coverage: `../../crates/capo-cli/src/main.rs`
- Connectivity exposure source projections consumed by the export: `../../crates/capo-state/src/lib.rs`
- Project-level evidence dashboard path reused by the export: `../../crates/capo-query/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR8.

## F7/RR9 Latest Connectivity Exposure Status

Observed 2026-05-26.

- Shared exact/latest connectivity exposure selectors and regression coverage: `../../crates/capo-query/src/lib.rs`
- CLI `connectivity exposure-status --exposure` and `--latest` rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Connectivity exposure source projections: `../../crates/capo-state/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR9.

## F6/V11 Latest Connectivity Exposure Conversation

Observed 2026-05-26.

- Voice latest-connectivity planning and unit coverage: `../../crates/capo-voice/src/lib.rs`
- CLI voice latest connectivity exposure rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared latest connectivity exposure selector consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for V11.

## F7/RR10 Latest Connectivity Exposure Evidence Export

Observed 2026-05-26.

- CLI latest connectivity evidence selector, renderer reuse, and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared latest connectivity selector consumed by the export: `../../crates/capo-query/src/lib.rs`
- Connectivity exposure source projections: `../../crates/capo-state/src/lib.rs`
- Remote runtime source workpad: `remote-runtime.md`
- No new third-party dependencies were added for RR10.

## F4/PT4 ACP Client Capability Gating

Observed 2026-05-26.

- ACP capability advertisement helper and regression coverage: `../../crates/capo-tools/src/lib.rs`
- ACP capability design source: `../architecture/tool-exposure.md`
- ACP provider/session setup design source: `../architecture/protocol-provider.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT4.

## F4/PT5 ACP Session Setup Capability Plan

Observed 2026-05-26.

- ACP adapter setup plan and regression coverage: `../../crates/capo-adapters/src/lib.rs`
- Capability gate consumed by setup: `../../crates/capo-tools/src/lib.rs`
- ACP session setup design source: `../architecture/protocol-provider.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT5.

## F4/PT6 ACP Client Handler Wrapper Routing

Observed 2026-05-26.

- ACP client handler routing and regression coverage: `../../crates/capo-adapters/src/lib.rs`
- Wrapper request and execution boundary consumed by routing: `../../crates/capo-tools/src/lib.rs`
- ACP tool/client handler design source: `../architecture/tool-exposure.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT6.

## F4/PT7 Adapter Native Tool Observation Contract

Observed 2026-05-26.

- Adapter observed-only tool observation contract and fixture regression coverage: `../../crates/capo-adapters/src/lib.rs`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- ACP/Codex/Claude fixture source: `../../crates/capo-adapters/fixtures/`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT7.

## F4/PT8 Observed-Only Tool Observation State Projection

Observed 2026-05-26.

- Observed-only tool observation projection, SQLite table, query method, and rebuild regression coverage: `../../crates/capo-state/src/lib.rs`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT8.

## F4/PT9 Query And Evidence Visibility For Tool Observations

Observed 2026-05-26.

- Shared session dashboard observation field and regression coverage: `../../crates/capo-query/src/lib.rs`
- CLI dashboard and session evidence observation rendering: `../../crates/capo-cli/src/main.rs`
- Observed-only tool observation source projection: `../../crates/capo-state/src/lib.rs`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT9.

## F4/PT10 Adapter Replay Tool Observation Ingestion

Observed 2026-05-26.

- Controller replay ingestion and regression coverage: `../../crates/capo-controller/src/lib.rs`
- CLI fixture replay evidence/dashboard regression coverage: `../../crates/capo-cli/src/main.rs`
- Adapter observed-only source contract: `../../crates/capo-adapters/src/lib.rs`
- Observed-only state projection: `../../crates/capo-state/src/lib.rs`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT10.

## F4/PT11 Session Status Tool Introspection

Observed 2026-05-26.

- CLI session status tool-call and tool-observation rendering: `../../crates/capo-cli/src/main.rs`
- Observed-only state projection consumed by status: `../../crates/capo-state/src/lib.rs`
- Dashboard/evidence observation rendering used for output alignment: `../../crates/capo-cli/src/main.rs`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- Permissions source workpad: `permissions-tools.md`
- No new third-party dependencies were added for PT11.

## F6/V12 Recent Work Tool Activity Conversation

Observed 2026-05-26.

- Voice recent-work read contract fields and planning tests: `../../crates/capo-voice/src/lib.rs`
- CLI voice recent-work rendering and regression coverage: `../../crates/capo-cli/src/main.rs`
- Shared dashboard session rows consumed by voice: `../../crates/capo-query/src/lib.rs`
- Voice source workpad: `voice.md`
- Tool observation architecture source: `../architecture/tool-exposure.md`
- No new third-party dependencies were added for V12.
