# Server Workpad Knowledge

## Objective

Record decisions and evidence while making Capo's server/control-plane model real.

## Initial Direction

Status: started on 2026-05-27.

Decisions:

- Start with a typed local server boundary over the existing controller/state/query stack before adding daemon transport. This proves ownership boundaries and deterministic state behavior without coupling the first slice to socket protocol decisions.
- The CLI should become a client of that boundary. Existing direct CLI commands can stay as compatibility paths while new server-backed commands are introduced.
- Mocked-agent coverage is the regression baseline. Codex support should reuse the same server boundary once the client path is stable.
- The server boundary must not absorb tunnel/connectivity, provider execution, memory backend internals, or input modality logic. Those remain separate modules behind contracts.

Open questions:

- Which transport should be first for the runnable daemon: local TCP, Unix domain socket, JSON-RPC, or an ACP-compatible control channel?
- Should the CLI default to an embedded local server when no daemon is running, or require explicit `capo server serve` for early dogfooding?
- Where should long-running subscription-backed agent process supervision live relative to runtime targets?

## SV0 - Server Boundary And Mock-Agent Tracking

Status: completed on 2026-05-27.

Implementation:

- Added `crates/capo-server` as a workspace crate.
- Added `CapoServer`, `ServerRequest`, `ServerCommand`, and typed response summaries over the existing controller/state/query stack.
- The server boundary currently supports agent registration, task send to a named mocked agent, agent list, dashboard snapshot, and recovery.
- The dashboard snapshot intentionally summarizes query read models instead of returning CLI-rendered text. This keeps CLI and future clients downstream of the server contract.
- Active-session counting is based on run status `running`, not only session status. Recovery currently marks active runs `exited_unknown` while the session read model can remain `active`; the server snapshot treats that as not actively executing.
- Review feedback accepted: remove the public raw state-store accessor from `capo-server`, and update `$next`/`/next` command routing so server work loads architecture artifacts and scaffold knowledge.

Verification:

- `cargo fmt`
- `cargo test -p capo-server`

Deferred:

- Runnable daemon/socket transport.
- CLI command routing through `capo-server`.
- Codex connector proof through the server boundary.
- Renaming `FakeBoundaryController` to a production-facing controller facade.
- Request identity/origin propagation through mutating commands.
- Boundary-hardening tests for unknown agents, multiple agents, repeated sends, and origin preservation.
