# Prototype Tasks

## Objective

Build and verify the minimal e2e Capo that proves the architecture with a real agent loop, durable state, a control surface, and markdown-compatible evidence.

Prototype work starts after the architecture gate unless explicitly authorized as a spike.

## P0 - Scaffold Decision

Status: pending

Acceptance:

- Rust/Python/hybrid scaffold decision recorded.
- Initial package layout proposed.
- Format/lint/test commands recorded.

## P1 - Core State Store

Status: pending

Acceptance:

- Persist agents, sessions, tasks, events, summaries, and capability grants.
- Restart recovery smoke defined.

## P2 - Local Runtime

Status: pending

Acceptance:

- Spawn and stop one local agent process or adapter.
- Capture normalized events.
- Prevent secret leakage in logs.

## P3 - Controller Loop

Status: pending

Acceptance:

- Create task/session.
- Send message/command to agent.
- Track status and latest summary.
- Interrupt or stop a session.

## P4 - ACP/Adapter Compatibility

Status: pending

Acceptance:

- Implement or stub adapter shape selected by architecture.
- Demonstrate how ACP messages map to Capo state, or record why ACP is deferred.

## P5 - First Control Surface

Status: pending

Acceptance:

- Provide CLI, TUI, or web dashboard for core prototype flows.
- Show active agents, status, current goal, recent events, and blockers.

## P6 - Workpad Persistence

Status: pending

Acceptance:

- Export or update markdown task/evidence artifacts.
- Preserve human-auditable fallback.

## P7 - Prototype E2E Smoke

Status: pending

Acceptance:

- Start Capo.
- Spawn/register one agent.
- Send task.
- Observe status/events/summary.
- Stop/restart Capo.
- Confirm state recovery.

## P8 - Prototype Gate Review

Status: pending

Acceptance:

- Gate result recorded in `knowledge.md`.
- Dogfood readiness gaps listed.
- Feature workpads split from findings.
