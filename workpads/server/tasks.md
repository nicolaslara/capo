# Server Workpad Tasks

## Objective

Make Capo run as a server/control plane that tracks agents and owns orchestration state, with CLI/client surfaces interacting with agents through that server boundary.

The first proof uses deterministic mocked agents. Once the boundary is stable, Codex must run through the same server path so Capo can dogfood its own agent orchestration instead of calling adapters directly from one-off CLI commands.

## SV0 - Server Boundary And Mock-Agent Tracking

Status: completed on 2026-05-27

Acceptance:

- Add a server/control-plane crate or module that owns controller, state, and query access behind typed request/response boundaries.
- Support at least registering an agent, sending a task to a named mocked agent, listing tracked agents, querying a dashboard snapshot, and recovering state.
- Keep the implementation modular so transport, runtime, protocol adapters, memory, tools, and input surfaces can be swapped later.
- Add deterministic tests proving a client interacts through the server boundary, the mocked agent is tracked, session/task/tool/memory state appears in the dashboard, and recovery does not lose state.
- Record why this slice is not yet the final network daemon or full CLI transport.

Evidence:

- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/tests.rs`
- `Cargo.toml`
- `workpads/server/knowledge.md`
- `cargo test -p capo-server`

Result:

- Added `capo-server` as the typed server/control-plane boundary.
- Added `CapoServer::handle(ServerRequest)` with register-agent, send-task, list-agents, dashboard, and recover commands.
- Server responses are typed summaries, not CLI text.
- Added deterministic mocked-agent coverage proving client-through-server agent tracking, task/session/run refs, tool call and memory packet counts, recovery, and reopen from persisted state.
- Deferred daemon transport and CLI rerouting to SV1/SV2 so transport choices do not define the server contract.
- Review fixes removed the public raw state-store accessor and updated `/next`/`$next` routing docs so server work loads architecture context.

Review follow-ups:

- Propagate stable server request IDs, idempotency keys, client IDs, actor IDs, and origin types through mutating server commands before the CLI-through-server path becomes default.
- Add boundary-hardening tests for unknown agents, multiple agents, repeated sends to the same mocked agent, and request-origin preservation.
- Replace or wrap `FakeBoundaryController` behind a production-facing controller facade before Codex runs through the server boundary.

## SV1 - CLI Client Through Server Boundary

Status: completed on 2026-05-27

Acceptance:

- Route a minimal local CLI surface through the server boundary instead of directly owning controller calls.
- Carry stable request identity, actor/client origin, and idempotency through server mutating commands.
- Cover agent registration, task send, agent list, dashboard/status, and recovery.
- Preserve compatibility for existing direct commands until the server-backed path is proven.
- Add tests that fail if the CLI bypasses the server boundary for the new server-backed commands, including unknown-agent and multi-agent coverage.

Evidence:

- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/src/tests.rs`
- `cargo test -p capo-server`
- `cargo test -p capo-cli server_cli_routes_agent_work_through_server_boundary -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Progress:

- Added `capo server agent register|list|status`, `capo server task send`, `capo server dashboard`, and `capo server recover` as server-backed CLI client commands.
- Server responses now carry request ID, client ID, actor ID, and input origin metadata.
- Mutating server requests now emit `server.request_handled` audit events with request/client/actor/origin/idempotency metadata.
- Added focused tests for request-origin propagation, unknown-agent rejection, audit-event correlation, multi-agent CLI flow, dashboard/status, and recovery through the server boundary.
- Review fixes reject repeated sends while the mocked controller still uses fixed session/run IDs, map unknown task sends to `UnknownAgent`, JSON-encode audit payloads, and assert CLI-persisted server audit events.

Result:

- SV1 keeps the existing direct CLI commands as compatibility paths and adds a server-backed `capo server ...` namespace for the product path.
- The server-backed CLI covers register, list, status, task send, dashboard, and recovery.
- Request identity and origin metadata are visible in CLI output and persisted as auditable state events for mutating server requests.
- Repeated task sends are temporarily rejected for agents with existing sessions until the controller facade can create request/task-aware session/run identities.

## SV2 - Runnable Local Server Transport

Status: pending

Acceptance:

- Add a runnable local server process or daemon mode with an explicit local transport.
- Keep transport serialization separate from server command semantics.
- Add a client command that connects to a running server and performs the SV1 flow.
- Include restart/recovery coverage.

## SV3 - Codex Agent Through Server

Status: pending

Acceptance:

- Execute or replay the Codex-backed connector through the same server boundary used by mocked agents.
- Record provider/subscription handling assumptions and avoid logging raw secrets or session credentials.
- Verify dashboard/status/recovery evidence for the Codex path.

## SV4 - Review Gate And Next Product Slice

Status: pending

Acceptance:

- Run xhigh review on the server implementation and evidence.
- Fix required issues or add explicit follow-up tasks.
- Decide the next product slice: richer CLI loop, ACP-first server session model, network transport hardening, or dashboard.
