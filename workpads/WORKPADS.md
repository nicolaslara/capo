# Active Workpads

> Source of truth for workpad context. Which workpad to run is chosen in [`../TASKS.md`](../TASKS.md).

## Current Focus

| Workpad | Status | Description |
| --- | --- | --- |
| **research** | Complete | Gate passed 2026-05-25 — ACP, prior art, stack, memory, subscription, tunnel, local-model, and voice research |
| **architecture** | Complete | Gate passed 2026-05-25 — boundary definitions, data model, contracts, security model, and prototype plan |
| **prototype** | Complete | Gate passed with constraints 2026-05-25 — local scaffold, fake-agent e2e, dashboard, voice contract, evidence export |
| **features** | Complete | Feature gate passed 2026-05-26 — real connectors, dogfood bridge, dashboard/query, permissions/tools, memory/eval, voice, remote runtime, and maintainability splits |
| **dogfood** | Complete | Dogfood gate passed 2026-05-26 — Capo-assisted development with markdown/git fallback |
| **scaffold** | Complete | Completed 2026-05-26 — product-spine alignment, project-memory aliases, deterministic scaffold proofs |
| **server** | Complete | Completed 2026-05-27 - Capo server/control plane, CLI-through-server path, mocked-agent tests, mocked Codex path, and manual real Codex smoke |
| **harness-research** | Complete | Completed 2026-05-28 - best practices for coding-agent harnesses and ACP's role as adapter boundary |
| **operator-control** | Complete | Completed 2026-05-28 - human operator REPL/control surface, tracked deterministic operator-agent mode, and live-gated Codex control |
| **goal-orchestration** | Design source | Canonical goal-loop DESIGN (GO0-GO14); implementation realized by goal-autonomy + tools-aci on the real-turn-loop substrate; closes as "design realized" |
| **dashboard-webclient** | Complete | Completed 2026-05-28 - first browser dashboard slice with design gate, mocked server-command API, and screenshot-reviewed browser smoke |
| **real-turn-loop** | **Active** | Real observe->decide->emit turn loop + one real workspace-write Codex adapter + provider-neutral AgentAdapter trait + minimal safety floor |
| **streaming-transport** | Planned | Streaming runtime + JSON-RPC framing + event-tail Subscribe + multi-turn thread + interrupt + server-side SSE/HTTP contract (evolves capo-web; not the web client) |
| **tools-aci** | Planned | Wire the real tool path + typed narrow tool I/O + edit/patch/search/test ACI quality + provenance/redaction instrumentation + GO2 reporting tools |
| **safety-gates** | Planned | PermissionPolicy wired into the loop + grant read-back/revoke + real VerificationRunner + checkpoint/rollback + liveness-aware recovery |
| **goal-autonomy** | Planned | Implements the goal-orchestration design: goal/evidence model + continuation scheduler + evidence-gated completion auditor + reattach-after-compaction |
| **depth** | Planned | Live ACP/Claude adapters + real memory packet/FTS5 retrieval + OS sandbox/worktrees + optional OTel; differentiated per-task prerequisites |

## research

**Status:** Complete. Gate passed 2026-05-25. Use as architecture input unless `TASKS.md` reopens research.

**Objective:** Turn the Capo product prompt into sourced recommendations for ACP, prior art, stack, subscription connectors, local models, memory, runtime/tunnel, and input surfaces.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/research/tasks.md
workpads/research/knowledge.md
workpads/research/references.md
```

**Quick nav:**

- `tasks.md` R0-R8: source capture, ACP, prior art, stack, subscriptions, local models, memory, tunnel, voice
- `knowledge.md` Research gate section
- `references.md` primary-source links

**Rules:**

- Prefer upstream docs and repos.
- Record date, license, and maturity where relevant.
- Make recommendations explicit and confidence-scored.
- No broad implementation unless task explicitly authorizes a spike.

## architecture

**Status:** Complete. Architecture gate passed 2026-05-25.

**Prerequisites:** Research gate passed 2026-05-25.

**Objective:** Convert research into durable boundaries, state/event contracts, capability model, runtime/tunnel plan, protocol/provider plan, memory architecture, and prototype plan.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/architecture/tasks.md
workpads/architecture/knowledge.md
workpads/architecture/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/research/knowledge.md
```

**Quick nav:**

- `boundaries.md` system boundary map and initial contracts
- `state-model.md` operational entities, event log, read models, SQLite/files layout, and restart recovery
- `acp-replay-dedupe.md` ACP `session/load`, `session/resume`, streaming, and dedupe design
- `capability-permissions.md` capability profiles, scopes, grants, permissions, revocation, and ACP option mapping
- `runtime-tunnel.md` runtime runners, local process lifecycle, remote runtime abstraction, connectivity/tunnel boundary, and exposure policy
- `protocol-provider.md` Codex, Claude Code, ACP adapter shape, provider connectors, credential scopes, and subscription policy
- `tool-exposure.md` Capo tool registry, wrapper tools, instrumentation, ACP/MCP tool mapping, and observed-only native tools
- `memory-architecture.md` memory records, provenance, indexing, memory packets, and external adapter path
- `prototype-plan.md` ordered implementation sequence, e2e smoke path, and dogfood prerequisites
- `gate-review.md` architecture gate result, user-sensitive decisions, and residual prototype risks
- `tasks.md` A0-A8: event model, capability model, runtime, security, prototype plan
- `knowledge.md` Architecture gate section

**Rules:**

- Keep connectivity/tunnel, execution runtime, controller, provider, input, state, and memory separate.
- Define interfaces before binding to concrete implementations.
- Record explicit user decisions where product direction is needed.

## prototype

**Status:** Complete. Gate passed with constraints 2026-05-25.

**Prerequisites:** Architecture gate passed 2026-05-25 unless explicitly reopened.

**Objective:** Build the smallest e2e Capo that can spawn or register an agent, send work, inspect progress, interrupt execution, persist state, and record evidence.

**Gate result:** The local scaffold is proven with fake agents, SQLite state/recovery, text dashboard, voice contract, Capo tools, memory packet refs, and markdown evidence export. Real subscription-backed connector proof and workpad import/update safety move to feature/dogfood work.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/prototype/spec.md
workpads/prototype/tasks.md
workpads/prototype/knowledge.md
workpads/prototype/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/architecture/knowledge.md
```

**Quick nav:**

- `spec.md` Prototype minimum and MVP v0
- `tasks.md` P0-P15 scaffold through dogfood readiness
- `knowledge.md` Prototype gate section

**Rules:**

- Build the smallest product that can really orchestrate an agent.
- Persist enough state to recover after restart.
- Prefer dogfood usefulness over showcase polish.

## features

**Status:** Complete. Feature gate passed 2026-05-26.

**Prerequisites:** Prototype gate passed with constraints on 2026-05-25.

**Objective:** Split post-prototype product work into independently executable feature workpads with dependencies, evidence standards, and review gates.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/features/tasks.md
workpads/features/knowledge.md
workpads/features/references.md
workpads/features/agent-connectors.md
workpads/features/dogfood-bridge.md
workpads/features/dashboard.md
workpads/features/permissions-tools.md
workpads/features/memory-eval.md
workpads/features/voice.md
workpads/features/remote-runtime.md
workpads/features/state-store.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/prototype/knowledge.md
```

**Rules:**

- Split large features into separate workpads once architecture is stable.
- Each feature needs acceptance criteria, evidence, and review requirements.
- Start with real local agent connector proof or the dogfood bridge, depending on whether the next pass prioritizes actual agent execution or importing Capo's own workpads.

## dogfood

**Status:** Complete. Dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback.

**Prerequisites:** Prototype gate passed and feature gate passed.

**Objective:** Move Capo's own project execution into Capo only after restart recovery, inspection, interruption, and markdown fallback are proven.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/dogfood/tasks.md
workpads/dogfood/knowledge.md
workpads/dogfood/references.md
workpads/prototype/spec.md
workpads/prototype/knowledge.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Rules:**

- Do not migrate project execution into Capo until restart recovery and state inspection are proven.
- Keep file workpads as the human-auditable fallback until Capo is demonstrably reliable.

## scaffold

**Status:** Complete. Scaffold alignment completed 2026-05-26.

**Prerequisites:** Architecture, prototype, feature, and dogfood gates are recorded as passed. The scaffold product-direction correction is complete.

**Objective:** Align the implemented scaffold with the intended product spine: Capo is a server/control plane with clients; tracked agents are represented through ACP-compatible protocol boundaries; project/workpad/task memory is data in Capo's DB pointing to markdown source files; the local CLI is one client for inspecting and steering tracked agents, not the domain model itself.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/scaffold/tasks.md
workpads/scaffold/knowledge.md
workpads/scaffold/references.md
workpads/prototype/spec.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/features/agent-connectors.md
workpads/features/dogfood-bridge.md
workpads/features/tasks.md
workpads/dogfood/knowledge.md
```

**Rules:**

- Prefer product-language surfaces: project, task, memory, context, agent, session, dispatch, evidence.
- Treat `capo workpad ...` as transitional development scaffolding for this repo's markdown files, not the long-term user-facing API.
- Keep voice, mobile, remote clients, rich dashboards, remote runtime adapters, graph/vector memory, and source-writing dogfood deferred or stubbed unless a task proves they are needed for the core spine.
- The next e2e proof should show a client talking to Capo, Capo tracking agents through protocol-shaped events, DB-backed project memory/context exposed to agents, persisted state/recovery, and evidence export.
- Use static dispatch where it keeps boundaries readable; allow a simpler alternative only when it improves naming, testability, and modularity without coupling controller, protocol, runtime, tools, and memory.

## server

**Status:** Complete. Milestone completed 2026-05-27.

**Prerequisites:** Scaffold alignment completed 2026-05-26.

**Objective:** Make Capo run as a server/control plane that owns agent tracking, state, and query behavior while local CLI/client surfaces interact with agents through that server boundary.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/server/tasks.md
workpads/server/knowledge.md
workpads/server/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/scaffold/knowledge.md
```

**Rules:**

- The server/control plane owns controller, state, query, and recovery behavior.
- The CLI is a client of the server boundary, not the owner of agent orchestration.
- Test with deterministic mocked agents first, then prove Codex through the same boundary.
- Keep tunnel/connectivity, runtime execution, protocol adapters, memory, and input surfaces modular.

## harness-research

**Status:** Complete. Research spike completed 2026-05-28.

**Objective:** Document best-known practices for building coding-agent harnesses,
compare modern harnesses and agent products, and answer whether ACP is enough
for Capo's harness architecture.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/harness-research/tasks.md
workpads/harness-research/knowledge.md
workpads/harness-research/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Quick nav:**

- `knowledge.md` executive answer, ACP coverage/gaps, harness practices,
  comparison matrix, and Capo recommendations
- `references.md` dated source links and notes

**Rules:**

- Treat ACP as an adapter/protocol boundary, not the whole harness.
- Do not inspect or rely on leaked proprietary source.
- Prefer primary sources and official docs; label closed-product observations as
  lower confidence.

## operator-control

**Status:** Complete. Completed 2026-05-28.

**Prerequisites:** Server/control-plane milestone completed 2026-05-27.

**Objective:** Give humans an ergonomic operator loop for inspecting and steering running Capo agents through the server boundary, starting with a no-planner command REPL and preserving a path to planner-backed modes.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/operator-control/tasks.md
workpads/operator-control/knowledge.md
workpads/operator-control/references.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
workpads/server/knowledge.md
```

**Rules:**

- The REPL is an input surface/client; it must not own orchestration state.
- `--planner none` is command-driven and deterministic. Future planner modes may call tools/LLMs, but must use the same server/control boundaries.
- Prefer human-readable summaries inspired by operator CLIs; keep machine-ish evidence available through existing commands.
- Do not bypass Capo server commands to mutate agent/session/runtime state.
- Keep live provider execution behind existing explicit opt-in gates.

## goal-orchestration

**Status:** Active. Started after operator-control closure on 2026-05-28.

**Prerequisites:** Server/control-plane milestone completed, harness-research completed, and operator-control sufficiently stable to inspect goal/report/story state through the server boundary.

**Objective:** Make Capo own long-running objectives and the evidence-backed story of execution: goal lifecycle, agent reports, requirement/evidence/review/validation ledgers, event-driven continuation, completion audit, parent/child agent reporting, provider-native goal delegation, and historical execution reports.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/goal-orchestration/tasks.md
workpads/goal-orchestration/knowledge.md
workpads/goal-orchestration/references.md
workpads/harness-research/knowledge.md
workpads/harness-research/references.md
workpads/operator-control/knowledge.md
workpads/server/knowledge.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Quick nav:**

- `tasks.md` ordered implementation slices from schema/design through mocked e2e and delegated Codex goal mode.
- `knowledge.md` scope decision, controller-loop design, agent-reporting semantics, story/report projections, and non-goals.
- `references.md` local architecture and harness-research source links.

**Rules:**

- The Capo server/controller owns the goal lifecycle, continuation policy, evidence ledger, and completion decision.
- Agent reports are structured operational data, not freeform transcript truth. Prose summaries are derived views.
- Provider-native goal modes such as Codex `/goal` are delegated inner loops; Capo still mirrors the objective, observes events, and audits completion.
- Do not add autonomous continuation until goal state, report/evidence records, stop policy, and mocked replay tests exist.
- Historical reports must be rebuildable from events, projections, and artifacts.

## dashboard-webclient

**Status:** Complete. First browser slice completed 2026-05-28 after the user explicitly pulled dashboard work forward.

**Prerequisites:** Server/control-plane milestone complete, shared dashboard/query contract available, operator-control usable as the CLI comparison surface. Rich goal/story views depend on goal-orchestration projections.

**Objective:** Build a browser dashboard/web client for Capo that lets an operator understand projects, agents, sessions, goals, evidence, validation, reviews, and execution history without reading terminal/debug output. The workpad requires explicit design, design review, accepted design, implementation, screenshot review, and iteration until the UI works and looks good.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/dashboard-webclient/tasks.md
workpads/dashboard-webclient/knowledge.md
workpads/dashboard-webclient/references.md
workpads/dashboard-webclient/design.md
workpads/dashboard-webclient/completion-audit.md
workpads/features/dashboard.md
workpads/operator-control/knowledge.md
workpads/goal-orchestration/knowledge.md
workpads/server/knowledge.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/capability-permissions.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/tool-exposure.md
workpads/architecture/memory-architecture.md
workpads/architecture/prototype-plan.md
```

**Quick nav:**

- `tasks.md` completed design-to-implementation task sequence and visual QA gates.
- `design.md` accepted product brief, IA, visual design, and review decision.
- `knowledge.md` design principles, implementation decisions, screenshot loop, and residual risks.
- `completion-audit.md` requirement-by-requirement closure audit.
- `references.md` local source links and visual QA evidence requirements.
- `web/dashboard/` dependency-free static browser client and dev server.

**Rules:**

- The web client is a client/input surface. It must not own controller,
  scheduler, runtime, permission, or provider state.
- Consume server/query/read-model APIs; do not read SQLite, artifacts, or raw
  provider logs directly from frontend code.
- Start with dense, operational UI rather than a marketing/landing page.
- Use screenshots and browser checks as required acceptance evidence.
- Iterate visual design until desktop and mobile layouts are usable, readable,
  and free of obvious overlap, truncation, or broken states.

## real-turn-loop

**Status:** Active. First workpad of the daily-driver harness track (started 2026-05-29).

**Prerequisites:** operator-control complete; the event-sourced SQLite state core; the typed `ServerCommand` boundary; the existing dispatch state machine (`PlanDispatch`/`PreflightLiveProvider`/`GateDispatch`/`RunDispatchLocal`/`RunLiveProviderLocal`).

**Objective:** Replace `FakeBoundaryController` with a genuine controller turn loop that observes normalized adapter events, updates projections, and emits `TurnFinished` while driving the existing dispatch primitives as a single orchestration path; run one real workspace-write Codex adapter end-to-end behind a minimal safety floor; and extract a provider-neutral `AgentAdapter` trait. This is the substrate every later workpad attaches to.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/real-turn-loop/tasks.md
workpads/real-turn-loop/knowledge.md
workpads/real-turn-loop/references.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/boundaries.md
workpads/architecture/state-model.md
workpads/architecture/protocol-provider.md
workpads/architecture/capability-permissions.md
workpads/architecture/tool-exposure.md
workpads/goal-orchestration/knowledge.md
```

**Rules:**

- The loop drives the dispatch primitives; do not create a second/parallel execution pipeline.
- The first real workspace write must be confined, reversible, bounded, and dry-run by default; full enforcement lands in safety-gates.
- Deterministic fake/scripted tests before live providers; live Codex stays behind explicit opt-in gates.
- No task closes on operator self-attestation alone; pair every manual smoke with a deterministic assertion (wire snapshot, exit status, or restart/replay).

## streaming-transport

**Status:** Planned. Depends on real-turn-loop.

**Objective:** Make the interactive loop real: a tokio streaming runtime (incremental output deltas + stdin), JSON-RPC framing with a notification/event variant, a `Subscribe{session_id, from_sequence}` tail over the append-only event log via a broadcast channel, a concurrent serve loop with timeouts and in-band cancel, a multi-turn thread read model, and a typed mid-turn interrupt. Deliver the server-side SSE/HTTP contract the web client consumes by evolving the in-tree `crates/capo-web` bridge.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/streaming-transport/tasks.md
workpads/streaming-transport/knowledge.md
workpads/streaming-transport/references.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/state-model.md
workpads/architecture/acp-replay-dedupe.md
workpads/architecture/boundaries.md
workpads/real-turn-loop/knowledge.md
```

**Rules:**

- Deliver the streaming CONTRACT (schema + wire-snapshot tests), not the browser client; `web/app` and `web/dashboard` are owned by a separate agent.
- Tail the event log by sequence; do not re-introduce the dashboard-poll antipattern.
- Redact on emit; never stream sensitive/unknown-redaction content.
- Document the Dashboard-poll -> Subscribe migration handoff for the web agent.

## tools-aci

**Status:** Planned. Depends on real-turn-loop; runs in parallel with streaming-transport.

**Objective:** Raise the agent-computer interface to daily-driver quality. Wire the real tool path (the registry, runtime wrappers, and path containment already exist but are routed to a fake), extend `ToolDefinition` with input/output schemas plus risk/scope/redaction metadata, give file/search/edit/patch/test tools narrow typed decision-grade output with lint-on-edit, add artifact+provenance+redaction instrumentation, and implement the GO2 agent-reporting/evidence tools that goal-autonomy needs.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/tools-aci/tasks.md
workpads/tools-aci/knowledge.md
workpads/tools-aci/references.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/tool-exposure.md
workpads/goal-orchestration/tasks.md
workpads/real-turn-loop/knowledge.md
```

**Rules:**

- Build on the existing registry/wrappers/containment; do not greenfield a parallel tool system.
- Every governed tool call becomes a durable, provenance-tagged, redaction-aware event.
- The GO2 reporting/evidence tools are agent-published claims, not authoritative truth; correlate with observed events.
- Deterministic tests before live; secrets stripped from artifacts and logs.

## safety-gates

**Status:** Planned. Depends on real-turn-loop, streaming-transport, tools-aci.

**Objective:** Convert built-but-inert safety machinery into real enforcement, sub-phased as enforcement | verification | checkpoint-recovery. Wire `PermissionPolicy` + `ToolExposure` into the live loop with inline permission cards over the stream and ACP `request_permission` handling; add grant read-back, revoke, and expiry; build a `VerificationRunner` that actually runs lint/test and records real exit-status evidence; add controller-owned checkpoint/rollback; and replace mark-all-exited recovery with liveness-aware reattach.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/safety-gates/tasks.md
workpads/safety-gates/knowledge.md
workpads/safety-gates/references.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/capability-permissions.md
workpads/architecture/state-model.md
workpads/real-turn-loop/knowledge.md
workpads/tools-aci/knowledge.md
```

**Rules:**

- Permissions are durable policy, not UI prompts; store requests, decisions, grants, and revocations as events.
- Verification means running checks and recording the real exit status, not trusting `--status passed`.
- Checkpoint/rollback must exist before any broadening of auto-approve or unattended writing.
- Keep enforcement in the controller; clients only render and request.

## goal-autonomy

**Status:** Planned. Depends on real-turn-loop, tools-aci, safety-gates. Implements the goal-orchestration design.

**Objective:** Realize the goal-orchestration design (GO0-GO14) on the now-real substrate. Milestone M1 builds the goal/evidence/report event model, projections, and lifecycle/server/read commands (depends on real-turn-loop + tools-aci). Milestone M2 adds the event-driven safe-boundary continuation scheduler, the evidence-gated completion auditor, and reattach-objective-after-compaction (hard-gated on safety-gates checkpoint/rollback + verification). Closes goal-orchestration as "design realized."

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/goal-autonomy/tasks.md
workpads/goal-autonomy/knowledge.md
workpads/goal-autonomy/references.md
workpads/goal-orchestration/knowledge.md
workpads/goal-orchestration/tasks.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/state-model.md
workpads/real-turn-loop/knowledge.md
workpads/safety-gates/knowledge.md
```

**Rules:**

- goal-orchestration is the design source; cite it, do not duplicate or re-specify it.
- The completion auditor is the only path to goal-complete; agents may propose completion but never assert it.
- No automatic continuation until checkpoint/rollback, verification, stop policy, and mocked replay exist.
- The continuation scheduler lives in the controller, never in a client.

## depth

**Status:** Planned. Differentiated per-task prerequisites (real-turn-loop + tools-aci for ACP/Claude/memory; safety-gates for sandbox; goal-autonomy for worktree-per-goal).

**Objective:** Deepen the harness once the core loop is trustworthy: a live ACP JSON-RPC adapter (with session/load + resume and replay/dedupe), a Claude workspace-write adapter as a second real provider, the real memory packet path (MarkdownMemoryBackend + FTS5 retrieval, killing the hardcoded strings) with extraction/staleness jobs, a first OS sandbox tier (seatbelt/landlock+bwrap), git worktree isolation, and an optional OTel exporter. These tasks deepen rather than unblock.

**Load:**

```text
../TASKS.md
../project.md
../WORKING.md
workpads/depth/tasks.md
workpads/depth/knowledge.md
workpads/depth/references.md
workpads/harness-research/daily-driver-review.md
workpads/architecture/memory-architecture.md
workpads/architecture/runtime-tunnel.md
workpads/architecture/protocol-provider.md
workpads/architecture/acp-replay-dedupe.md
workpads/real-turn-loop/knowledge.md
workpads/safety-gates/knowledge.md
```

**Rules:**

- These tasks deepen the harness; do not let them block or precede the core loop, streaming, tools, or safety work.
- Live ACP/Claude/sandbox work stays behind explicit opt-in gates with deterministic fake/replay tests first.
- Keep runtime, connectivity/tunnel, protocol, and memory boundaries modular and swappable.
- Real memory packets are fractional, sourced, and staleness-tracked; do not dump whole transcripts.

## How To Switch Focus

1. Edit `../TASKS.md` Active Now and queue checkboxes.
2. Record why in this file or the target workpad's `knowledge.md` if the switch changes phase order.
3. Load the new workpad context before selecting a task.
