# Prototype References

Record implementation references once architecture selects the scaffold.

## Candidate Protocol/SDK Links

- https://github.com/agentclientprotocol/agent-client-protocol

## Candidate Runtime Links

To fill during architecture/prototype.

## Architecture Inputs

Observed 2026-05-25.

- Prototype plan: `../architecture/prototype-plan.md`
- Architecture gate review: `../architecture/gate-review.md`
- Boundary contracts: `../architecture/boundaries.md`
- State/event model: `../architecture/state-model.md`
- ACP replay/dedupe model: `../architecture/acp-replay-dedupe.md`
- Capability and permission model: `../architecture/capability-permissions.md`
- Runtime and tunnel plan: `../architecture/runtime-tunnel.md`
- Protocol and provider plan: `../architecture/protocol-provider.md`
- Tool exposure model: `../architecture/tool-exposure.md`
- Memory architecture: `../architecture/memory-architecture.md`

## P0 Workspace Scaffold

Observed 2026-05-25.

- Local toolchain: `rustc 1.94.1 (e408947bf 2026-03-25)`
- Local Cargo: `cargo 1.94.1 (29ea6fb6a 2026-03-24)`
- Workspace manifest: `../../Cargo.toml`
- CLI crate: `../../crates/capo-cli`
- Core crate: `../../crates/capo-core`
- State crate: `../../crates/capo-state`
- Adapter crate: `../../crates/capo-adapters`
- Runtime crate: `../../crates/capo-runtime`
- Tool crate: `../../crates/capo-tools`
- Memory crate: `../../crates/capo-memory`
- Evaluation crate: `../../crates/capo-eval`

## P1 Domain And Boundary Skeleton

Observed 2026-05-25.

- Core domain/controller preview: `../../crates/capo-core/src/lib.rs`
- Fake adapter/provider boundary: `../../crates/capo-adapters/src/lib.rs`
- Fake runtime/tunnel boundary: `../../crates/capo-runtime/src/lib.rs`
- Fake state boundary: `../../crates/capo-state/src/lib.rs`
- Fake tool/permission boundary: `../../crates/capo-tools/src/lib.rs`
- Fake memory boundary: `../../crates/capo-memory/src/lib.rs`
- Fake evaluation boundary: `../../crates/capo-eval/src/lib.rs`
- Cross-boundary wiring test: `../../crates/capo-cli/src/main.rs`

## P2 SQLite Event Store And Projections

Observed 2026-05-25.

- State implementation: `../../crates/capo-state/src/lib.rs`
- State manifest: `../../crates/capo-state/Cargo.toml`
- `rusqlite` crate: version `0.39.0`, license MIT, repository `https://github.com/rusqlite/rusqlite`, observed via `cargo info rusqlite`
- Added features: `bundled`

## P3 Fake Boundary E2E

Observed 2026-05-25.

- Controller orchestration crate: `../../crates/capo-controller/src/lib.rs`
- Controller manifest: `../../crates/capo-controller/Cargo.toml`
- Fake adapter/provider boundary methods: `../../crates/capo-adapters/src/lib.rs`
- Fake runtime attach/interrupt methods: `../../crates/capo-runtime/src/lib.rs`
- Trusted-local permission and fake tool methods: `../../crates/capo-tools/src/lib.rs`
- Fake memory packet builder: `../../crates/capo-memory/src/lib.rs`
- SQLite read-model queries and event kinds used by controller: `../../crates/capo-state/src/lib.rs`

## P4 First CLI Control Surface

Observed 2026-05-25.

- CLI command parser and renderers: `../../crates/capo-cli/src/main.rs`
- CLI manifest: `../../crates/capo-cli/Cargo.toml`
- Controller command-envelope handlers: `../../crates/capo-controller/src/lib.rs`
- SQLite agent/session/run/evidence read-model queries: `../../crates/capo-state/src/lib.rs`
- Manual smoke used `cargo run -p capo-cli` with temporary state and evidence directories.

## P5 Local Process Runtime

Observed 2026-05-25.

- Local process runtime implementation and tests: `../../crates/capo-runtime/src/lib.rs`
- Runtime/tunnel architecture source: `../architecture/runtime-tunnel.md`
- Runtime event vocabulary source: `../architecture/runtime-tunnel.md`
- No new third-party dependencies were added for P5.
