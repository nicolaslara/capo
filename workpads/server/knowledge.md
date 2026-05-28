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

## SV1 - CLI Client Through Server Boundary

Status: completed on 2026-05-27.

Implementation so far:

- Added a server-backed CLI namespace under `capo server ...`.
- The new CLI namespace opens `CapoServer` and sends typed `ServerRequest` values instead of calling `FakeBoundaryController` directly.
- Server responses include `server_boundary=capo-server`, request ID, client ID, actor ID, and input origin so tests and humans can see the command crossed the server boundary.
- Mutating server requests emit `server.request_handled` events. Task-send audit events are scoped to the task/agent/session/run so they show up in session event history and correlate to server request metadata.
- Review feedback accepted: task sends now reject unknown agents at the server boundary and reject repeated sends to an agent that already has a session while the mock controller still uses fixed session/run IDs.
- Audit payloads use JSON encoding and event IDs include a hash of the full server idempotency identity.
- Existing direct `capo agent`, `capo task`, `capo session`, and `capo recover` commands remain compatibility paths.

Verification so far:

- `cargo fmt`
- `cargo test -p capo-server`
- `cargo test -p capo-cli server_cli_routes_agent_work_through_server_boundary -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Review:

- Xhigh review required fixing repeated send behavior, CLI-level audit proof, audit JSON/event identity, and unknown-agent task-send mapping before commit.
- All required review fixes were applied.

Deferred:

- Replace the temporary repeated-send rejection with request/task-aware session and run identities in the controller facade.
- Flip selected normal CLI commands to server-backed defaults only after the runnable transport path is available.

## SV2 - Runnable Local Server Transport

Status: completed on 2026-05-27.

Implementation so far:

- Added a foreground local server transport in `capo-server`.
- The transport is loopback TCP by default, using newline-delimited JSON frames that decode into `ServerRequest`, call `CapoServer::handle`, and encode `ServerResponse`.
- Added `capo server serve --addr ADDR [--max-requests N]`.
- Server-backed CLI commands now accept `--connect ADDR`; without `--connect`, they keep the embedded SV1 path.
- Added a process-level integration test that starts `capo server serve`, connects separate CLI processes, and verifies register/send/status/dashboard/recover through the running server.
- The process-level integration test now stops the first server before recovery, starts a second server on the same state root, runs `server recover --connect`, and verifies the recovered run state.

Decision:

- An xhigh planning pass recommended Unix-domain sockets for local-only safety. SV2 uses loopback TCP instead because it keeps the transport local by default while also aligning with later Tailscale/remote-control work. The server binds to the user-provided address and examples/tests use `127.0.0.1`; public binding and authentication remain future hardening.
- Xhigh review required loopback-only enforcement before commit. `capo server serve` now rejects non-loopback bind addresses; explicit public/remote exposure must go through a future authenticated exposure path.
- `--addr` and `--connect` now fail closed when present without values rather than falling back to defaults or embedded mode.

Verification:

- `cargo fmt`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Deferred:

- Unix-domain socket transport for same-machine hardening.
- TCP auth tokens and explicit exposure records before any non-loopback or tunneled listener.
- Read/write timeouts for long-lived or dashboard-style clients.

## SV3 - Codex Agent Through Server

Status: completed on 2026-05-27.

Implementation so far:

- Added a server-owned adapter fixture replay path for Codex JSONL.
- `capo server adapter replay-fixture --adapter codex --fixture PATH --agent NAME --goal GOAL [--connect ADDR]` sends the request through the same server boundary and transport path as mocked-agent commands, after the agent has been explicitly registered with the server.
- The CLI reads the fixture file, but the server owns parsing, normalized adapter event application, state mutation, and dashboard/recovery visibility.
- The replay path records `provider_cli_executed=false`, hashes fixture content, and reports `raw_content_policy=content_hashed_not_rendered`; it does not inspect Codex subscription credentials, OAuth/session stores, cookies, keychain entries, or vendor config files.
- Review feedback accepted: replay safety metadata is now persisted in the server request audit event and asserted after server restart/recovery, instead of existing only in the immediate response.

Decision:

- Use fixture replay as the first SV3 proof because the workpad explicitly allows execute or replay. This proves Codex-shaped adapter events through the server without requiring a live subscription CLI run in normal tests.
- Keep live Codex execution as an opt-in follow-up behind existing dispatch/run-local preflight and explicit provider execution controls.

Verification so far:

- `cargo fmt`
- `cargo test -p capo-server server_replays_codex_fixture_through_server_boundary -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_replays_codex_fixture_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Review:

- Xhigh review required durable replay safety metadata before commit/readiness. The fix persists provider execution status, raw content policy, fixture hash, and raw-body non-persistence in the server request audit event and asserts those fields after restart.
- Follow-up findings were recorded below: old compatibility replay bypass, fixture size/wire policy, and fake-session bootstrap wording/architecture.

Deferred:

- Live Codex subscription execution through the running server with explicit opt-in and artifact scanning.
- A server-native dispatch plan/gate/run-local workflow so live provider execution is not coupled to CLI-only helper modules.
- Raw adapter event artifact retention with redaction metadata, if needed for deeper replay/debug.
- Deprecate or clearly mark the old direct `capo adapter replay-fixture` compatibility path before treating server replay as the default Codex proof path.
- Add a fixture-size cap and explicit operator docs for raw fixture transport: raw fixture bodies are local-loopback request input only and must not be persisted.
- Replace the fake-session bootstrap used by replay with a server-native adapter session start so `provider_cli_executed=false` cannot be confused with “nothing executed at all.”

## SV4 - Review Gate And Next Product Slice

Status: completed on 2026-05-27.

Review findings:

- Broad xhigh review found that server request idempotency could hide a real mutation from the audit trail if the caller reused a request ID for a different command. Accepted fix: server audit idempotency now includes a command identity hash, and a regression proves two different registrations with the same request ID produce two server audit events.
- Broad xhigh review found that `--connect` could send a request to a non-loopback host even though replay fixtures are documented as local-loopback request input only. Accepted fix: the CLI validates `--connect` with the same loopback resolver guard before calling `send_tcp`.

Verification so far:

- `cargo fmt`
- `cargo test -p capo-server server_request_idempotency_is_bound_to_command_identity -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Decision:

- SV0-SV3 are accepted as the server foundation after fixing command-hash-bound server audit idempotency and client-side loopback enforcement for `--connect`.
- The next product slice should be SV5: an ACP-first server session model. This means server-native sessions and turns with ACP-shaped adapter ingress, not full ACP runtime execution yet.
- This is higher leverage than richer CLI, dashboard, or network hardening because it removes the fake-session bootstrap from replay, fixes fixed-ID/repeated-send limitations structurally, and gives live Codex dispatch a server-owned session/run/turn target.

Follow-ups retained:

- Add fixture size limits and sanitize/pseudonymize fixture labels before general operator replay use.
- Mark direct `capo adapter replay-fixture` as compatibility/deprecated once server replay becomes the default path.
- Replace fake-session bootstrap with server-native adapter session start before live provider/session ownership claims.

## SV5 - ACP-First Server Session Model

Status: completed on 2026-05-27.

Implementation so far:

- Server-native adapter sessions now start through `ServerCommand::StartSession` and `capo server session start`.
- Adapter fixture replay now targets an existing server session/run/turn by explicit `session_id`, `run_id`, and `turn_id`; it no longer creates a fake session from `agent_name` and `goal`.
- Codex fixture replay and ACP-shaped fixture replay use the same server session ingress path: parse provider events to normalized adapter events, apply them to the existing run, and persist state through controller projections.
- Multiple historical sessions per agent are supported by explicit session/run IDs after recovery has marked the previous run non-running. Concurrent active sessions for one agent are rejected for now because the dashboard/read-model contract exposes one current session per agent.
- `StartSession` rejects duplicate run IDs so a new session cannot silently reassign an existing run projection.
- Replay rejects missing sessions and oversized fixture bodies before state mutation.
- Replay audit events persist adapter kind, fixture hash, provider execution status, raw content policy, raw body non-persistence, target session/run/turn IDs, and local-loopback raw fixture scope.
- The local TCP transport caps request frames before JSON decoding so arbitrary loopback clients cannot bypass the fixture cap by sending a very large frame.
- Server dashboard/status summaries expose adapter kind, evidence refs/count, turn ids/count, tool counts, memory counts, and recovered run status.
- The command identity hash no longer formats raw fixture text into the server audit idempotency key; replay identity uses adapter, session, fixture name, and fixture hash metadata.
- The direct `capo adapter replay-fixture` path remains available but is now labeled as a compatibility bypass in CLI help. The product path is `capo server session start` followed by `capo server adapter replay-fixture --session`.

Decision:

- Start with explicit session IDs and run IDs for deterministic tests and recovery. Default IDs remain available for operator use, but live provider dispatch should pass IDs allocated by the server dispatch/session planner.
- Treat explicit turn IDs as Capo-owned replay/ingress IDs for now. Provider timeline keys still exist inside normalized events, but server commands name the Capo turn that receives those events.
- Reject concurrent active sessions per agent until the dashboard and control model can represent and steer more than one active session for the same agent.
- Treat fixture bodies as local-loopback request input only. The server records hashes and metadata, not raw provider content.
- Use ACP-shaped fixture replay as the deterministic ACP ingress proof for SV5. Full ACP runtime execution remains a later provider connector slice.

Verification so far:

- `cargo fmt`
- `cargo test -p capo-server server_replays_codex_fixture_through_server_boundary -- --nocapture`
- `cargo test -p capo-server server_replays_acp_fixture_into_server_native_session -- --nocapture`
- `cargo test -p capo-server server_native_sessions_allow_multiple_historical_sessions_per_agent -- --nocapture`
- `cargo test -p capo-server server_rejects_adapter_fixture -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_oversized_frames_before_json_decode -- --nocapture`
- `cargo test -p capo-server server_request_idempotency_is_bound_to_command_identity -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_replays_codex_fixture_through_running_server_process -- --nocapture`
- `cargo test -p capo-cli help_mentions_command_envelopes_and_no_credentials -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV5 review found four must-fix issues: duplicate run IDs, replay missing explicit run/turn keys, transport caps after JSON decode, and insufficient dashboard/status metadata.
- Accepted all four fixes in the same slice.

Deferred:

- Live Codex subscription execution through the running server with explicit opt-in and artifact scanning.
- A server-native dispatch plan/gate/run-local workflow so live provider execution is not coupled to CLI-only helper modules.
- Raw adapter event artifact retention with redaction metadata, if needed for deeper replay/debug.
- Validate replay adapter kind against persisted session adapter metadata once an adapter-session read model exists.
- Add a runtime output warning for direct `capo adapter replay-fixture`, beyond the help-text compatibility label.
- Sanitize or hash fixture labels if fixture paths become externally visible.

## SV6 Planning

Status: planned on 2026-05-27.

Xhigh planning recommendation:

- Make SV6 server-native dispatch plan/gate/run-local.
- Do not jump straight to live Codex execution. The existing dispatch chain still lives mostly in CLI-side `capo adapter ...` helpers, and live provider execution should run through server-owned planning, gates, runtime launch, artifact scanning, and server-native stream ingress first.
- Keep the slice local and deterministic: process-level tests should use mocked/deterministic runtime output, not a live Codex subscription run.

Rejected next-slice options:

- Live Codex through the running server: high value, but premature before server-owned dispatch planning/gating/runtime execution.
- Richer interactive CLI loop: useful after the control-plane path exists; otherwise it is UX over an incomplete server path.
- Dashboard-only improvements: some dashboard metadata is already present after SV5; richer dashboard work should follow server-native execution records.
- Network hardening: current loopback-only posture is sufficient for the next local-control-plane slice. Do not add public/tunneled exposure in SV6.

## SV6 - Server-Native Dispatch Plan/Gate/Run-Local

Status: completed on 2026-05-27.

Implementation so far:

- Added typed server commands and responses for `PlanDispatch`, `GateDispatch`, and `RunDispatchLocal`.
- Added `capo server dispatch plan`, `capo server dispatch gate`, and `capo server dispatch run-local` for both embedded and `--connect` paths.
- Dispatch planning now validates the target agent/session/run relationship, normalizes adapter kind, records a deterministic runtime target, records prompt source metadata by hash, and persists an `AdapterDispatchPlan` plus `AdapterDispatchPromptSource`.
- Gate/preflight persists `AdapterDispatchGate`, `AdapterDispatchPromptMaterialization`, and `AdapterDispatchExecutionRequest`. The SV6 deterministic path fails closed unless `CAPO_SERVER_DETERMINISTIC_DISPATCH=1` is present on the planning CLI command and the deterministic runtime/artifact-scan policies are intact.
- Run-local uses deterministic fixture output, parses it through the adapter parser, ingests normalized events into the explicit server-owned session/run/turn, and persists dispatch execution and replay projections.
- Dashboard/status summaries now expose dispatch plan/gate/execution IDs, execution status, runtime process ref, credential scan status, raw prompt/output policies, turn IDs, tool counts, memory counts, evidence refs, and recovered run state.
- Repeated `RunDispatchLocal` with the same request identity and fixture is idempotent: no duplicate process/projection/audit state is added, and adapter stream projections remain single-copy. Re-running the same dispatch plan with a changed fixture hash is rejected before stream ingestion.
- The process-level transport test drives register -> session start -> dispatch plan -> gate -> run-local -> dashboard -> restart -> recover -> status through a running `capo server serve` process.

Decision:

- Keep SV6 deterministic and local. It does not execute live Codex, read subscription state, or require provider credentials in CI.
- Treat SV6 deterministic fixture ingestion as `provider_cli_executed=false`. It proves the server-owned dispatch path without claiming a live provider CLI or credential scan ran.
- Keep prompt and output bodies non-rendered/non-persisted by the server path. The server records hashes, policy labels, artifact refs, counts, and scan status.
- Use dispatch records as the dashboard-visible adapter kind fallback when stream events have not yet provided adapter metadata.
- Mark a successful deterministic dispatch run `exited` so restart recovery does not have to convert it to `exited_unknown`.

Verification so far:

- `cargo fmt`
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

- Xhigh SV6 review found must-fix issues: gate was not fail-closed, deterministic run-local claimed provider execution and clean credential scan, changed-fixture reruns could append more stream state under the same execution identity, run status remained `running` after deterministic dispatch, and dashboard/status omitted provider execution booleans.
- Accepted fixes:
  - Dispatch planning records explicit deterministic opt-in from `CAPO_SERVER_DETERMINISTIC_DISPATCH=1`; gate blocks without it and records `missing_deterministic_fixture_opt_in`.
  - Deterministic fixture run-local records `provider_cli_executed=false` and `credential_scan_status=not_applicable_fixture`.
  - The server rejects changed fixture hashes for a previously executed dispatch plan before parsing or ingesting events.
  - Successful deterministic run-local appends a terminal `run.exited` projection.
  - Dashboard/status now expose `dispatch_provider_cli_execution_allowed` and `dispatch_provider_cli_executed`.

Follow-ups:

- `crates/capo-server/src/lib.rs` has grown to roughly 2,000 LOC. This is beyond the LLM-friendly file target and should be split by responsibility soon, even if SV6 review accepts behavior.
- The SV6 gate is intentionally deterministic and not yet the full live-provider permission/credential/artifact gate. Live Codex execution must remain follow-up work.
- Adapter replay idempotency is still not fully session/run-scoped at the normalized adapter-event key level; keep it as a follow-up before broad multi-session replay.

## SV7 - Server Crate Responsibility Split

Status: completed on 2026-05-27.

Implementation:

- Split `capo-server` by responsibility while preserving the public server boundary.
- `lib.rs` now owns the `CapoServer` struct and the top-level `handle` command router. Keeping the router together makes command semantics easier to audit while still moving helper logic out of the file.
- `types.rs` owns `ServerRequest`, `ServerCommand`, `ServerResponse`, `ServerError`, origin types, and response summary structs.
- `dashboard.rs` owns dashboard/status summary projection and agent lookup helpers.
- `dispatch.rs` owns dispatch plan/gate/run-local helper methods, dispatch projections, replay projections, and dispatch execution outcome handling.
- `server_core.rs` owns shared command-envelope, response, run lookup, and server audit helpers.
- `transport.rs` owns the loopback TCP server/client path and request/response frame handling.
- `transport/wire.rs` owns transport wire conversion helpers and typed error serialization.
- `util.rs` owns shared adapter parsing, adapter/provider label normalization, command identity hashing, turn extraction, slugging, and stable hashing.

Decisions:

- Keep `CapoServer::handle` in one file for now. It is still the primary semantic map from command to behavior, and splitting each command arm before the review gate would make cross-command invariants harder to inspect.
- Treat `types.rs` as a boundary surface, not an implementation dumping ground. Future feature-specific summaries should move into narrower modules if they become difficult to navigate.
- Keep `tests.rs` as one file for now because it is under the project test-file upper bound, but split by feature once new live-provider or Claude coverage lands.
- Recorded one behavior correction during the split: dispatch replay metadata now preserves the replay's `provider_cli_executed` value instead of hard-coding it to `true`.

Module size evidence:

- `lib.rs`: 704 LOC.
- `dispatch.rs`: 594 LOC.
- `types.rs`: 454 LOC.
- `transport.rs`: 696 LOC.
- `transport/wire.rs`: 154 LOC.
- `server_core.rs`: 154 LOC.
- `dashboard.rs`: 119 LOC.
- `util.rs`: 105 LOC.
- `tests.rs`: 1030 LOC.

Verification:

- `cargo fmt`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review caveat:

- Xhigh SV7 planning and review subagents were attempted, but both timed out or stalled and were closed without returning findings.
- Because no xhigh findings were returned, SV7 is implementation/validation complete but not a sufficient gate for live-provider execution.
- The next task must begin with a fresh xhigh review of the split and current server path before any live Codex or Claude Code execution is added.

## SV8 - Server Split Review Gate

Status: completed on 2026-05-27.

Xhigh review findings:

- Must-fix: `capo-server::serve_tcp` did not enforce loopback at the public server crate boundary. The CLI had a loopback guard, but callers could use the re-exported `serve_tcp` with a non-loopback listener.
- Must-fix: adapter kind was not bound to the server session before replay or dispatch planning. A Codex session could accept ACP/Claude replay or dispatch commands if the session/run IDs matched.
- Live-provider readiness finding: the current server dispatch gate is deterministic-fixture-only. It hardcodes deterministic runtime shape and does not yet implement provider connector authorization, capability profile checks, workspace/runtime policy, credential-scan plan, or artifact retention policy for live Codex/Claude execution.

Accepted fixes:

- `serve_tcp` now checks `listener.local_addr()` and rejects non-loopback listeners before opening `CapoServer` or accepting requests.
- Added `ServerError::AdapterSessionMismatch` and transport wire serialization for it.
- Added `CapoServer::require_session_adapter`, which reads the session's recorded adapter metadata from session events and rejects requested adapter kinds that do not match.
- `ReplayAdapterFixture` now normalizes the requested adapter, validates the existing session's adapter, and only then parses the fixture or mutates state.
- `PlanDispatch` now validates that the requested adapter matches the existing server session before creating a dispatch plan.

Verification:

- `cargo fmt`
- `cargo test -p capo-server server_rejects_adapter_replay_and_dispatch_that_mismatch_session_adapter -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_non_loopback_listener_at_server_boundary -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Decision:

- The two foundation issues are fixed in the server path before live-provider work.
- Do not treat the deterministic fixture gate as a live-provider gate. The next product slice should add a real live-provider preflight/authorization path before any server command can execute Codex or Claude Code for real.

Planning:

- Xhigh next-slice planning was attempted after the SV8 fixes but stalled and was closed without returning a recommendation.
- The next slice is selected from the completed review evidence: live-provider preflight/authorization before live execution. This directly addresses the remaining review finding and keeps the product path honest.

Follow-ups:

- Split `crates/capo-server/src/tests.rs` by feature before live-provider tests grow much further.
- Split `crates/capo-server/src/transport.rs` again once a second transport is added or payload codecs grow.
- Keep direct `capo adapter ...` compatibility paths visibly marked as bypasses until they are removed or wrapped through server instrumentation.

## SV9 - Live Provider Preflight And Authorization Gate

Status: completed on 2026-05-27.

Implementation:

- Added server-owned live-provider preflight through `ServerCommand::PreflightLiveProvider`.
- Added CLI support as `capo server dispatch live-preflight` for embedded and `--connect` paths.
- The command supports Codex and Claude Code adapter kinds through the same request shape and rejects ACP as a non-provider live-preflight target.
- The preflight validates the existing server agent/session/run target and reuses the adapter/session mismatch guard before it records any preflight state.
- The preflight records dispatch plan, gate, and execution-request projections with `provider_cli_executed=false`.
- Dashboard/status summaries now expose latest dispatch gate status, reason codes, and next action, so preflight readiness is visible without a provider execution record.
- The CLI derives opt-in from `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1`; missing opt-in is fail-closed.

Security decisions:

- SV9 does not execute Codex or Claude Code.
- SV9 does not inspect credential/session stores, OAuth tokens, cookies, keychains, or raw provider config.
- SV9 does not persist raw prompts or raw provider output.
- Credential policy is represented as metadata-only policy text: `metadata_only_no_secret_read`.
- Tool-wrapper policy is represented explicitly as `capo_wrapped_required`; future live execution must satisfy this before launching a provider CLI.

Verification:

- `cargo fmt`
- `cargo test -p capo-server server_live_provider_preflight -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_preflights_live_codex_and_claude_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Xhigh SV9 review found two must-fixes: live-provider preflight idempotency omitted authorization inputs/outcome, and default preflight request IDs included a slugged raw goal.
- Accepted fixes:
  - Preflight identity now includes stored capability/runtime/credential/raw-output/tool policy fields, opt-in, reason codes, and status. Repeated preflights with changed policy cannot leave stale ready state.
  - Default live-provider preflight request IDs use adapter, agent, session, run, turn, and a goal hash instead of `slug(goal)`.
  - Invalid policy values are stored as `rejected` rather than arbitrary caller-provided strings.
  - Public `send_tcp` now rejects non-loopback addresses at the server transport boundary.
- SV9 is review-complete for preflight-only behavior. It is not permission to launch live provider CLIs.

Additional verification:

- `cargo test -p capo-server server_live_provider_preflight_changed_policy_does_not_leave_stale_ready_gate -- --nocapture`
- `cargo test -p capo-server server_live_provider_preflight_default_request_id_does_not_slug_raw_goal -- --nocapture`
- `cargo test -p capo-server tcp_transport_rejects_non_loopback_connect_at_server_boundary -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Follow-ups:

- Before actual live execution, replace shallow workspace/artifact path checks with normalized path policy and explicit rejection of credential/config/session-store paths.
- Split large server files again before live execution if the next slice touches the same files heavily.

## SV10 - Live Provider Execution Planning Review

Status: completed on 2026-05-27.

Planning result:

- Xhigh planning recommended a shared live-provider execution shape with Codex enabled first and Claude explicitly blocked for the first live slice.
- Codex is the lower-risk first provider because the repo already has Codex fixture replay, Codex JSONL normalization, and restrictive local launch-plan defaults.
- Claude Code should reuse the eventual command shape, but execution should stay blocked until its permission, MCP, session-persistence, and output-mode behavior receives a focused safety review.
- The existing live-run chain in `crates/capo-cli/src/adapter_dispatch_run.rs` is useful prior art, but it is CLI-owned. The server path needs a server-owned command so Capo can track agents and clients interact through the server boundary.

Decision:

- SV11 is `Server-Owned Codex Live Run-Local`.
- Build the command shape now, make the running-server CLI path work, test with mocked provider output, and keep real provider execution behind `CAPO_SERVER_RUN_CODEX_LIVE=1`.
- Use mocked provider output in automated tests so CI never launches Codex or Claude.
- Keep raw prompts transient request input for real launch only; compare the prompt hash against the preflight prompt source and do not persist/render the raw prompt.

## SV11 - Server-Owned Codex Live Run-Local

Status: completed on 2026-05-27.

Implementation so far:

- Added `ServerCommand::RunLiveProviderLocal` and `capo server dispatch live-run-local`.
- Added `live_provider.rs` so live-run validation and execution logic is no longer owned by direct CLI compatibility paths.
- The command requires a ready SV9 live-provider preflight and compares the live-run goal hash to the preflight prompt hash.
- Only `codex_exec` is enabled. Claude and ACP are blocked before launch with `provider_not_enabled_for_first_live_slice`.
- Real Codex launch requires `CAPO_SERVER_RUN_CODEX_LIVE=1`; mocked provider output requires `CAPO_SERVER_MOCK_LIVE_PROVIDER_RUNTIME=1`.
- Mocked provider output is ingested through server-native session/run/turn ingress and records `provider_cli_executed=false`.
- Real Codex launch wiring uses `CodexExecAdapter::local_launch_plan`, `LocalProcessRunner`, artifact scanning, and server-native adapter event ingestion, but no automated test runs provider CLIs.
- Workspace and artifact paths are normalized before launch and credential/config/session-store-like path components are rejected.
- Live-run compares the supplied execution goal hash against the SV9 preflight prompt hash and blocks stale prompt execution.
- Repeating the same mocked live-run returns the existing execution summary instead of duplicating projected tool state.
- Symlinked artifact paths are canonicalized before policy checks, so a friendly-looking path cannot point into a credential-like directory.
- Repeated mocked live-run now has explicit event-count coverage for no duplicate stream, replay, execution, or audit events.

Security decisions:

- Do not read subscription credentials, OAuth/session stores, cookies, keychains, API keys, or provider config.
- Do not persist or render raw prompts or raw provider output.
- Do not enable non-loopback transport or tunnels.
- Treat mocked provider output as test-only server ingress evidence, not proof that a real provider CLI ran.

Verification so far:

- `cargo fmt`
- `cargo test -p capo-server server_live_provider_local_run -- --nocapture`
- `cargo test -p capo-cli --test server_transport cli_live_runs_codex_mock_output_through_running_server_process -- --nocapture`
- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Review:

- Two initial xhigh SV11 review attempts stalled and were closed without findings.
- A fresh narrow xhigh review returned no must-fix findings.
- Accepted review follow-ups:
  - Symlink/canonical path coverage was added.
  - Repeat-run event-count idempotency coverage was added.
- Retained review follow-up: add explicit redaction/bounds coverage for real provider stdout/stderr artifacts before broader live-provider use.

Result:

- SV11 is complete for a server-owned Codex live-run command shape using mocked provider output in automation.
- The real provider launch path exists behind `CAPO_SERVER_RUN_CODEX_LIVE=1`, but has not yet been manually smoked in this task.

Follow-ups:

- Split large server and server-client files before adding the next provider execution layer.
- After review, run a manual Codex live smoke with `CAPO_SERVER_RUN_CODEX_LIVE=1` against a loopback server and confirm provider execution metadata, artifact scanning, dashboard/status, and restart/recovery behavior.

## SV12 - LLM-Friendly Server/CLI Split

Status: completed on 2026-05-27.

Planning:

- Xhigh planning recommended a behavior-preserving split before manual Codex smoke.
- Rationale: real provider smoke is safety-sensitive and will otherwise layer new live-provider evidence onto files already in the 800-1,700 LOC warning zone.
- Keep `handle` as the semantic command map while moving large bodies or tests behind responsibility-oriented modules.

Implementation so far:

- Split server tests by feature under `crates/capo-server/src/tests/`.
- Split server dispatch CLI handlers into `crates/capo-cli/src/server_client/dispatch.rs`.
- Split server TCP framing/runtime from JSON request/response serialization:
  - `crates/capo-server/src/transport.rs` now owns loopback TCP serving/sending, frame limits, and stream handling.
  - `crates/capo-server/src/transport/codec.rs` owns command/payload JSON encoding and decoding.
  - `crates/capo-server/src/transport/wire.rs` remains the low-level wire parsing/error helper module.
- Split process-level server transport integration tests by scenario under `crates/capo-cli/tests/server_transport/`.
- Split live-provider preflight command handling into `crates/capo-server/src/live_provider.rs`.
- Moved recovery command handling into `crates/capo-server/src/server_core.rs`.

Review:

- Xhigh review found no must-fix issue in the transport split.
- Accepted recommendation: move the `PreflightLiveProvider` command arm from `lib.rs` into `live_provider.rs` because it is one responsibility and already shares the live-provider boundary.
- Watch item: `crates/capo-server/src/transport/codec.rs` is 707 LOC. Future transport command additions should add focused codec round-trip coverage or split command/payload helpers before the codec crosses the 800 LOC warning zone.

Verification:

- `cargo test -p capo-server`
- `cargo test -p capo-cli --test server_transport -- --nocapture`
- `cargo test -p capo-cli server_cli_transport_options_fail_closed -- --nocapture`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `git diff --check`

Final size evidence:

- `crates/capo-server/src/lib.rs`: 799 LOC after moving live preflight and recovery handling out of the router.
- `crates/capo-cli/src/server_client.rs`: 496 LOC after moving dispatch handlers.
- `crates/capo-server/src/tests.rs`: module root only after moving tests into feature files.
- `crates/capo-server/src/transport.rs`: 149 LOC after moving serialization into `transport/codec.rs`.
- `crates/capo-server/src/transport/codec.rs`: 707 LOC, below the warning zone and isolated to serialization mechanics.
- `crates/capo-cli/tests/server_transport.rs`: module root only after moving scenarios into `tests/server_transport/*.rs`.
- Largest process-level transport test module is `crates/capo-cli/tests/server_transport/live.rs` at 238 LOC.
- `crates/capo-server/src/live_provider.rs`: 762 LOC after taking live-provider preflight handling.

## SV13 - Manual Real Codex Smoke Through Running Server

Status: completed on 2026-05-27.

Planning:

- Xhigh planning recommended a loopback-only manual smoke with fresh `/tmp` state and artifact roots, inline opt-in env vars, no mocked-runtime env, and a harmless canary prompt.
- The planner flagged a must-fix before using real prompts: server-native session start persisted the raw `--goal` into session/task state.

Implementation fixes:

- Server-native session start now stores `goal_hash:<hash>;raw_policy:not_rendered` as the task/session goal reference instead of the raw goal.
- `server.request_handled` for session start records `goal_hash` and `raw_goal_policy=not_rendered`.
- Live-provider real execution now records consistent metadata across execution, run-exit, replay, and request audit events:
  - `provider_cli_executed=true`
  - `raw_prompt_policy=not_rendered`
  - `raw_output_policy=bounded_redacted_artifacts`
  - `reason_codes=provider_cli_executed_and_artifacts_scanned`
- Mocked live-provider output keeps explicit non-provider metadata:
  - `provider_cli_executed=false`
  - `raw_content_policy=content_hashed_not_rendered`
  - `reason=mock_live_provider_output_ingested_without_provider_cli`
- Accepted xhigh review must-fixes:
  - Existing real provider executions no longer bypass stale live-run goal checks.
  - Default `ServerRequest::cli(StartSession { .. })` request IDs no longer slug raw goal text.
- Accepted xhigh review follow-ups:
  - Run-exit audit idempotency and event IDs now include provider execution metadata.
  - Real provider stdout parsing now rejects zero normalized events.
- Final closeout review found the run-exit `event_id` was still plan-scoped even after idempotency included provider metadata. The fix adds provider execution metadata to the event ID and is covered by `server_live_provider_run_exit_audit_distinguishes_mock_and_real_metadata`.

Manual smoke evidence:

- `codex --version`: `codex-cli 0.134.0`.
- Final smoke state root: `/tmp/capo-sv13-state-1779902046`.
- Final smoke artifact root: `/tmp/capo-sv13-artifacts-1779902046`.
- Final smoke log root: `/tmp/capo-sv13-logs-1779902046`.
- Running server path:
  - `capo server serve --addr 127.0.0.1:0 --max-requests 5`
  - `capo server agent register --connect`
  - `capo server session start --connect`
  - `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 capo server dispatch live-preflight --connect`
  - `CAPO_SERVER_RUN_CODEX_LIVE=1 capo server dispatch live-run-local --connect`
  - `capo server dashboard --connect`
  - restart server, `capo server recover --connect`, and `capo server agent status --connect`
- Live run output showed `provider_cli_executed=true`, `mock_runtime_opt_in=false`, `status=exited`, `credential_scan_status=clean`, `raw_prompt_policy=not_rendered`, `raw_output_policy=bounded_redacted_artifacts`, `input_events=4`, `appended_events=2`, `summary_events=1`, and `completed_turns=1`.
- Dashboard and status after restart showed the same dispatch execution, `run_status=exited`, `dispatch_provider_cli_executed=true`, and `dispatch_credential_scan_status=clean`.

Safety inspection:

- Secret-marker scan over final state, artifacts, and logs returned no matches for token/cookie/key/session markers.
- SQLite session/task rows stored only `goal_hash:9f0f498029779a1c;raw_policy:not_rendered`.
- Event payloads for `session.started`, `server.request_handled`, `adapter.dispatch_executed`, `run.exited`, and `adapter.dispatch_replayed` did not persist the raw goal.
- Output artifacts were bounded: stdout 339 bytes, stderr 39 bytes.
- The stdout artifact contained the expected canary response `CAPO_SERVER_CODEX_LIVE_OK`.

Verification:

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

- Xhigh SV13 review concluded the smoke evidence was otherwise strong, but not enough to close until the two must-fixes above were addressed.
- The must-fixes and should-fixes were accepted, implemented, and covered by focused regressions plus full validation.
