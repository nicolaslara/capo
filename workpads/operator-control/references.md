# Operator Control References

## Objective

Track local and external references used for the operator REPL/control surface.

## Local Sources

- `project.md` - product goal for human interaction with coding agents through CLI, dashboard, voice, mobile, and other input surfaces.
- `WORKING.md` - workpad execution, review, and verification rules.
- `.agents/skills/next/SKILL.md` - Codex next workflow for loading active workpads and recording evidence.
- `workpads/server/tasks.md` - completed server/control-plane milestone and server command evidence.
- `workpads/server/knowledge.md` - server command behavior and live Codex proof notes.
- `crates/capo-cli/src/main.rs` - current CLI command routing.
- `crates/capo-cli/src/server_client.rs` - current server-backed CLI client commands.
- `crates/capo-cli/src/operator_control.rs` - no-planner operator control loop and command parser.
- `crates/capo-cli/src/operator_control/planner.rs` - no-planner parser and future planner boundary.
- `crates/capo-cli/src/operator_control/render.rs` - human-readable control output rendering.
- `crates/capo-cli/src/operator_control/server_process.rs` - control-loop server discovery and autostart.
- `crates/capo-server/src/types.rs` - typed server request/response command boundary.
- `crates/capo-server/src/lib.rs` - server command handling, including `SteerAgent`.
- `crates/capo-cli/tests/server_transport/basic.rs` - scripted control-loop integration test against a running server process.
- `README.md` - current human-facing commands for server and operator control usage.
- `workpads/operator-control/planner-boundary.md` - planner mode semantics, future tool surface, safety rules, and audit/display requirements.

## External / Prior-Art Inspirations

- Docker CLI operator command style: concise commands such as list, inspect, attach, exec, logs, and stop are a useful naming/reference point. No Docker dependency is planned for this slice.
