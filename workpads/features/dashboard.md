# Dashboard Feature

## Objective

Move dashboard data access into a reusable query surface and build richer operator views without letting UI code own orchestration state.

## Prototype Inputs

- P13 added a text dashboard that reads SQLite projections.
- P15 decided that text dashboard is enough for first dogfood, while richer views can follow.

## Dependencies

- CLI, dashboard, voice, mobile, and web views must render the same read-model/query contract.
- No dashboard view should read live adapter/runtime process state directly.

## Tasks

### DS1 - Query Surface Extraction

Status: pending

Acceptance:

- Extract agent/session/dashboard aggregation from `capo-cli` into a reusable controller or query crate/module.
- Keep output independent from terminal rendering.
- Preserve P12/P13 assertions through existing CLI commands.

### DS2 - Operator Dashboard View

Status: pending

Acceptance:

- Show active agents, sessions, goals, blockers, confidence, evidence refs, tool calls, and memory packet refs.
- Add filtering by project/session/status.
- Keep dashboard rendering read-only.
