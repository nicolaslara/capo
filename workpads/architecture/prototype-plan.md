# Capo Prototype Plan

## Objective

Translate the architecture artifacts into the first implementation sequence for Capo: a Rust-first local controller that proves the boundaries with fake implementations, then with local Codex and Claude Code adapter paths, before any dashboard, voice, remote runtime, or dogfood migration claims are made.

The prototype should prove that Capo can receive human intent, create controller-owned state, dispatch work to an agent boundary, expose instrumented tools, inspect progress, interrupt execution, persist state, recover after restart, and write human-auditable evidence.

## Prototype Shape

Build a local-first Rust scaffold with static dispatch over known in-tree implementations:

| Boundary | First implementation | Next implementation | Deferred |
| --- | --- | --- | --- |
| Input surface | CLI commands | Local TUI or web dashboard | Mobile and production voice |
| Controller | Rust command/session manager | Scheduler for multiple agents | Distributed controller |
| Agent adapter | `FakeAdapter` | `CodexExecAdapter`, `ClaudeCodeAdapter`, ACP fixture adapter | Capo as an ACP agent/editor backend |
| Runtime runner | `FakeRuntimeRunner` | `LocalProcessRunner` over pipes | SSH, containers, cloud VMs |
| Connectivity/tunnel | `FakeTunnel`, `LocalLoopbackTunnel` | Local HTTP/control endpoint | Tailscale/Funnel/reverse tunnel exposure |
| Provider connector | `FakeProviderConnector` | Local subscription connector metadata for Codex/Claude | Hosted subscription brokering |
| Permission policy | `AllowTrustedLocalProfilePolicy` | Static and user-approval policy | Security-agent policy |
| Tool exposure | Fake and Capo-owned tools | Runtime/file/git wrappers | MCP publication, native-provider enforcement |
| State store | In-memory for unit tests | SQLite event log/projections plus artifact files | Postgres/server deployment |
| Memory | Fake packet builder | Markdown/SQLite source-linked packets | External graph/vector memory |
| Evaluation | Fake/local completion report | Outcome and review read models | Full scoring framework |

The scaffold should use Rust for the controller, state model, runtime supervision, adapter normalization, permission routing, and tool instrumentation. Python remains allowed later for voice, local-model, memory-system, or research sidecars, but it should not be required for the first e2e controller.

## Package Layout

Start with one Cargo workspace. Keep modules separate even if implementations are thin:

```text
crates/
  capo-cli/          # command-line control surface
  capo-core/         # domain types, controller, command envelopes
  capo-state/        # event store, SQLite projections, artifacts
  capo-adapters/     # fake, Codex, Claude Code, ACP mapping
  capo-runtime/      # fake and local process runners
  capo-tools/        # tool registry, Capo-owned tools, wrappers
  capo-memory/       # fake and markdown/SQLite memory packets
  capo-eval/         # completion/evidence reports
tests/
  e2e/               # smoke tests that exercise CLI/controller/state
```

Keep static dispatch readable by making each boundary enum small and local to its owning crate. Avoid a shared "everything adapter" enum that erases the architecture boundaries.

## Ordered Prototype Tasks

1. Scaffold the Cargo workspace and command surface.
   - Create the workspace, crates, common error/result conventions, format/lint/test commands, and a `capo --help` skeleton.
   - No real adapter work until the module boundaries compile independently.

2. Implement core domain types and boundary traits/enums.
   - Add typed IDs, command envelopes, lifecycle status vocabulary, agent/session/run/turn/tool/memory/evidence records, and static dispatch wrappers for fake variants.
   - The goal is compile-time clarity before persistence.

3. Implement SQLite event store, projections, and artifact layout.
   - Append controller-owned events, rebuild read models, and store large payloads as artifacts.
   - Include idempotency keys, raw adapter-event references, and restart recovery records from the architecture.

4. Build a fake e2e agent loop.
   - Drive `FakeAdapter`, `FakeRuntimeRunner`, `FakeProviderConnector`, fake permission policy, fake memory packet, and fake tool handlers through the real controller and state store.
   - This is the first smoke test because it proves Capo's boundaries without relying on provider CLIs.

5. Add the first CLI control surface.
   - Support starting local state, registering/spawning an agent, sending a task, listing agents/sessions, showing status/recent events/latest summary, interrupting/stopping, and exporting evidence.
   - CLI output should be intentionally simple and backed by read models.

6. Add local process runtime.
   - Implement process launch, stdin/stdout capture, interrupt, terminate, kill, health, cleanup, redaction hooks, and orphan recovery.
   - Keep runtime and connectivity separated; local loopback remains only endpoint resolution.

7. Add Codex and Claude Code fixture adapters.
   - Capture or hand-author non-secret golden streams for `codex exec --json` and `claude -p --output-format stream-json`.
   - Parse into normalized adapter events and prove duplicate/replay handling where fields exist.

8. Add first real local adapter smoke.
   - Prefer Codex first if it can run a harmless fixture task without leaking secrets; add Claude Code next when its non-interactive path is equally safe.
   - Keep subscription connectors local-only and do not inspect vendor credential storage.
   - Use restrictive launch defaults: isolated temporary workspace, no MCP configs, no browser tools, no provider-native write/network tools unless explicitly scoped, Codex read-only sandbox by default, Claude restricted `--allowedTools` / permission mode, and no workspace-write smoke until fixture/redaction proof exists.

9. Add Capo tool exposure and instrumentation loop.
   - Implement `capo.task_status`, `capo.agent_status`, `capo.session_summary`, `capo.workpad_read`, `capo.evidence_record`, and `capo.capability_request`.
   - Prove tool call request, permission decision, grant use, invocation, result artifact, and adapter result-delivery events.

10. Add memory packet and context provenance.
    - Build a source-linked packet from markdown/workpad pointers and local events.
    - Make packet artifacts replayable and visible in session/read-model inspection.

11. Add restart recovery and replay tests.
    - Restart Capo against an existing SQLite store, rebuild projections, recover or mark active runs, and avoid duplicate UI/read-model rows.
    - Include ACP fixture replay tests before implementing broader ACP support.

12. Add workpad evidence export.
    - Write workpad-like markdown evidence for a completed or interrupted run.
    - Preserve markdown as the human-auditable fallback and avoid corrupting existing project workpads.

13. Run prototype e2e gate review.
    - Execute the smoke path below, record evidence, list dogfood gaps, and decide whether the architecture gate can move to prototype or needs another architecture pass.

## E2E Smoke Test

The first full smoke should be runnable from a clean checkout after dependencies are installed:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -p capo-cli -- init --state .capo-dev
cargo run -p capo-cli -- agent register --name fake-codex --adapter fake --runtime fake
cargo run -p capo-cli -- task send --agent fake-codex --goal "Inspect the project and write a short status summary" --scenario tool-memory
cargo run -p capo-cli -- agent register --name fake-reviewer --adapter fake --runtime fake
cargo run -p capo-cli -- task send --agent fake-reviewer --goal "Review the status summary for blockers" --scenario summary-review
cargo run -p capo-cli -- session status --agent fake-codex
cargo run -p capo-cli -- session redirect --agent fake-reviewer --goal "Focus only on dogfood blockers"
cargo run -p capo-cli -- session interrupt --agent fake-codex --reason "smoke interrupt"
cargo run -p capo-cli -- recover --state .capo-dev
cargo run -p capo-cli -- evidence export --session <session-id> --out workpads/prototype/evidence/
```

The smoke passes only when it proves all of these:

- A controller-owned task/session/run exists in SQLite.
- The fake agent receives a command through the adapter boundary.
- Two fake agents/sessions can be tracked at the same time.
- Recent events, current goal, status, latest summary, blocker, confidence, tool observations, and evidence refs come from read models.
- Redirect/steer and interrupt/stop emit durable events and change the read model.
- Restart recovery rebuilds projections without duplicate session/update rows.
- The `tool-memory` fake scenario forces at least one Capo tool request, permission decision, grant use, adapter result delivery, and tool output artifact.
- Memory context is attached as a replayable packet artifact with source refs.
- Evidence export writes markdown without modifying unrelated workpads.
- Logs and artifacts contain no provider credentials, subscription tokens, cookies, or raw sensitive transcripts.
- Persistent raw provider/runtime artifacts are classified as `safe` or `redacted`; unclassified or sensitive raw streams are rejected or quarantined outside read models/evidence.

Real Codex/Claude smoke tests should be separate opt-in tests until fixture parsing and secret redaction are proven.

## Dogfood Gate Prerequisites

Capo can begin managing its own project only after evidence shows:

- Multiple work items and agent sessions can be tracked concurrently.
- State survives restart and recovery without duplicate read-model/UI state.
- Each session exposes active goal, status, blocker, confidence, recent events, latest summary, capabilities, memory packet refs, and evidence refs.
- The user can interrupt or redirect a running agent reliably.
- Workpad evidence export is deterministic and markdown remains a trustworthy fallback.
- Capo can read project workpads and write evidence/update artifacts without corrupting user-authored files.
- Capability decisions are routed through `PermissionPolicy` and audited, even if the policy allows everything in local dogfood mode.
- Subscription-backed Codex and Claude Code paths run only as user-local connectors and do not read or persist credential material.
- A human review records what the prototype proves, what remains stubbed, and which gaps block dogfooding.

## Explicit Deferrals

- Production voice control and mobile UI are deferred until the CLI/read-model/control protocol is stable. Voice remains architecturally first-class because it will converse with Capo over the same command envelopes and read models.
- Remote runtimes, Tailscale/Funnel exposure, cloud execution, and container sandboxing are deferred until local runtime and recovery semantics are proven.
- Capo as an ACP agent/editor backend is out of prototype scope. Capo remains the entrypoint/controller.
- Full ACP implementation is deferred behind fixture replay and adapter mapping tests. ACP compatibility should improve adapter design without replacing Capo's domain model.
- External memory systems, embeddings, graph memory, and generated-memory review UX are deferred until dogfood traces show retrieval needs.

## Architecture Gate Evidence

This plan satisfies A7 when:

- `workpads/prototype/tasks.md` has ordered tasks derived from the architecture boundaries.
- This file defines the e2e smoke test and dogfood prerequisites.
- Routing docs load this file before prototype work.
- Review records any plan gaps in `knowledge.md`.
