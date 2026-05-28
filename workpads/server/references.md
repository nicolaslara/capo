# Server Workpad References

## Local Sources

- `project.md` - product source of truth for Capo as server/control plane.
- `TASKS.md` - routes the active workpad to server implementation.
- `WORKING.md` - verification and review rules.
- `workpads/WORKPADS.md` - server load list and workpad rules.
- `workpads/architecture/boundaries.md` - controller/runtime/protocol/state/tunnel/input boundary map.
- `workpads/architecture/state-model.md` - event log, projections, and recovery model.
- `workpads/architecture/protocol-provider.md` - Codex, Claude Code, and ACP adapter direction.
- `workpads/scaffold/knowledge.md` - product-spine correction and deterministic scaffold evidence.

## SV0 Implementation Sources

- `crates/capo-controller/src/lib.rs` - existing controller/state orchestration boundary used by the server.
- `crates/capo-controller/src/fake_session.rs` - deterministic mocked-agent send path.
- `crates/capo-query/src/dashboard.rs` - dashboard read-model query reused by server snapshots.
- `crates/capo-query/src/types.rs` - dashboard row types summarized by the server boundary.
- `crates/capo-server/src/lib.rs` - typed server/control-plane request and response boundary.
- `crates/capo-server/src/tests.rs` - deterministic mocked-agent server-boundary recovery proof.
- `.agents/skills/next/SKILL.md` - Codex next workflow now includes server workpad context.
- `.cursor/commands/next.md` - slash-command next workflow now includes server workpad context.
- `.opencode/commands/next.md` - opencode next workflow now includes server workpad context.

## SV1 Implementation Sources

- `crates/capo-cli/src/server_client.rs` - server-backed CLI client commands.
- `crates/capo-cli/src/main.rs` - routes `capo server ...` commands to the server client module.
- `crates/capo-cli/src/cli_surface.rs` - help text for server-backed CLI commands.
- `crates/capo-cli/src/tests.rs` - server-backed CLI flow coverage.
- `crates/capo-server/src/lib.rs` - request identity/origin propagation and status command.
- `crates/capo-state/src/event.rs` - `server.request_handled` audit event kind.

## SV2 Implementation Sources

- `crates/capo-server/src/transport.rs` - loopback TCP transport and JSON wire conversion.
- `crates/capo-cli/src/server_client.rs` - `capo server serve` and `--connect` client routing.
- `crates/capo-cli/tests/server_transport.rs` - process-level running server proof.

## SV3 Implementation Sources

- `crates/capo-server/src/lib.rs` - server-owned Codex fixture replay command and replay summary.
- `crates/capo-server/src/transport.rs` - transport encoding/decoding for adapter fixture replay.
- `crates/capo-cli/src/server_client.rs` - `capo server adapter replay-fixture` client command.
- `crates/capo-cli/tests/server_transport.rs` - process-level Codex replay through a running server and recovery proof.
- `crates/capo-adapters/fixtures/codex-exec.jsonl` - deterministic Codex JSONL fixture used for SV3 replay.

## SV4 Review Sources

- `crates/capo-server/src/lib.rs` - command-hash-bound server audit idempotency.
- `crates/capo-server/src/tests.rs` - regression for reused request IDs with different commands.
- `crates/capo-cli/src/server_client.rs` - loopback guard for `--connect`.
- `crates/capo-cli/src/tests.rs` - fail-closed CLI transport option coverage.
- xhigh review subagent - broad server foundation review through SV3.
- xhigh planning subagent - recommended SV5 as ACP-first server session model before live Codex dispatch.

## SV5 Implementation Sources

- `crates/capo-server/src/lib.rs` - `StartSession`, session/run/turn-targeted adapter replay, run-ID uniqueness, fixture cap, dashboard metadata, replay metadata, and raw-content-safe command identity.
- `crates/capo-server/src/transport.rs` - transport encoding/decoding for server-native session start, session/run/turn-targeted replay, dashboard metadata, error mapping, and bounded request frames.
- `crates/capo-server/src/tests.rs` - Codex, ACP-shaped fixture, multiple-session, duplicate-run, missing-session, fixture-size, and transport-frame regression coverage.
- `crates/capo-cli/src/server_client.rs` - `capo server session start`, session-targeted replay, CLI fixture cap, and loopback enforcement before connected replay.
- `crates/capo-cli/src/main.rs` - CLI routing for server session start and server adapter replay.
- `crates/capo-cli/src/cli_surface.rs` - help text marking direct adapter replay as compatibility bypass and documenting server replay by session ID.
- `crates/capo-cli/tests/server_transport.rs` - process-level running-server Codex replay through register, session start, replay, dashboard, restart, recovery, and status.
- `crates/capo-adapters/fixtures/codex-exec.jsonl` - deterministic Codex JSONL replay input.
- `crates/capo-adapters/fixtures/acp-replay.jsonl` - deterministic ACP-shaped replay input.

## SV6 Implementation Sources

- `crates/capo-server/src/lib.rs` - typed dispatch plan/gate/run-local commands, dispatch projections, deterministic runtime metadata, server-native stream ingress, idempotent execution request behavior, and dashboard dispatch metadata.
- `crates/capo-server/src/transport.rs` - transport encoding/decoding for dispatch plan/gate/run-local requests and responses.
- `crates/capo-server/src/tests.rs` - deterministic server-level dispatch plan -> gate -> run-local -> idempotent repeat proof.
- `crates/capo-cli/src/server_client.rs` - `capo server dispatch plan|gate|run-local` CLI client commands and dispatch status rendering.
- `crates/capo-cli/src/main.rs` - CLI routing for server dispatch commands.
- `crates/capo-cli/src/cli_surface.rs` - help text for server dispatch commands.
- `crates/capo-cli/tests/server_transport.rs` - process-level running-server dispatch proof through restart/recovery.
- `crates/capo-adapters/fixtures/codex-exec.jsonl` - deterministic Codex-shaped runtime output used as mocked run-local output.

## SV7 Implementation Sources

- `crates/capo-server/src/lib.rs` - `CapoServer` struct, public module exports, and top-level server command router.
- `crates/capo-server/src/types.rs` - server request/response/error/origin types and response summary structs.
- `crates/capo-server/src/dashboard.rs` - dashboard/status summary projection helpers.
- `crates/capo-server/src/dispatch.rs` - dispatch plan/gate/run-local helpers, dispatch projections, and dispatch replay/execution handling.
- `crates/capo-server/src/server_core.rs` - shared command envelope, response, run lookup, and request audit helpers.
- `crates/capo-server/src/transport.rs` - loopback TCP transport, request frame bounds, and request/response round trip.
- `crates/capo-server/src/transport/wire.rs` - transport JSON wire conversion helpers and error serialization.
- `crates/capo-server/src/util.rs` - adapter parsing, adapter/provider labels, stable hashing, command identity hashing, and small shared utilities.
- `crates/capo-server/src/tests.rs` - current server-level regression suite; retained as one test file in SV7.

## SV8 Review And Fix Sources

- xhigh review subagent - found server transport loopback boundary and adapter/session mismatch must-fixes before live-provider execution.
- xhigh planning subagent - attempted next-slice planning after SV8 fixes but stalled and was closed without a recommendation.
- `crates/capo-server/src/transport.rs` - `serve_tcp` loopback enforcement at the server crate boundary.
- `crates/capo-server/src/server_core.rs` - `require_session_adapter` helper for session adapter invariants.
- `crates/capo-server/src/lib.rs` - replay and dispatch planning now validate requested adapter against the server session before parsing or mutating events.
- `crates/capo-server/src/types.rs` - `ServerError::AdapterSessionMismatch`.
- `crates/capo-server/src/transport/wire.rs` - transport serialization for adapter/session mismatch errors.
- `crates/capo-server/src/tests.rs` - focused regressions for non-loopback listener rejection and adapter/session mismatch rejection.

## SV9 Implementation Sources

- xhigh review subagent - found live-provider preflight idempotency and raw-goal request ID must-fixes.
- `crates/capo-server/src/types.rs` - `PreflightLiveProvider`, `LiveProviderPreflightSummary`, and dashboard/session preflight summary fields.
- `crates/capo-server/src/lib.rs` - server-owned live-provider preflight validation, projection persistence, safety metadata, policy-value rejection, and authorization-input identity.
- `crates/capo-server/src/dashboard.rs` - latest dispatch gate status/reasons/next action exposed through server dashboard summaries.
- `crates/capo-server/src/transport.rs` - transport encoding/decoding for live-provider preflight commands/responses and server-boundary loopback checks for send/serve.
- `crates/capo-cli/src/server_client.rs` - `capo server dispatch live-preflight` CLI client command and rendering.
- `crates/capo-cli/src/main.rs` - CLI route for server live-provider preflight.
- `crates/capo-cli/src/cli_surface.rs` - help text for server live-provider preflight.
- `crates/capo-server/src/tests.rs` - deterministic server tests for Codex/Claude ready preflight, fail-closed blocked preflight, changed-policy preflight identity, non-raw default request IDs, and non-loopback send rejection.
- `crates/capo-cli/tests/server_transport.rs` - process-level running-server live preflight proof for Codex and Claude.

## SV10 Planning Sources

- xhigh planning subagent - recommended a shared live-provider execution command shape with only Codex enabled first.
- `crates/capo-cli/src/adapter_dispatch_run.rs` - existing CLI-owned live run-local chain used as prior art, not the server product path.
- `crates/capo-adapters/src/local_subscription.rs` - Codex and Claude local launch-plan defaults, subscription safety assertion, env allowlist, and sensitive artifact scanner.
- `crates/capo-runtime/src/lib.rs` - local process runner, bounded output artifacts, redaction, spawn, wait, and timeout behavior.

## SV11 Implementation Sources

- `crates/capo-server/src/live_provider.rs` - server-owned live-provider local-run validation, Codex-only enablement, mock output ingress, real Codex launch wiring, artifact scanning, and path policy.
- `crates/capo-server/src/types.rs` - `RunLiveProviderLocal` server command.
- `crates/capo-server/src/lib.rs` - command router and live-preflight prompt source persistence.
- `crates/capo-server/src/dispatch.rs` - dispatch execution projection helpers and latest execution lookup.
- `crates/capo-server/src/transport.rs` - transport encoding/decoding for live-run requests.
- `crates/capo-cli/src/server_client.rs` - `capo server dispatch live-run-local` client command and output rendering.
- `crates/capo-cli/src/main.rs` - CLI route for server live-run.
- `crates/capo-cli/src/cli_surface.rs` - help text for server live-run.
- `crates/capo-server/src/tests.rs` - server tests for Codex mocked live-run and Claude first-slice blocking.
- `crates/capo-cli/tests/server_transport.rs` - process-level running-server proof for Codex mocked live-run through CLI transport.
- `crates/capo-adapters/fixtures/codex-exec.jsonl` - mocked Codex provider output for automated live-run path tests.
- xhigh SV11 narrow review subagent - returned no must-fix findings and suggested symlink/path and repeat-run event-count coverage.

## SV12 Split Sources

- xhigh SV12 planning subagent - recommended behavior-preserving server/CLI split before manual Codex smoke.
- `crates/capo-server/src/tests.rs` - reduced to a test module root.
- `crates/capo-server/src/tests/foundation.rs` - server foundation and mocked-agent tests.
- `crates/capo-server/src/tests/replay.rs` - Codex/ACP replay and adapter/session mismatch tests.
- `crates/capo-server/src/tests/live_provider.rs` - live-provider preflight and mocked live-run tests.
- `crates/capo-server/src/tests/dispatch.rs` - deterministic dispatch plan/gate/run-local tests.
- `crates/capo-server/src/tests/sessions.rs` - server-native session and fixture cap tests.
- `crates/capo-server/src/tests/transport.rs` - TCP transport and restart/recovery tests.
- `crates/capo-cli/src/server_client/dispatch.rs` - server dispatch CLI handlers split from the server client facade.
- `crates/capo-server/src/transport.rs` - loopback TCP runtime, frame limits, and public transport API after codec split.
- `crates/capo-server/src/transport/codec.rs` - JSON request/response command and payload codec split from transport runtime.
- `crates/capo-server/src/transport/wire.rs` - low-level wire parsing and transport error mapping helpers.
- `crates/capo-cli/tests/server_transport.rs` - process-level server transport test module root.
- `crates/capo-cli/tests/server_transport/basic.rs` - mocked-agent loopback server process scenario.
- `crates/capo-cli/tests/server_transport/replay.rs` - Codex replay through running server process scenario.
- `crates/capo-cli/tests/server_transport/dispatch.rs` - deterministic dispatch through running server process scenario.
- `crates/capo-cli/tests/server_transport/live.rs` - live-provider preflight and mocked live-run process scenarios.
- `crates/capo-cli/tests/server_transport/support.rs` - shared CLI process test helpers.
- `crates/capo-server/src/live_provider.rs` - live-provider preflight and local live-run command handling.
- `crates/capo-server/src/server_core.rs` - shared server command helpers and recovery command handling.

## SV13 Real Codex Smoke Sources

- xhigh SV13 planning subagent - recommended the loopback-only manual smoke command sequence and flagged raw-goal persistence as a must-fix.
- `/tmp/capo-sv13-state-1779902046` - final manual real-Codex smoke SQLite state root.
- `/tmp/capo-sv13-artifacts-1779902046` - final manual real-Codex stdout/stderr artifact root.
- `/tmp/capo-sv13-logs-1779902046` - final manual real-Codex CLI transcript logs.
- `crates/capo-server/src/lib.rs` - server-native session start now stores hash-only goal references.
- `crates/capo-server/src/dispatch.rs` - dispatch run-exit and replay metadata helpers now distinguish mocked fixture ingest from real provider execution.
- `crates/capo-server/src/live_provider.rs` - real Codex execution path records live-provider replay/run metadata with `provider_cli_executed=true`.
- `crates/capo-server/src/tests/sessions.rs` - regression that server-native session start does not persist raw goals.
- `crates/capo-server/src/tests/live_provider.rs` - regressions for mocked live-run metadata, mock-vs-real run-exit audit event IDs, and Codex/Claude live-provider gates.
- xhigh SV13 review subagent - found stale-goal and raw request-id must-fixes before SV13 closeout.
- xhigh SV13 final closeout subagent - found plan-scoped run-exit event ID collision risk after idempotency metadata was fixed.
- `crates/capo-server/src/types.rs` - default request IDs now hash `StartSession` goals instead of slugging raw goal text.
