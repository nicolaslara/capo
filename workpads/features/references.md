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
