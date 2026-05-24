# Architecture Knowledge

## Objective

Record the architectural decisions that make Capo modular: each boundary should be explicit enough to implement, test, replace, and review independently.

## Status

Architecture gate not passed.

## Initial Direction

- Keep the controller, agent runtime, connectivity/tunnel, provider connector, state store, memory layer, and input surfaces separate.
- Start with local execution before remote/cloud execution.
- Start with simple durable state and human-readable workpads before advanced memory.
- Build for ACP compatibility, but verify exact protocol fit during research before making it the only agent boundary.

## Architecture Gate

Status: not passed.

Required evidence:

- Boundary contracts.
- State/event model.
- Capability model.
- Runtime/tunnel plan.
- Protocol/provider plan.
- Memory architecture.
- Prototype task plan.

## Open Questions

- Should the core process be a long-running server from day one, or a CLI that later grows a daemon?
- Should the first UI be TUI, web dashboard, or both?
- What is the first concrete agent connector?
