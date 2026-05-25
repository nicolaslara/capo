# Architecture Tasks

## Objective

Convert research findings into Capo's durable boundary definitions, state/event model, capability model, runtime/tunnel plan, protocol/provider plan, memory architecture, and the thinnest credible prototype plan.

Architecture turns research into contracts and a prototype plan.

## A0 - Research Ingestion

Status: completed

Acceptance:

- Research gate findings summarized in `knowledge.md`.
- Architecture risks and open questions listed.

Evidence:

- `workpads/architecture/knowledge.md` A0 section
- `workpads/architecture/references.md` Research Gate Inputs section

## A1 - Boundary Contracts

Status: completed

Acceptance:

- `boundaries.md` updated with concrete interfaces for input, controller, protocol adapter, runtime, tunnel, provider, capability, state, memory, and evaluation.
- Each interface has responsibilities, non-responsibilities, failure modes, and test strategy.

Evidence:

- `workpads/architecture/boundaries.md`
- `workpads/architecture/knowledge.md` A1 section

## A2 - State Model And Event Log

Status: completed

Acceptance:

- Define entities, event types, and read models for prototype.
- Specify restart recovery behavior.
- Choose SQLite/files layout for prototype.

Evidence:

- `workpads/architecture/state-model.md`
- `workpads/architecture/knowledge.md` A2 section

## A2a - ACP Streaming Replay And Dedupe Research

Status: completed

Context:

- ACP `session/load` can replay prior conversation through `session/update`.
- Capo also needs restart recovery from its own persisted event log.
- Architecture must avoid duplicate dashboard/read-model state when both mechanisms are involved.

Acceptance:

- Research ACP replay semantics, update identifiers, and session/load behavior deeply enough to design Capo's event identity model.
- Define how Capo stores partial streaming updates, chunk finalization, tool-call updates, and replay markers.
- Define dedupe/idempotency rules for ACP replay, Capo restart recovery, and UI projections.
- Record recommendation in `knowledge.md` and any protocol notes in `references.md`.

Evidence:

- `workpads/architecture/acp-replay-dedupe.md`
- `workpads/architecture/state-model.md` adapter replay additions
- `workpads/architecture/knowledge.md` A2a section
- `workpads/architecture/references.md` Protocol section

## A3 - Capability And Permission Model

Status: completed

Acceptance:

- Define capability grants, scopes, approvals, revocation, and audit events.
- Cover shell, filesystem, git, network, browser, MCP/tools, and voice transcript access.
- Start with a trusted local prototype policy that can allow broad local scopes, but define a modular permission-decision interface that can later use static policy, user approval, or a fast security agent.
- Define how ACP permission options such as `allow_once`, `allow_always`, `reject_once`, and `reject_always` map into Capo policy decisions even if the first policy allows everything.

Evidence:

- `workpads/architecture/capability-permissions.md`
- `workpads/architecture/state-model.md` capability/permission additions
- `workpads/architecture/knowledge.md` A3 section
- `workpads/architecture/references.md` ACP permission references

## A4 - Runtime And Tunnel Plan

Status: completed

Acceptance:

- Define local runtime interface.
- Define remote runtime/tunnel abstraction.
- Choose prototype runtime and defer list.

Evidence:

- `workpads/architecture/runtime-tunnel.md`
- `workpads/architecture/state-model.md` runtime/connectivity additions
- `workpads/architecture/knowledge.md` A4 section
- `workpads/architecture/references.md` Runtime And Connectivity section

## A5 - Protocol And Provider Plan

Status: completed

Acceptance:

- Decide ACP adapter shape.
- Decide first agent/provider connector.
- Record subscription-backed connector policy and risks.
- Treat Claude Code and Codex as the first concrete target adapters.
- Defer Capo-as-ACP-agent/editor-backend mode; Capo remains the user entrypoint for the prototype.

Evidence:

- `workpads/architecture/protocol-provider.md`
- `workpads/architecture/state-model.md` adapter/provider additions
- `workpads/architecture/knowledge.md` A5 section
- `workpads/architecture/references.md` Protocol section local CLI observations

## A5a - Capo Tool Exposure And Instrumentation

Status: completed

Acceptance:

- Define the first small set of Capo-exposed tools.
- Define how Capo wraps existing agent tools so tool calls can be tracked, instrumented, audited, and eventually governed.
- Define how exposed tools relate to ACP client capabilities, MCP servers, local runtime tools, and provider-native tools.
- Record what is in prototype scope and what is deferred.

Evidence:

- `workpads/architecture/tool-exposure.md`
- `workpads/architecture/state-model.md` tool definition/invocation/observation additions
- `workpads/architecture/knowledge.md` A5a section
- `workpads/architecture/references.md` Tool Exposure section

## A6 - Memory Architecture

Status: completed

Acceptance:

- Define v0 memory storage and indexing.
- Define how operational state references memory.
- Define migration path toward layered/fractional memory.

Evidence:

- `workpads/architecture/memory-architecture.md`
- `workpads/architecture/state-model.md` memory record/source/index/packet/job additions
- `workpads/architecture/capability-permissions.md` memory scopes
- `workpads/architecture/knowledge.md` A6 section
- `workpads/architecture/references.md` State And Memory section

## A7 - Prototype Plan

Status: completed

Acceptance:

- Convert architecture into ordered prototype tasks.
- Define e2e smoke test.
- Define dogfood gate prerequisites.

Evidence:

- `workpads/architecture/prototype-plan.md`
- `workpads/prototype/tasks.md`
- `workpads/architecture/knowledge.md` A7 section
- Routing docs updated to load `prototype-plan.md` after A7

## A8 - Architecture Gate Review

Status: pending

Acceptance:

- Review recorded in `knowledge.md`.
- User-sensitive decisions called out.
- Prototype workpad is ready to start.
