# Capo

Capo is an early-stage controller and harness for orchestrating coding LLM agents.

The current repo is a planning/workpad scaffold plus the first Rust prototype workspace. Start with:

- [`project.md`](./project.md) for product direction
- [`TASKS.md`](./TASKS.md) for active phase
- [`WORKING.md`](./WORKING.md) for the agent workflow
- [`workpads/WORKPADS.md`](./workpads/WORKPADS.md) for per-workpad load lists

The active target is the minimal end-to-end prototype that can spawn, track, and interact with at least one coding agent.

## Prototype Commands

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -p capo-cli -- --help
```
