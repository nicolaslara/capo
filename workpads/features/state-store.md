# State Store Resilience Feature

## Objective

Make Capo's SQLite/event-log state layer resilient as the scaffold grows, reducing schema/query drift while preserving the append-only event model, rebuildable projections, and local-first dogfood workflow.

## Prototype Inputs

- P2 established SQLite as the prototype operational source of truth.
- Feature work has added many projections for adapters, dispatch, workpads, permissions, tools, memory, evaluation, connectivity, and runtime targets.
- The current `rusqlite` implementation is explicit and testable, but every new projection repeats schema DDL, upsert SQL, row mapping, projection-log encoding, projection-log decoding, read queries, and rebuild tests.

## Dependencies

- Keep SQLite as the local prototype store until dogfood traces prove a server/Postgres requirement.
- Preserve append-only events and rebuildable read models regardless of database library.
- Do not introduce async runtime requirements into the controller core unless the selected state library needs it and the architecture records that tradeoff.

## Tasks

### SS1 - State Store Library Decision

Status: completed

Acceptance:

- Review Rust database options against Capo's event-log/projection model.
- Decide whether manual `rusqlite` SQL remains acceptable for the next scaffold slices.
- Record the recommended migration/hardening path before adding another broad projection family.
- Do not add a new runtime dependency in this decision-only slice.

Evidence:

- Current implementation pressure point: `../../crates/capo-state/src/lib.rs`.
- `rusqlite` official repository describes it as ergonomic SQLite bindings and documents `bundled` usage plus MIT licensing: https://github.com/rusqlite/rusqlite
- Diesel official docs describe it as an ORM/query builder that reduces boilerplate and validates queries through a code schema; Diesel migrations support embedded migrations: https://docs.diesel.rs/main/diesel/index.html and https://docs.diesel.rs/main/diesel_migrations/index.html
- SQLx official repository describes an async SQL toolkit with compile-time checked queries, SQLite support, and embedded migrations: https://github.com/launchbadge/sqlx
- SeaORM official docs describe an async dynamic ORM with migration support: https://www.sea-ql.org/SeaORM/docs/index/
- No new third-party dependencies were added for SS1.

Decision:

- Manual `rusqlite` SQL was the right P2 choice because it let the scaffold prove event append, projections, artifact policy, and rebuild behavior without committing to an ORM too early.
- Manual SQL is no longer the best default for continued projection growth. It is creating repeated schema/query/mapping/rebuild code that can drift across tables.
- Keep `rusqlite` in place for now, but stop adding broad new projection families until SS2 introduces a typed projection helper or SS3 proves a Diesel migration path.
- Evaluate Diesel first because Capo is currently sync, Rust-first, SQLite-local, schema-sensitive, and projection-heavy. Diesel's schema-aware query builder and embedded migrations match those constraints better than introducing an async stack immediately.
- Treat SQLx as the second candidate if Capo's server mode or future Postgres path becomes async-first. SQLx preserves SQL visibility and compile-time checking, but it would pull the state layer toward async runtime decisions.
- Treat SeaORM as lower priority for the controller core because Capo's persistence model is event log plus projections, not CRUD-heavy active-record entities. Revisit SeaORM for a future hosted API/web service if the product shape changes.
- Keep a structured-rusqlite option alive for the shortest safe path: typed projection descriptors, centralized DDL/upsert/read/rebuild helpers, and table-specific codecs. This may reduce risk faster than an ORM migration if the Diesel spike is too invasive.

Follow-up:

- SS2 should add a small typed projection helper around one existing projection family before any new broad state model is added.
- SS3 should run a contained Diesel spike against one projection family and compare code size, compile friction, migration ergonomics, rebuild behavior, and test readability.
- After SS2/SS3, choose either a staged Diesel migration or a stricter in-house `rusqlite` projection registry.

### SS2 - State Crate Test Module Split

Status: completed

Acceptance:

- Move the large `capo-state` inline test module out of `src/lib.rs` into a separate module file.
- Preserve all existing test behavior and keep crate-root public APIs unchanged.
- Do not change schema, projection encoding, rebuild semantics, or runtime behavior in this slice.
- Run focused `capo-state` tests and the standard workspace gate before completion.

Evidence:

- `../../crates/capo-state/src/lib.rs` now keeps `#[cfg(test)] mod tests;` instead of an inline test module.
- `../../crates/capo-state/src/tests.rs` contains the former state test module body.
- `../../crates/capo-state/src/lib.rs` is now 6,109 lines and `../../crates/capo-state/src/tests.rs` is 1,876 lines.
- Xhigh split analysis recommended internal module decomposition before crate splits, with `capo-state` tests as the safest first stage.
- `git diff --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo fmt --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

Decision:

- Start with tests because this reduces the largest state file without touching projection semantics or downstream imports.
- Keep the first split mechanical and single-crate. Deeper state modules for events, projections, schema, and queries should follow only after this low-risk move lands cleanly.

### SS2a - State Event And Error Module Split

Status: completed

Acceptance:

- Move stable event envelope, redaction, artifact, recovery, and state-error types out of `src/lib.rs`.
- Preserve crate-root public APIs through re-exports so downstream crates can continue importing the same names from `capo_state`.
- Do not change SQLite schema, projection encoding, query behavior, rebuild behavior, or event kind strings.
- Run focused `capo-state` tests and the standard workspace gate before completion.

Evidence:

- `../../crates/capo-state/src/event.rs` now owns `EventKind`, `RedactionState`, `NewEvent`, `EventRecord`, `ArtifactRecord`, and `RecoveryAttempt`.
- `../../crates/capo-state/src/error.rs` now owns `StateError` and `StateResult`.
- `../../crates/capo-state/src/lib.rs` re-exports those types at the crate root.
- Resulting state crate file sizes: `../../crates/capo-state/src/lib.rs` 5,875 lines; `../../crates/capo-state/src/event.rs` 207 lines; `../../crates/capo-state/src/error.rs` 37 lines; `../../crates/capo-state/src/tests.rs` 1,876 lines.
- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

Decision:

- Split event/error types before projection internals because they are stable boundary vocabulary and can be moved without touching persisted projection rows.
- Keep the new modules private with crate-root `pub use` exports for now. This preserves the current `capo_state::{EventKind, StateError, ...}` import surface while making later `schema`, `projections`, and `queries` splits easier.

### SS2b - State Projection Type Module Split

Status: completed

Acceptance:

- Move projection record and read-model type definitions out of `src/lib.rs`.
- Preserve crate-root public APIs through re-exports so downstream crates can continue importing the same projection names from `capo_state`.
- Do not change SQLite schema, projection encoding, projection decoding, query behavior, rebuild behavior, or persisted projection kind strings.
- Run focused `capo-state` tests and the standard workspace gate before completion.

Evidence:

- `../../crates/capo-state/src/projections.rs` now owns `ProjectionRecord` and the projection/read-model structs.
- `../../crates/capo-state/src/lib.rs` re-exports projection types at the crate root.
- Resulting state crate file sizes: `../../crates/capo-state/src/lib.rs` 5,379 lines; `../../crates/capo-state/src/projections.rs` 503 lines; `../../crates/capo-state/src/event.rs` 207 lines; `../../crates/capo-state/src/error.rs` 37 lines; `../../crates/capo-state/src/tests.rs` 1,876 lines.
- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

Decision:

- Keep projection data structures separate from projection persistence code. This makes the public state read-model vocabulary easier to inspect while leaving the higher-risk SQL, projection log codec, and rebuild code in place for a later dedicated split.
- Preserve `MemoryRecordProjection::is_packet_eligible` with the type definition because it is a read-model invariant used by consumers, not a SQLite codec concern.

### SS2c - State Schema Module Split

Status: completed

Acceptance:

- Move SQLite migration and projection-table reset helpers out of `src/lib.rs`.
- Preserve SQLite schema DDL, compatibility column backfills, projection reset table list, and rebuild semantics.
- Do not change projection row encoding/decoding, read queries, event append behavior, or crate-root public APIs.
- Run focused `capo-state` tests and the standard workspace gate before completion.

Evidence:

- `../../crates/capo-state/src/schema.rs` now owns `migrate`, compatibility `add_missing_column`, and `clear_projection_tables`.
- `../../crates/capo-state/src/lib.rs` imports schema helpers privately and keeps the public API unchanged.
- Resulting state crate file sizes: `../../crates/capo-state/src/lib.rs` 4,848 lines; `../../crates/capo-state/src/schema.rs` 537 lines; `../../crates/capo-state/src/projections.rs` 503 lines; `../../crates/capo-state/src/event.rs` 207 lines; `../../crates/capo-state/src/error.rs` 37 lines; `../../crates/capo-state/src/tests.rs` 1,876 lines.
- `git diff --check`: passed.
- `cargo fmt --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

Decision:

- Keep schema and projection-table reset together for now because both define the physical SQLite table set. This makes later projection codec extraction safer by leaving the table lifecycle in one place.
- Leave `update_watermark`, projection row encoding/decoding, and `apply_projection_record` in `lib.rs` for this slice. Those functions are part of the projection runtime path and should move together in a dedicated codec/apply split with rebuild-focused regression evidence.

### SS2d - State Projection Codec Module Split

Status: completed

Acceptance:

- Move projection-log row encoding and decoding out of `src/lib.rs`.
- Preserve persisted projection kind strings, `a` through `h` column mapping, `payload_json` contents, decode errors, and rebuild behavior.
- Keep projection apply SQL, state queries, event append behavior, and crate-root public APIs unchanged.
- Run focused `capo-state` tests and the standard workspace gate before completion.

Evidence:

- `../../crates/capo-state/src/codec.rs` now owns `ProjectionRecordRow`, projection row encoding, projection row decoding, decode errors, and projection JSON field validation.
- `../../crates/capo-state/src/lib.rs` imports codec helpers privately and still owns event append, projection insertion, projection apply SQL, queries, and recovery.
- Shared `optional_id` and `escape_json` helpers remain in `../../crates/capo-state/src/lib.rs` because both query/recovery code and the codec use them.
- Resulting state crate file sizes: `../../crates/capo-state/src/lib.rs` 2,801 lines; `../../crates/capo-state/src/codec.rs` 2,067 lines; `../../crates/capo-state/src/schema.rs` 537 lines; `../../crates/capo-state/src/projections.rs` 503 lines; `../../crates/capo-state/src/event.rs` 207 lines; `../../crates/capo-state/src/error.rs` 37 lines; `../../crates/capo-state/src/tests.rs` 1,876 lines.
- `git diff --check`: passed.
- `cargo test -p capo-state`: passed.
- `cargo fmt`: applied.
- `cargo fmt --check`: passed.
- `cargo test --workspace --all-targets`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

Decision:

- Move encode/decode together because those functions jointly define the durable projection-log compatibility contract.
- Keep `apply_projection_record` in `lib.rs` for now. It is SQL write behavior, not row codec behavior, and should move in a separate projection-apply slice with rebuild-focused evidence.
