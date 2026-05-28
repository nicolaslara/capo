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

Status: completed on 2026-05-27

Acceptance:

- Add a runnable local server process or daemon mode with an explicit local transport.
- Keep transport serialization separate from server command semantics.
- Add a client command that connects to a running server and performs the SV1 flow.
- Include restart/recovery coverage.

Evidence:

- `crates/capo-server/src/transport.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/tests.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Progress:

- Added a foreground loopback TCP transport with one newline-delimited JSON request/response per connection.
- Added `capo server serve --addr ADDR [--max-requests N]`.
- Added `--connect ADDR` to the server-backed CLI commands from SV1.
- Added a process-level integration test that spawns a running `capo server serve` process, then drives register, task send, status, dashboard, and recovery from separate CLI processes over the transport.
- Chose loopback TCP over Unix sockets for SV2 because it is still local by default while matching the future remote/tunnel control path. Unix-domain sockets remain a good local-only hardening follow-up.
- Review fixes enforce loopback-only bind addresses, make `--addr` and `--connect` fail closed when present without values, and prove true post-restart recovery by restarting the server before calling recover.

Result:

- Capo can now run as a foreground local server process.
- CLI commands can connect to the running process over loopback transport using `--connect ADDR`.
- The transport path preserves the typed `ServerRequest`/`ServerResponse` boundary and keeps serialization in `capo-server/src/transport.rs`.
- Restart recovery is tested by starting a second server process against the same state root, running `server recover --connect`, and confirming the recovered run state.

## SV3 - Codex Agent Through Server

Status: completed on 2026-05-27

Acceptance:

- Execute or replay the Codex-backed connector through the same server boundary used by mocked agents.
- Record provider/subscription handling assumptions and avoid logging raw secrets or session credentials.
- Verify dashboard/status/recovery evidence for the Codex path.

Progress:

- Added a server-owned adapter fixture replay command for Codex JSONL. This is a replay proof, not live subscription execution.
- The CLI reads the fixture and submits it through `capo server adapter replay-fixture`; the running server parses normalized Codex events and applies them to Capo state.
- The replay response records `provider_cli_executed=false` and `raw_content_policy=content_hashed_not_rendered`.
- Added process-level transport coverage that starts `capo server serve`, registers the Codex-named agent through `--connect`, replays the Codex fixture through `--connect`, checks dashboard state, restarts the server, and verifies recovery.
- Review fix: replay safety metadata (`provider_cli_executed=false`, `raw_content_policy=content_hashed_not_rendered`, fixture hash, and raw body non-persistence) is now persisted in the server request audit event and checked after restart.

Evidence:

- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Result:

- Capo can now prove Codex-shaped adapter activity through the same running server boundary used by mocked agents.
- The SV3 proof uses deterministic Codex JSONL replay, not live provider execution.
- The running server path covers explicit agent registration, Codex fixture replay, dashboard/status visibility, server restart, recovery, and durable safety audit metadata.
- The replay path preserves subscription safety by not launching Codex, not inspecting credentials, and not persisting raw fixture text.

Review follow-ups:

- Deprecate or clearly mark the old direct `capo adapter replay-fixture` compatibility command, because it still bypasses the running server.
- Add fixture-size caps and explicit raw fixture wire/in-memory policy for local-loopback replay requests.
- Replace the fake-session bootstrap used by replay with a server-native adapter session start before live Codex execution.

## SV4 - Review Gate And Next Product Slice

Status: completed on 2026-05-27

Acceptance:

- Run xhigh review on the server implementation and evidence.
- Fix required issues or add explicit follow-up tasks.
- Decide the next product slice: richer CLI loop, ACP-first server session model, network transport hardening, or dashboard.

Progress:

- Broad xhigh review found two must-fix issues before closing the server gate:
  - server request idempotency could hide real mutations from the audit trail when the same request ID was reused for different commands;
  - `--connect` could send requests, including raw fixture replay bodies, to non-loopback addresses.
- Accepted fixes:
  - server audit idempotency now includes a command identity hash in addition to client, actor, and request ID;
  - server client `--connect` now enforces loopback resolution before sending a request;
  - focused regression tests cover both cases.

Evidence:

- xhigh server review: must-fix audit idempotency and client loopback enforcement were accepted and fixed.
- xhigh next-slice planning: recommended SV5 as an ACP-first server session model before live Codex dispatch, richer CLI loop, network hardening, or dashboard polish.
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Result:

- SV0-SV3 are good enough as the server foundation after the two SV4 must-fixes.
- The next product slice should be SV5: server-native sessions and turns with ACP-shaped adapter ingress.

## SV5 - ACP-First Server Session Model

Status: completed on 2026-05-27

Acceptance:

- Add server-native session/turn commands keyed by agent, session ID, run ID, and turn ID.
- Remove the fake-session bootstrap from adapter fixture replay; replay should target an existing server session/run.
- Allow multiple historical sessions per agent, and either support or explicitly reject concurrent active sessions with a real policy reason rather than fixed-ID collision.
- Ingest ACP-shaped mock events and Codex fixture events through the same server session ingress path.
- Persist adapter/session metadata needed for replay/recovery: adapter kind, external session ref, raw content policy, fixture hash, and provider execution status.
- Add raw fixture size caps and keep the local-loopback raw body policy explicit.
- Verify dashboard/status after restart shows session/run/turn state, adapter kind, tool counts, memory/evidence refs, and recovery state.
- Mark direct `capo adapter replay-fixture` as compatibility/bypass while the server path becomes the product path.

Progress:

- Added `capo server session start` as the server-native adapter session entrypoint.
- Changed `capo server adapter replay-fixture` to target explicit `--session SESSION_ID --run RUN_ID --turn TURN_ID` instead of creating a fake session from `--agent` and `--goal`.
- Added server-side, CLI-side, and transport-frame fixture size caps.
- Persisted replay metadata in `server.request_handled`: adapter kind, fixture hash, provider execution status, raw content policy, raw body non-persistence, target session/run/turn, and local-loopback transport scope.
- Added Codex and ACP-shaped fixture replay coverage through the same server session ingress.
- Added coverage for multiple historical sessions per agent after recovery, explicit concurrent active-session rejection, duplicate run-ID rejection, unknown-session replay rejection, and oversized fixture rejection.
- Dashboard/status now expose adapter kind, evidence refs/count, turn ids/count, tool counts, memory counts, and recovered run status for the current session.
- Marked direct `capo adapter replay-fixture` as a compatibility bypass in CLI help.

Evidence:

- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo test -p capo-server server_replays_codex_fixture_through_server_boundary -- --nocapture`
- `cargo test -p capo-server server_replays_acp_fixture_into_server_native_session -- --nocapture`
- `cargo test -p capo-server server_native_sessions_allow_multiple_historical_sessions_per_agent -- --nocapture`
- `cargo test -p capo-server server_rejects_adapter_fixture -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_oversized_frames_before_json_decode -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_replays_codex_fixture_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV5 review found four must-fixes: duplicate run IDs could corrupt historical sessions; replay was not explicitly run/turn keyed; raw fixture caps were after transport decode; dashboard/status did not expose enough SV5 metadata.
- Accepted fixes:
  - `StartSession` rejects duplicate run IDs.
  - `ReplayAdapterFixture` requires explicit session, run, and turn IDs.
  - TCP transport rejects oversized request frames before JSON decode.
  - Dashboard/status expose adapter kind, evidence refs/count, turn ids/count, tool counts, memory counts, and recovered run state.

Result:

- SV5 is complete for deterministic server-native adapter session ingress.
- Capo can register an agent, start a server-owned Codex/ACP adapter session, replay provider-shaped events into an explicit session/run/turn, inspect state through the running server, restart/recover, and retain auditable metadata without persisting raw provider content.
- Concurrent active sessions per agent remain intentionally rejected until the dashboard/control model can represent more than one active session for the same agent.

Follow-ups discovered:

- Live Codex subscription execution through the running server with explicit opt-in and artifact scanning.
- A server-native dispatch plan/gate/run-local workflow so live provider execution is not coupled to CLI-only helper modules.
- Raw adapter event artifact retention with redaction metadata, if needed for deeper replay/debug.

## SV6 - Server-Native Dispatch Plan/Gate/Run-Local

Status: completed on 2026-05-27

Acceptance:

- Add typed server commands/responses for dispatch plan, dispatch gate/preflight, and run-local execution, exposed through `capo server dispatch ...` for embedded and `--connect` paths.
- Make dispatch planning server-owned: validate agent/session/run/turn target, adapter kind, runtime target, capability profile, prompt source/materialization hash, artifact root, timeout/output limits, and opt-in environment.
- Gate/preflight must fail closed with structured reason codes for missing opt-in, stale prompt hash, missing smoke evidence, adapter/session mismatch, unknown agent/session, active-session conflict, unsafe runtime/network scope, or missing artifact-scan policy.
- Run-local must go through server -> controller -> runtime runner -> adapter parser -> server-native session ingress. Automated tests should use mocked/deterministic runtime output, not live Codex.
- Add a process-level test that starts `capo server serve`, then drives register -> dispatch plan -> gate/preflight -> deterministic run-local -> dashboard/status -> restart -> recover from separate CLI processes.
- Dashboard/status should show dispatch plan/gate/execution IDs, runtime process ref, artifact refs, credential scan status, `provider_cli_executed`, raw prompt/output policies, stream ingest counts, turn IDs, tool counts, memory counts, evidence refs, and recovered run state.
- Idempotency tests must prove repeated execution requests do not spawn duplicate processes or duplicate projected stream state.
- Verify with `cargo test -p capo-server`, server transport integration tests, `cargo clippy --all-targets --all-features -- -D warnings`, full `cargo test`, and `git diff --check`.

Must not do:

- Do not make live Codex execution default or required for CI.
- Do not read OAuth tokens, cookies, vendor session stores, keychains, or raw credential files.
- Do not persist or render raw prompts or raw provider output; store hashes, artifact refs, redaction metadata, and scan results.
- Do not let direct `capo adapter ...` CLI helpers mutate server state for the server path.
- Do not add public/non-loopback listeners, tunnels, remote clients, or auth-token exposure in this slice.
- Do not add concurrent active sessions per agent unless the dashboard/control model is upgraded in the same slice.

Progress:

- Added typed server commands/responses for dispatch planning, dispatch gate/preflight, and deterministic run-local execution.
- Exposed the server-owned dispatch path through `capo server dispatch plan`, `capo server dispatch gate`, and `capo server dispatch run-local`, including `--connect` transport support.
- Planning validates the target agent/session/run and records adapter kind, deterministic runtime metadata, prompt source hash, artifact root, and raw prompt policy.
- Gate/preflight records dispatch gate, prompt materialization, and execution-request projections with structured reason codes.
- Run-local ingests deterministic Codex-shaped fixture output through adapter parsing and server-native session/run/turn ingress, then records execution and replay projections.
- Dashboard/status render dispatch plan/gate/execution IDs, execution status, runtime process ref, credential scan status, raw prompt/output policies, turn IDs, tool counts, memory counts, evidence refs, and recovered run state.
- Added server-level idempotency coverage proving repeated run-local requests do not duplicate projected stream state.
- Added process-level running-server coverage for register -> session start -> dispatch plan -> gate -> run-local -> dashboard -> restart -> recover -> status.
- Review fixes made dispatch gate fail closed without deterministic opt-in, changed deterministic fixture execution metadata to avoid claiming live provider execution or clean credential scans, rejected changed-fixture reruns, marked successful fixture dispatch runs `exited`, and exposed provider execution booleans in dashboard/status.

Evidence:

- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo test -p capo-server server_dispatch_plan_gate_and_run_local_ingest_codex_fixture_idempotently -- --nocapture`
- `cargo test -p capo-server server_dispatch_gate_blocks_without_deterministic_opt_in -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_dispatches_codex_fixture_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV6 review found five must-fix issues. Accepted and fixed: fail-closed gate, truthful deterministic fixture execution metadata, changed-fixture idempotency, terminal run projection, and dashboard/status provider execution fields.
- Review follow-ups retained: split `crates/capo-server/src/lib.rs` by responsibility before live-provider dispatch; make normalized adapter replay idempotency session/run-scoped before broad multi-session replay.

Result:

- SV6 is complete for deterministic server-native dispatch.
- Capo can run a local server, register an agent, start a server-owned session, plan and gate deterministic dispatch, ingest Codex-shaped fixture output through server-native run-local, show dispatch state through dashboard/status, restart/recover, and preserve truthful provider/credential metadata.

## SV7 - Server Crate Responsibility Split

Status: completed on 2026-05-27

Objective:

Split the oversized server implementation into LLM-friendly modules without changing behavior, so future live-provider dispatch work can be done safely.

Acceptance:

- Split `crates/capo-server/src/lib.rs` by responsibility instead of arbitrary chunks. Likely modules: request/response types, dashboard summaries, adapter fixture replay, dispatch plan/gate/run-local, transport-facing helpers, and tests by feature.
- Keep public API compatibility for `CapoServer`, `ServerRequest`, `ServerCommand`, `ServerResponse`, transport exports, and summaries.
- Preserve server/transport behavior and all existing tests.
- Update references/knowledge with the module map and any invariants discovered during the split.
- Verify with `cargo fmt`, `cargo test -p capo-server`, `cargo test -p capo-cli --test server_transport -- --nocapture`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and `git diff --check`.

Progress:

- Split the server crate by responsibility while preserving the existing public API exports.
- Moved request/response types and summaries into `types.rs`.
- Moved dashboard/status summary projection into `dashboard.rs`.
- Moved dispatch plan/gate/run-local helpers and projections into `dispatch.rs`.
- Moved shared command/audit helpers into `server_core.rs`.
- Moved adapter/event utility helpers into `util.rs`.
- Moved transport wire conversion helpers into `transport/wire.rs`.
- Kept the `CapoServer::handle` command router in `lib.rs` so request semantics remain locally understandable.
- Kept behavior-compatible exports for `CapoServer`, `ServerRequest`, `ServerCommand`, `ServerResponse`, transport exports, and summary types.

Evidence:

- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/types.rs`
- `crates/capo-server/src/dashboard.rs`
- `crates/capo-server/src/dispatch.rs`
- `crates/capo-server/src/server_core.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-server/src/transport/wire.rs`
- `crates/capo-server/src/util.rs`
- `wc -l crates/capo-server/src/lib.rs crates/capo-server/src/dispatch.rs crates/capo-server/src/types.rs crates/capo-server/src/transport.rs crates/capo-server/src/transport/wire.rs crates/capo-server/src/server_core.rs crates/capo-server/src/dashboard.rs crates/capo-server/src/util.rs crates/capo-server/src/tests.rs`
- `cargo fmt`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Initial xhigh planning and review subagents were attempted for SV7 but did not return findings before timing out and being closed.
- A fresh xhigh review in SV8 found two must-fix foundation issues before live-provider execution: `serve_tcp` needed loopback enforcement at the public server-transport boundary, and replay/dispatch needed to reject adapter/session mismatches.
- Accepted and fixed both foundation issues in the SV8 pass.
- The review also confirmed that live-provider preflight is not implemented yet. Keep that as the next product slice instead of extending the deterministic fixture gate by accident.

Result:

- `capo-server` is now below the Rust module hard warning target for production modules: `lib.rs` 704 LOC, `dispatch.rs` 594 LOC, `types.rs` 454 LOC, `transport.rs` 696 LOC, `transport/wire.rs` 154 LOC, `server_core.rs` 154 LOC, `dashboard.rs` 119 LOC, and `util.rs` 105 LOC.
- `tests.rs` remains 1030 LOC, which is acceptable for the current test-file guidance but should be split when it becomes harder to navigate.
- The split makes the next server work easier to review without changing the server boundary or transport behavior.

## SV8 - Server Split Review Gate And Next Product Slice

Status: completed on 2026-05-27

Objective:

Run a fresh xhigh review on the SV7 split and the current server path, fix any must-fix findings, then choose the next product slice toward live Codex/Claude-through-server orchestration.

Acceptance:

- Run xhigh review on the current `capo-server` split, server transport path, CLI-through-server path, deterministic mocked-agent path, and Codex fixture/dispatch path.
- Classify findings as must-fix now, follow-up task, or rejected with reason.
- Fix must-fix findings before starting live-provider execution work.
- Run xhigh planning for the next product slice after review. Candidate slices include live Codex dispatch through the server, Claude Code support through the same boundary, session/run-scoped adapter replay idempotency, or mocked-agent test harness hardening.
- Update `tasks.md`, `knowledge.md`, and `references.md` with the review result, next-slice decision, and validation evidence.
- Verify with at least `cargo test -p capo-server`, `cargo test -p capo-cli --test server_transport -- --nocapture`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and `git diff --check` after any code changes.

Progress:

- Ran xhigh review on the SV7 split and current server path.
- Fixed the two server-foundation must-fixes from review:
  - `capo-server::serve_tcp` now rejects non-loopback listeners at the public transport boundary.
  - Server replay and dispatch planning now reject adapter/session mismatches before parsing or mutating adapter events.
- Added focused server tests for non-loopback listener rejection and adapter/session mismatch rejection.
- Kept the live-provider preflight finding as the next product slice instead of pretending deterministic dispatch is live-provider-ready.

Evidence:

- `crates/capo-server/src/transport.rs`
- `crates/capo-server/src/server_core.rs`
- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/types.rs`
- `crates/capo-server/src/transport/wire.rs`
- `crates/capo-server/src/tests.rs`
- `cargo fmt`
- `cargo test -p capo-server server_rejects_adapter_replay_and_dispatch_that_mismatch_session_adapter -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_non_loopback_listener_at_server_boundary -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Planning:

- Xhigh next-slice planning was attempted after the SV8 fixes, but the subagent stalled and was closed without returning a plan.
- Based on the completed review evidence, the next slice is SV9: live-provider preflight/authorization. This is the narrow gate needed before any live Codex or Claude Code execution through the server.

Result:

- SV8 fixed the server-foundation review blockers and left live-provider execution blocked behind an explicit preflight/authorization slice.

## SV9 - Live Provider Preflight And Authorization Gate

Status: completed on 2026-05-27

Objective:

Add a server-owned live-provider preflight/authorization path for Codex and Claude Code that proves Capo can decide whether live provider execution is allowed without launching provider CLIs, reading subscription credentials, or reusing the deterministic fixture gate.

Acceptance:

- Add typed server command/response support for live-provider preflight against an existing server agent/session/run/turn target.
- Support at least Codex and Claude Code adapter kinds through the same command shape, with adapter/session mismatch rejection preserved.
- Validate provider connector kind, runtime target scope, workspace scope, artifact root policy, capability profile, opt-in environment, credential-scan plan, raw prompt policy, raw output policy, and tool-wrapper/instrumentation policy.
- Persist a preflight/audit projection or event that dashboard/status can show without exposing raw prompts, raw provider output, secrets, session files, OAuth tokens, cookies, or keychain material.
- Gate must fail closed with structured reason codes when opt-in is missing, adapter/session mismatches, runtime/network scope is unsafe, capability profile is missing, artifact scan policy is missing, or credential handling policy is not explicit.
- CLI must expose the preflight through `capo server dispatch ...` for embedded and `--connect` paths.
- Add deterministic tests for mocked/preflight-only Codex and Claude paths through the server and running transport.
- Update dashboard/status to show latest live-provider preflight status and next action.
- Verify with `cargo fmt`, focused server tests, server transport integration tests, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and `git diff --check`.

Must not do:

- Do not execute live Codex or Claude Code provider CLIs in this slice.
- Do not inspect or persist credentials, OAuth/session stores, cookies, keychain entries, or raw provider config files.
- Do not make non-loopback transport or tunneled exposure available.
- Do not persist or render raw prompts or raw provider output.
- Do not bypass Capo tool wrappers/instrumentation for future live execution; the preflight must record whether wrapper/instrumentation policy is satisfied.

Progress:

- Added typed `PreflightLiveProvider` server command and `LiveProviderPreflightSummary` response.
- Exposed `capo server dispatch live-preflight` for embedded and `--connect` server paths.
- Supported Codex and Claude Code adapter kinds through the same command shape; ACP is rejected for live-provider preflight.
- Preserved adapter/session mismatch rejection before preflight planning.
- Persisted preflight state through dispatch plan, gate, and execution-request projections without provider CLI execution.
- Added dashboard/status fields for dispatch gate status, reasons, and next action.
- Added deterministic server tests for ready Codex/Claude preflight and fail-closed blocked preflight.
- Added a process-level running-server test for Codex and Claude live-provider preflight through CLI transport.

Evidence:

- `crates/capo-server/src/types.rs`
- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/dashboard.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo fmt`
- `cargo test -p capo-server server_live_provider_preflight -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_preflights_live_codex_and_claude_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV9 review found two must-fixes:
  - Live-provider preflight idempotency did not include all authorization inputs/outcome, so a repeated preflight with changed policy could leave stale ready state.
  - Default request IDs for live-provider preflight included a slug of the raw goal.
- Accepted and fixed both must-fixes:
  - Live-provider preflight identity now includes stored policy fields, opt-in, reason codes, and status.
  - Default live-provider preflight request IDs use target IDs plus a goal hash instead of `slug(goal)`.
- Also accepted a small transport hardening follow-up: public `send_tcp` now rejects non-loopback addresses at the server crate boundary.

Result:

- SV9 adds a server-owned live-provider preflight gate for Codex and Claude Code without executing provider CLIs.
- SV9 is review-complete for preflight-only behavior.
- Actual live provider execution remains intentionally blocked behind a later explicit execution slice.

Additional evidence:

- `cargo test -p capo-server server_live_provider_preflight_changed_policy_does_not_leave_stale_ready_gate -- --nocapture`
- `cargo test -p capo-server server_live_provider_preflight_default_request_id_does_not_slug_raw_goal -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_non_loopback_connect_at_server_boundary -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Follow-ups:

- Before actual live execution, normalize workspace/artifact paths and reject credential/config/session-store locations instead of using shallow path-string checks.
- Split large server files again before live-execution implementation if the edit surface grows: `lib.rs`, `transport.rs`, and `tests.rs` are back in the warning zone.

## SV10 - Live Provider Execution Planning Review

Status: completed on 2026-05-27

Objective:

Run xhigh planning for the first explicit live-provider execution slice after SV9, then define a narrow implementation task that can launch Codex or Claude Code only behind the reviewed preflight gate, loopback server boundary, tool-wrapper policy, artifact scanning, and explicit opt-in.

Acceptance:

- Run xhigh planning before implementation.
- Decide whether the first live execution slice should target Codex only, Claude only, or a shared live execution abstraction with one provider enabled.
- Define exact launch boundary, required opt-in env, runtime target, tool-wrapper/instrumentation policy, artifact scanner behavior, and recovery behavior.
- Confirm how Capo will avoid reading provider credential/session stores while still using subscription-backed CLIs.
- Add must-not-do boundaries for credentials, raw prompt/output persistence, non-loopback transport, and unwrapped tools.
- Update `tasks.md`, `knowledge.md`, and `references.md` with the plan.
- Do not implement provider CLI launch in SV10 unless the planning review explicitly recommends a very small same-slice implementation and the user direction permits it.

Planning result:

- Xhigh planning recommended a shared live-provider execution command shape with only Codex enabled in the first live slice.
- Codex is first because Capo already has Codex fixture replay, Codex JSONL parsing, and restrictive `CodexExecAdapter::local_launch_plan` support.
- Claude Code stays blocked in the first live-run slice with a structured `provider_not_enabled_for_first_live_slice` reason until a focused Claude safety review.
- The first implementation slice is SV11: server-owned `capo server dispatch live-run-local`.
- SV11 must require the latest ready SV9 live preflight, explicit `CAPO_SERVER_RUN_CODEX_LIVE=1` for real provider execution, loopback transport, normalized workspace/artifact paths, artifact scanning, raw prompt/output non-persistence, and idempotency/no duplicate provider launch.
- Automated tests must use mocked provider output and must not run Codex or Claude provider CLIs.

Evidence:

- xhigh planning subagent returned the SV11 recommendation and boundaries.
- Local inspection confirmed the existing live launch primitives are CLI-owned in `crates/capo-cli/src/adapter_dispatch_run.rs`, while server-owned execution needs its own command shape.

Result:

- SV10 is complete as a planning gate.
- SV11 is selected as the next implementation slice.

## SV11 - Server-Owned Codex Live Run-Local

Status: completed on 2026-05-27

Objective:

Add the first server-owned live-provider execution command shape, using a shared live-run command but enabling only Codex in this slice. The command must run through the server boundary and be testable with mocked provider output without executing provider CLIs in automation.

Acceptance:

- Add `capo server dispatch live-run-local --dispatch-plan ID --goal GOAL` for embedded and `--connect` paths.
- Require the latest SV9 live preflight to be ready for the same dispatch plan, adapter, session, run, turn, prompt hash, and policies.
- Enable only `codex_exec`; reject Claude/ACP with `provider_not_enabled_for_first_live_slice` and no provider launch.
- Require explicit `CAPO_SERVER_RUN_CODEX_LIVE=1` for real Codex execution.
- Allow deterministic mocked provider output only behind explicit mock-runtime opt-in so CI can prove server ingress without running provider CLIs.
- Normalize workspace/artifact paths and reject credential/config/session-store-like locations before any real launch.
- Use restrictive Codex launch defaults from the adapter layer for real runs.
- Scan stdout/stderr artifacts before ingestion; do not persist or render raw prompts or raw provider output.
- Persist execution metadata, artifact refs, scan status, raw policies, event counts, and dashboard/status visibility.
- Prove the running-server CLI path with mocked Codex output.

Must not do:

- Do not run Codex or Claude provider CLIs in automated tests.
- Do not enable Claude live execution in this first slice.
- Do not read OAuth tokens, cookies, keychains, session stores, vendor config, or API keys.
- Do not persist/render raw prompts or raw provider output.
- Do not add non-loopback transport, tunnels, remote clients, or public listeners.
- Do not route through direct `capo adapter run-local` compatibility paths.

Progress:

- Added `ServerCommand::RunLiveProviderLocal`.
- Added server-owned `live_provider.rs` for live-run validation, Codex-only enablement, path policy, mock provider output ingestion, artifact scanning, and real Codex launch wiring.
- Added `capo server dispatch live-run-local` for embedded and `--connect` CLI paths.
- Added transport encoding/decoding for live-run requests.
- Added server tests for Codex mocked live-run ingress and Claude blocked behavior.
- Added process-level running-server test for Codex mocked live-run through CLI transport.
- Added regressions for stale prompt hash blocking, credential-like artifact path blocking, and repeated mocked live-run returning the existing execution rather than duplicating tool state.
- Added symlink/canonical path regression coverage for credential-like artifact targets.
- Added event-count assertion proving repeated mocked live-run does not duplicate stream, replay, execution, or audit events.

Evidence so far:

- `crates/capo-server/src/live_provider.rs`
- `crates/capo-server/src/lib.rs`
- `crates/capo-server/src/types.rs`
- `crates/capo-server/src/dispatch.rs`
- `crates/capo-server/src/transport.rs`
- `crates/capo-cli/src/server_client.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-cli/src/cli_surface.rs`
- `crates/capo-server/src/tests.rs`
- `crates/capo-cli/tests/server_transport.rs`
- `cargo fmt`
- `cargo test -p capo-server server_live_provider_local_run -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_live_runs_codex_mock_output_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Initial xhigh SV11 review attempts stalled and were closed without findings.
- A narrow xhigh review returned no must-fix findings.
- Accepted follow-ups from review:
  - Added symlink/canonical path coverage for credential-like path targets.
  - Added event-count assertion for repeated mocked live-run idempotency.
- Retained follow-up: add explicit redaction/bounds coverage for real provider stdout/stderr artifacts before broad live-provider use.

Result:

- SV11 is review-complete for the server-owned live-run command shape and mocked Codex ingress.
- Capo can now drive mocked Codex provider output through `capo server dispatch live-run-local` over the running server boundary without executing provider CLIs in automation.
- Real Codex execution remains opt-in only and should be exercised manually in a later task.

Follow-ups:

- Split `crates/capo-server/src/tests.rs`, `crates/capo-server/src/lib.rs`, `crates/capo-server/src/transport.rs`, and `crates/capo-cli/src/server_client.rs` before adding more live-provider behavior; all are in or near the warning zone.
- Add a manual opt-in Codex smoke run only after review confirms the launch path is safe enough to exercise locally.

## SV12 - LLM-Friendly Server/CLI Split

Status: completed on 2026-05-27

Objective:

Split the large server, server-client, transport, and process-level test surfaces before adding real Codex smoke behavior, so the next safety-sensitive live-provider work is reviewable by coding agents.

Acceptance:

- Preserve behavior and public API compatibility for `CapoServer`, `ServerRequest`, `ServerCommand`, `ServerResponse`, `send_tcp`, `serve_tcp`, and CLI command output.
- Do not add product features or run real Codex/Claude provider CLIs.
- Split by responsibility, not arbitrary chunks.
- Bring the hottest files under the warning zone where practical, prioritizing `capo-server/src/tests.rs`, `capo-cli/src/server_client.rs`, `capo-server/src/lib.rs`, `capo-server/src/transport.rs`, and `capo-cli/tests/server_transport.rs`.
- Keep command routing easy to audit; route arms may delegate but should remain the semantic map.
- Validate with focused server/server-transport tests, clippy, full tests, `git diff --check`, and `wc -l` evidence.

Planning:

- Xhigh planning recommended SV12 as a behavior-preserving split before manual Codex smoke because real provider execution will touch server, transport, CLI, and test surfaces.
- Follow-up SV13 should be a manual real Codex smoke through the loopback server with `CAPO_SERVER_RUN_CODEX_LIVE=1`.

Progress:

- Split `crates/capo-server/src/tests.rs` into feature modules:
  - `tests/foundation.rs`
  - `tests/replay.rs`
  - `tests/live_provider.rs`
  - `tests/dispatch.rs`
  - `tests/sessions.rs`
  - `tests/transport.rs`
- Split server dispatch CLI handlers into `crates/capo-cli/src/server_client/dispatch.rs`.
- Split server TCP runtime from transport JSON codec:
  - `crates/capo-server/src/transport.rs`
  - `crates/capo-server/src/transport/codec.rs`
  - `crates/capo-server/src/transport/wire.rs`
- Split process-level server transport integration tests by scenario:
  - `crates/capo-cli/tests/server_transport/basic.rs`
  - `crates/capo-cli/tests/server_transport/replay.rs`
  - `crates/capo-cli/tests/server_transport/dispatch.rs`
  - `crates/capo-cli/tests/server_transport/live.rs`
  - `crates/capo-cli/tests/server_transport/support.rs`
- Split live-provider preflight command handling from the server router into `crates/capo-server/src/live_provider.rs`.
- Moved the small recovery command body from the server router into `crates/capo-server/src/server_core.rs`.

Evidence:

- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`
- xhigh review of the transport split found no must-fix issues and recommended the live-provider preflight split.
- `wc -l` now shows `crates/capo-server/src/tests.rs` reduced to a small module root, `crates/capo-cli/src/server_client.rs` reduced to 496 LOC, `crates/capo-server/src/transport.rs` reduced to 149 LOC, `crates/capo-cli/tests/server_transport.rs` reduced to a module root, and `crates/capo-server/src/lib.rs` reduced to 799 LOC.

Result:

- SV12 preserves the server, transport, and CLI behavior while making the next Codex live-provider smoke easier to review.
- The remaining larger files are responsibility-oriented modules under the warning threshold: transport codec, live-provider command handling, dispatch helpers, and feature-specific tests.
- Follow-up SV13 should run the manual real Codex smoke through the loopback server with `CAPO_SERVER_RUN_CODEX_LIVE=1`.

## SV13 - Manual Real Codex Smoke Through Running Server

Status: completed on 2026-05-27

Objective:

Prove real Codex execution can run through the same running server boundary as mocked Codex, without weakening subscription safety or leaking raw prompt/output/secrets into durable state.

Acceptance:

- Start a loopback `capo server serve` process and drive `register`, `session start`, live preflight, and `dispatch live-run-local` from CLI commands using `--connect`.
- Execute Codex only with explicit manual opt-in: `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and `CAPO_SERVER_RUN_CODEX_LIVE=1`.
- Keep Claude blocked for this first real live-provider slice.
- Verify dashboard/status and restart recovery after the live run.
- Inspect persisted events/artifacts enough to confirm raw prompt policy, raw output policy, credential scan status, provider execution flags, and no obvious secret/session material in server request audit payloads.
- Record exact command transcript shape, redaction/bounds observations, and any required follow-up before broadening live-provider use.
- Run focused validation after any fixes; do not add broad product behavior in this task unless the smoke reveals a must-fix correctness/safety issue.

Planning:

- Xhigh planning recommended a loopback-only manual smoke with fresh `/tmp` state/artifact roots and inline opt-in env vars.
- The planner found a must-fix: server-native session start persisted raw `--goal` into durable state. That was fixed before final smoke evidence was accepted.

Progress:

- Ran real Codex through the running server path using:
  - `capo server serve --addr 127.0.0.1:0 --max-requests 5`
  - `capo server agent register --connect`
  - `capo server session start --connect`
  - `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 capo server dispatch live-preflight --connect`
  - `CAPO_SERVER_RUN_CODEX_LIVE=1 capo server dispatch live-run-local --connect`
  - `capo server dashboard --connect`
  - restarted server, then `capo server recover --connect` and `capo server agent status --connect`
- Fixed durable raw-goal persistence for server-native session start by storing `goal_hash:<hash>;raw_policy:not_rendered`.
- Fixed real live-provider run-exit and replay metadata so real provider execution is not mislabeled as deterministic fixture ingest.
- Fixed xhigh review must-fix: `RunLiveProviderLocal` now rechecks the current request's prompt hash before returning an existing real provider execution.
- Fixed xhigh review must-fix: `ServerRequest::cli(StartSession { .. })` now hashes the goal in the default request ID instead of slugging raw goal text.
- Fixed review follow-ups: real provider stdout rejects zero normalized events, and run-exit audit idempotency/event IDs include provider execution metadata.
- Fixed final xhigh closeout finding: mocked and real `run.exited` audits for the same dispatch plan can no longer collide on the same event ID.
- Added regression coverage for hash-only server-native session goals and mocked live-run metadata.

Evidence:

- `codex --version`: `codex-cli 0.134.0`
- Final smoke state root: `/tmp/capo-sv13-state-1779902046`
- Final smoke artifact root: `/tmp/capo-sv13-artifacts-1779902046`
- Final smoke log root: `/tmp/capo-sv13-logs-1779902046`
- Final live-run output: `provider_cli_executed=true`, `mock_runtime_opt_in=false`, `status=exited`, `credential_scan_status=clean`, `raw_prompt_policy=not_rendered`, `raw_output_policy=bounded_redacted_artifacts`, `reason_codes=provider_cli_executed_and_artifacts_scanned`, `input_events=4`, `appended_events=2`, `summary_events=1`, `completed_turns=1`.
- Dashboard/status after restart showed `run_status=exited`, `dispatch_provider_cli_executed=true`, `dispatch_credential_scan_status=clean`, and the same dispatch execution ID.
- Secret marker scan over final state, artifacts, and logs returned no matches for token/cookie/key/session markers.
- SQLite inspection showed task/session goals persisted as `goal_hash:9f0f498029779a1c;raw_policy:not_rendered`; relevant event payloads did not contain the raw prompt.
- Output artifacts were bounded: stdout 339 bytes, stderr 39 bytes.
- `cargo test -p capo-server server_live_provider_local_run -- --nocapture`
- `cargo test -p capo-server server_live_provider_local_run_rechecks_prompt_after_existing_real_execution -- --nocapture`
- `cargo test -p capo-server server_native_session_start_persists_goal_hash_instead_of_raw_goal -- --nocapture`
- `cargo test -p capo-server server_live_provider_run_exit_audit_distinguishes_mock_and_real_metadata -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_live_runs_codex_mock_output_through_running_server_process -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV13 review found two must-fixes: existing real executions could bypass stale-goal checks, and default server request IDs could include raw `StartSession` goals.
- Both must-fixes were accepted and fixed with focused regression coverage.
- Xhigh should-fixes for run-exit idempotency metadata and empty real stdout parsing were also accepted and fixed.
- Final xhigh closeout found run-exit `event_id` was still plan-scoped; accepted and fixed by adding provider execution metadata to the event ID and adding a mock-vs-real audit regression.

Result:

- SV13 proves real Codex can run through a loopback Capo server using the same CLI-through-server boundary as mocked agents.
- The server records provider execution and restart/status state without durable raw goal storage in server-native session/task state.
- Claude remains blocked for live execution in the first live-provider slice.
