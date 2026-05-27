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
