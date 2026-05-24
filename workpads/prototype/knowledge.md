# Prototype Knowledge

## Objective

Record what the prototype proves, what it fails to prove, and whether it is reliable enough to become the harness for Capo's own work.

## Status

Prototype gate not passed.

## Initial Direction

- Build the smallest e2e loop that can actually orchestrate one coding agent.
- Persist state before adding many input surfaces.
- Keep workpads as the fallback human-readable state until dogfooding is proven.

## Prototype Gate

Status: not passed.

Required evidence:

- Spawn/register an agent.
- Send and interrupt work.
- Inspect status, goal, events, latest summary, and blocker.
- Persist and recover state.
- Record evidence in workpad-like artifact.

## Open Questions

- Which agent adapter is first?
- Which control surface is first?
- How much of ACP should be implemented in prototype rather than stubbed?
