# Architecture Tasks

## Objective

Convert research findings into Capo's durable boundary definitions, state/event model, capability model, runtime/tunnel plan, protocol/provider plan, memory architecture, and the thinnest credible prototype plan.

Architecture turns research into contracts and a prototype plan.

## A0 - Research Ingestion

Status: pending

Acceptance:

- Research gate findings summarized in `knowledge.md`.
- Architecture risks and open questions listed.

## A1 - Boundary Contracts

Status: pending

Acceptance:

- `boundaries.md` updated with concrete interfaces for input, controller, protocol adapter, runtime, tunnel, provider, capability, state, memory, and evaluation.
- Each interface has responsibilities, non-responsibilities, failure modes, and test strategy.

## A2 - State Model And Event Log

Status: pending

Acceptance:

- Define entities, event types, and read models for prototype.
- Specify restart recovery behavior.
- Choose SQLite/files layout for prototype.

## A2a - ACP Streaming Replay And Dedupe Research

Status: pending

Context:

- ACP `session/load` can replay prior conversation through `session/update`.
- Capo also needs restart recovery from its own persisted event log.
- Architecture must avoid duplicate dashboard/read-model state when both mechanisms are involved.

Acceptance:

- Research ACP replay semantics, update identifiers, and session/load behavior deeply enough to design Capo's event identity model.
- Define how Capo stores partial streaming updates, chunk finalization, tool-call updates, and replay markers.
- Define dedupe/idempotency rules for ACP replay, Capo restart recovery, and UI projections.
- Record recommendation in `knowledge.md` and any protocol notes in `references.md`.

## A3 - Capability And Permission Model

Status: pending

Acceptance:

- Define capability grants, scopes, approvals, revocation, and audit events.
- Cover shell, filesystem, git, network, browser, MCP/tools, and voice transcript access.
- Start with an all-allowed local prototype policy, but define a modular permission-decision interface that can later use static policy, user approval, or a fast security agent.
- Define how ACP permission options such as `allow_once`, `allow_always`, `reject_once`, and `reject_always` map into Capo policy decisions even if the first policy allows everything.

## A4 - Runtime And Tunnel Plan

Status: pending

Acceptance:

- Define local runtime interface.
- Define remote runtime/tunnel abstraction.
- Choose prototype runtime and defer list.

## A5 - Protocol And Provider Plan

Status: pending

Acceptance:

- Decide ACP adapter shape.
- Decide first agent/provider connector.
- Record subscription-backed connector policy and risks.
- Treat Claude Code and Codex as the first concrete target adapters.
- Defer Capo-as-ACP-agent/editor-backend mode; Capo remains the user entrypoint for the prototype.

## A5a - Capo Tool Exposure And Instrumentation

Status: pending

Acceptance:

- Define the first small set of Capo-exposed tools.
- Define how Capo wraps existing agent tools so tool calls can be tracked, instrumented, audited, and eventually governed.
- Define how exposed tools relate to ACP client capabilities, MCP servers, local runtime tools, and provider-native tools.
- Record what is in prototype scope and what is deferred.

## A6 - Memory Architecture

Status: pending

Acceptance:

- Define v0 memory storage and indexing.
- Define how operational state references memory.
- Define migration path toward layered/fractional memory.

## A7 - Prototype Plan

Status: pending

Acceptance:

- Convert architecture into ordered prototype tasks.
- Define e2e smoke test.
- Define dogfood gate prerequisites.

## A8 - Architecture Gate Review

Status: pending

Acceptance:

- Review recorded in `knowledge.md`.
- User-sensitive decisions called out.
- Prototype workpad is ready to start.
