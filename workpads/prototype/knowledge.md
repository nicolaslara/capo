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
