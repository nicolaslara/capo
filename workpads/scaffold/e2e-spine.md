# Scaffold E2E Spine

## Objective

Define the narrow product spine Capo should prove before adding more breadth.

Capo is a server/control plane. Clients submit commands and render read models. Tracked agents are represented through ACP-shaped adapter events where possible. Project memory is DB-backed source/context data that points to markdown files and is exposed to agents through tools or context packets.

## Minimum Flow

The next scaffold gate should prove one focused path:

1. A client submits an instruction through the Capo command surface.
2. The controller resolves project, task, agent, session, run, capability profile, and source-memory refs.
3. The controller dispatches to a tracked agent through an adapter boundary.
4. The adapter emits normalized, ACP-shaped events for messages, tool calls, permissions, completion, failure, and interruption.
5. The controller appends durable events and updates SQLite read models.
6. The memory/tool layer exposes the relevant markdown-backed project memory as a small context packet or read-only tool result, with source path, anchor, hash, and purpose recorded.
7. The client can inspect agent/session state, recent events, tool activity, context refs, blockers, confidence, and evidence from read models only.
8. Restart/recovery rebuilds the same read models without duplicate rows.
9. Evidence export writes Capo-owned markdown artifacts without editing source markdown.

This is the core loop. Voice, rich dashboards, remote clients, remote runtimes, source writeback, graph/vector memory, and broad provider automation should wait unless they directly prove this loop.

## Existing Evidence

### Proved Enough To Reuse

- Fake-controller e2e: the prototype can register fake agents, send tasks, track sessions/runs, record tool/memory/evidence refs, interrupt/stop, recover state, and export evidence.
- Scripted mock agent: `ScriptedMockAgent` emits deterministic normalized events and routes through static `AgentAdapter` dispatch; controller tests can replay scripted multi-turn adapter behavior.
- Provider fixture replay: Codex, Claude Code, and ACP fixture streams normalize into adapter events and replay through controller state without retaining raw provider content in read models.
- Bounded Codex proof: a real opt-in Codex dispatch stream has been ingested through the same adapter/controller path with clean artifact scanning.
- Project-memory CLI alias: `capo project memory ...` now gives a product-language surface over the markdown source adapter while `capo workpad ...` remains compatibility scaffolding.

### Not Yet Proved As One Product Spine

- No single deterministic e2e test starts from `capo project memory ...`, binds a markdown-backed source task, dispatches it through a scripted mock or ACP-shaped adapter turn, exposes project memory/context, recovers state, and exports evidence.
- ACP is present as fixture parsing and capability/client-call mapping, but tracked local agents are not yet represented end-to-end as first-class ACP sessions.
- Project memory still uses workpad-named internals: event kinds, projections, dashboard/read-model fields, and tool IDs.
- The Capo CLI is still command-oriented. It is not yet the intended local client agent loop for discussing tracked agent state and steering sessions.
- Some historical docs still use old phase-gate language and workpad-first product phrasing. Treat those as history unless active scaffold docs or `project.md` say otherwise.

## Next Deterministic Proof

Build `S1a - Scripted Project-Memory Dispatch E2E` as the next implementation slice:

1. Index markdown-backed project memory through `capo project memory index`.
2. Select and import a source task through `capo project memory next` and `capo project memory import --source-task`.
3. Register a deterministic scripted mock agent.
4. Dispatch the imported task through controller/adapter state using scripted normalized events that include:
   - an assistant message/update,
   - a project-memory/context tool request,
   - a tool completion/observation,
   - a turn completion.
5. Confirm read models show the tracked agent/session/run, adapter-native tool activity, context/memory refs, current summary/confidence, and task status.
6. Run recovery and confirm rebuilt read models match the expected state without duplicates.
7. Export evidence and confirm source markdown was not modified.

Use the scripted mock path for determinism. The bounded Codex proof remains provider evidence, not the main regression test.

## Naming Rules For New Work

- Product-facing names: project, source task, project memory, context, agent, session, run, turn, tool, dispatch, evidence.
- Transitional names: workpad command, workpad projection, workpad event kind, `capo-workpads`, `capo.workpad_read`.
- New user-facing commands should prefer `capo project memory ...` or a future local client loop command.
- New tests may assert compatibility keys, but the primary assertion names should use product language when testing new surfaces.

## Deferred Breadth

- Voice remains a future client surface over command envelopes and read models.
- Web/mobile/rich dashboard work waits until the command/query spine is cleaner.
- Remote runtime/tunnel adapters wait until local dispatch/recovery semantics are tight.
- Source-writing dogfood waits for reviewed patch artifacts, source-hash validation at apply time, rollback evidence, and explicit confirmation.
- Graph/vector/external memory waits until markdown-backed packets and provenance prove useful in dogfood traces.
- Full ACP implementation waits behind the deterministic scripted/fixture proof and a narrow real adapter session model.

## Gate Criteria

The scaffold alignment gate should not close until:

- `project.md` and workpads route future work toward this spine.
- `capo workpad ...` is no longer the only user-facing way to express project-memory/task context.
- A deterministic e2e test proves the product spine above without provider subscriptions.
- Real provider proof remains opt-in, cleanly scanned, and secondary to deterministic regression coverage.
- Remaining workpad-named internals are explicitly classified as compatibility or scheduled migration work.
