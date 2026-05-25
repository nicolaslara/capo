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

## F1/AC1-AC2 Local Connector Preflight

Observed 2026-05-25.

- Codex CLI path/version: `/Users/nicolas/.nvm/versions/node/v24.10.0/bin/codex`, `codex-cli 0.133.0`.
- Codex help checked with `codex exec --help`; planned safe-smoke flags are present: `--json`, `--sandbox read-only`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, and `--cd`.
- Claude Code path/version: `/Applications/cmux.app/Contents/Resources/bin/claude`, `2.1.150 (Claude Code)`.
- Claude help checked with `claude --help`; restricted smoke flags are present: `-p`, `--output-format stream-json`, `--permission-mode plan`, `--tools`, `--disallowedTools`, `--mcp-config`, `--strict-mcp-config`, `--no-session-persistence`, and `--disable-slash-commands`.
- Local adapter smoke plan and safety scanner implementation: `../../crates/capo-adapters/src/lib.rs`.
- Agent connectors source workpad: `agent-connectors.md`.
- No new third-party dependencies were added for AC1/AC2 preflight.

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
