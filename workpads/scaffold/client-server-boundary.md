# Client/Server Boundary Alignment

## Objective

Clarify what belongs in the Capo server/controller/query surface versus the local `capo` CLI client.

Capo is the control plane. The local CLI is one client for inspecting and steering tracked agents. Future clients should be able to reuse the same controller commands and query/read models without copying CLI-specific state logic.

## Boundary Rule

```text
CLI / voice / dashboard / remote clients
  parse user input
  submit CommandEnvelope or query requests
  render returned read models

Controller / query / state / tools
  own task/session/agent/run transitions
  own dispatch, adapter events, permissions, memory, tool activity
  own persisted state and derived read models
```

The CLI may compose narrow workflows while the scaffold is local-only, but those workflows should be treated as client convenience unless they mutate controller state. Durable product semantics belong behind controller/query APIs.

## Current Shape

### Already Aligned

- `CommandEnvelope` is used by core write paths such as init, agent register/spawn, task send, redirect, interrupt/stop, workpad/project-memory import, and proposal/apply commands.
- `FakeBoundaryController` owns the fake orchestration loop, recovery, adapter replay, scripted mock replay, local dispatch planning, and state mutation helpers.
- `capo-query` owns shared dashboard, dogfood readiness, adapter gate/status, runtime target status, and workpad/source-task selection logic.
- `capo-tools` owns governed Capo/wrapper tool definitions and permission checks.
- The new `capo project memory ...` surface is a CLI client alias over the markdown source adapter, not a new product domain.

### Still Too CLI-Coupled

- `crates/capo-cli/src/main.rs` is a large hand-written dispatcher. It is acceptable for prototype routing, but adding many more product concepts here will make the CLI the accidental API.
- `crates/capo-cli/src/workpad.rs` owns too much workflow composition: source indexing, next selection, import, start-next dispatch, dispatch prompt planning, proposal rendering, and compatibility output.
- `crates/capo-cli/src/project_memory.rs` is currently a thin alias layer. That is fine short-term, but product-memory behavior should move to controller/query functions before more clients need it.
- `crates/capo-cli/src/dashboard.rs`, `dogfood.rs`, `voice.rs`, and `voice_render.rs` still render workpad-specific fields directly. These should consume product-language query aliases before UI/client expansion.
- `adapter_dispatch_prepare.rs` and `adapter_dispatch_run.rs` materialize workpad prompt sources by reading workpad projections directly. This should eventually use `SourceBinding` / source-task query APIs.
- `evidence.rs` uses `--follow-up-workpad-task` and review artifact wording. Follow-up task links should move toward `source_task_id` or `task_id`.

## CLI Responsibilities

The local `capo` CLI should keep:

- argument parsing and validation that is specific to terminal UX.
- local state-root selection and local output formatting.
- command submission to controller APIs.
- query submission to shared query APIs.
- compatibility aliases such as `capo workpad ...` while they are still needed.
- local operator-only helpers for explicit opt-in real provider smoke/dispatch.

The CLI should not keep:

- authoritative task/session state decisions.
- source-memory selection policies that future clients must duplicate.
- permission decisions.
- direct provider/runtime execution except through controller/runtime helpers.
- product terminology that future clients inherit accidentally.

## Controller/Query Responsibilities

Move or keep these behind non-CLI APIs:

- agent/session/run lifecycle transitions.
- task/source binding.
- source-task selection.
- project-memory/context packet construction.
- adapter dispatch planning, materialization, preflight, and execution records.
- dogfood/scaffold readiness computations.
- evidence record/export decisions.
- permission/tool authorization and audit events.

## Near-Term Code Movement

The next minimal code movement should not be a large server rewrite. It should be one narrow extraction:

1. Add query/product aliases for project-memory source tasks, backed by existing `WorkpadTaskProjection`.
2. Change CLI project-memory commands to render those aliases as the primary output.
3. Leave `capo workpad ...` and `workpad_*` query fields as compatibility.

This gives future clients a product-language read model without migrating storage or removing compatibility.

## Deferred Server Work

Defer until after scaffold gate:

- persistent daemon/server process.
- HTTP/RPC transport.
- remote CLI client.
- web/mobile UI.
- voice production client.
- push subscriptions/read-model streaming.

These should be built after the controller/query product surface is clear enough that clients do not copy transitional workpad logic.

## File Split Notes

Current file sizes show the main risk areas:

- `crates/capo-cli/src/tests.rs` is very large and should eventually split by command/domain once the scaffold gate stabilizes.
- `crates/capo-cli/src/workpad.rs`, `evidence.rs`, `voice_render.rs`, `adapter_dispatch_prepare.rs`, and `adapter_dispatch_run.rs` are in the 500-740 LOC range and should be monitored.
- `crates/capo-cli/src/main.rs` is under the warning threshold but should remain a router only.

Do not split files just to reduce line counts during scaffold alignment. Split when a boundary extraction removes real coupling.

## Migration Tasks

1. Query aliases for source tasks/project memory.
2. CLI project-memory rendering from product-language query aliases.
3. SourceBinding projection and controller import API.
4. Move project-memory start-next composition behind controller/query helpers.
5. Rename dashboard/dogfood/voice presentation fields.
6. Only then consider hiding/deprecating `capo workpad ...`.
