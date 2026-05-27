# Scaffold Gate Review

Date: 2026-05-26

## Decision

The scaffold is directionally stable enough to keep iterating one task at a time, but it is not yet the final product architecture. The important correction is in place: new work now routes through server/control-plane, tracked-agent, project-memory, query, tool, and evidence concepts instead of treating repository workpads as the product.

The gate passes for continued scaffold implementation with constraints:

- Keep `capo workpad ...` and `workpad_*` storage/query names as compatibility until replacement source-binding read models exist.
- Prefer `capo project memory ...`, `SourceTaskProjection`, `capo.project_memory_read`, task/session/run, controller/query, and evidence language in new code.
- Do not add broad client surfaces such as voice, remote apps, or rich dashboards until the source-binding and tracked-agent spine is cleaner.
- Do not rename SQLite tables or event kinds until product-language query/controller surfaces are proven and covered.

## Real

- The local CLI can act as a client for registering agents, starting tasks, steering sessions, stopping sessions, recovering state, and exporting evidence.
- The controller owns task/session/run state transitions and can replay deterministic scripted mock adapter turns.
- The state layer persists events and read models in SQLite and can rebuild/recover without duplicating tool or memory rows.
- The project-memory CLI surface exists: `capo project memory index|next|plan-next|start-next|import|propose|apply`.
- The deterministic product-spine test starts from `capo project memory ...`, dispatches a markdown-backed source task to a scripted mock agent, records project-memory tool activity, verifies read models, recovers, exports evidence, and leaves source markdown unchanged.
- `capo.project_memory_read` is a governed Capo tool/runtime wrapper alias with static policy coverage and constrained markdown path access.
- `SourceTaskProjection` and `ProjectDashboard::source_tasks|next_source_task|next_source_task_candidate_count` expose product-language query aliases over current workpad projections.
- Real local connector proof exists as bounded, opt-in Codex evidence in the feature workpad history, but it is not the default deterministic test path.

## Stubbed Or Limited

- ACP is represented through adapter/protocol boundaries, fixtures, capability planning, and replay/dedupe tests, but it is not yet the primary live tracked-session execution path.
- The scripted mock agent is deterministic and useful for CI, but it is not a full ACP server/client simulation.
- Project memory is backed by markdown source scanning and compatibility workpad projections. It does not yet have explicit `SourceDocument`, `SourceSection`, `SourceTask`, or `SourceBinding` storage projections.
- Memory packets can reference source-linked context, but the project-memory/source-binding path is still partly hard-coded through compatibility source refs.
- `capo project memory next` now uses a product-language query alias, but `plan-next`, `start-next`, `import`, `propose`, and `apply` still delegate mostly through workpad implementation code.
- Source writeback remains intentionally unsupported except for proposal artifact generation.

## Transitional Compatibility

- `capo workpad ...` remains available and tested.
- `WorkpadTaskProjection`, workpad event kinds, workpad SQLite tables, workpad filters, and workpad dashboard fields remain internal compatibility surfaces.
- `capo.workpad_read` remains available beside `capo.project_memory_read`.
- Voice and dogfood readiness still render workpad vocabulary in places; they are retained because they are existing compatibility/client surfaces, not because voice is scaffold-critical.
- The `capo-workpads` crate remains the markdown source adapter implementation.

## Deferred

- Remote CLI/app clients.
- Voice as an active implementation priority.
- Rich dashboard polish.
- Remote runtime adapters beyond existing runtime/tunnel contracts and tests.
- Graph/vector memory and external memory adapters.
- SQLite/event/table renames.
- Provider-subscription automation beyond explicit opt-in smoke/proof paths.

## Ordered Next Tasks

1. Add an explicit source-binding projection.
   - Goal: represent `Task -> SourceTask/SourceDocument` links without parsing source facts from summaries or compatibility workpad refs.
   - Stop condition: project-memory import/start records and queries can read a binding projection while workpad storage remains compatibility.

2. Move project-memory workflow composition behind reusable controller/query helpers.
   - Goal: make CLI one client over request/query APIs instead of duplicating source-task orchestration in `project_memory.rs` and `workpad.rs`.
   - Stop condition: `capo project memory plan-next|start-next|import` call product-language helpers, with workpad commands as thin compatibility wrappers where practical.

3. Add a deterministic ACP-shaped mock agent connector.
   - Goal: make adapter interaction tests look closer to ACP session traffic while remaining deterministic and provider-free.
   - Stop condition: the narrow product-spine test can run through an ACP-shaped mock connector and still assert the same controller/state/evidence outcomes.

4. Retire workpad vocabulary from shared presentation surfaces.
   - Goal: dashboard/dogfood/voice readouts expose source-task/project-memory names first, with compatibility fields only where needed.
   - Stop condition: new client-facing output does not require `workpad_*` keys for source task selection or context inspection.

5. Revisit real local connector proof.
   - Goal: keep Codex/Claude subscription-backed connectors auditable and opt-in while connecting the proof to the cleaner source-binding and ACP-shaped path.
   - Stop condition: a bounded real-provider proof can be rerun manually without raw secrets/prompts and with evidence linked to the same source-binding model.

## Stop Rule

Do not add voice, remote clients, or broad dashboard work before tasks 1-3 have landed. Those features can reuse the cleaner control plane later; adding them now would increase compatibility debt.
