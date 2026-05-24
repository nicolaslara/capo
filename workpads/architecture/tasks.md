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

## A3 - Capability And Permission Model

Status: pending

Acceptance:

- Define capability grants, scopes, approvals, revocation, and audit events.
- Cover shell, filesystem, git, network, browser, MCP/tools, and voice transcript access.

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
