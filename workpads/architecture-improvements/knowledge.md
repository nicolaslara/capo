# Architecture Improvements Knowledge

## Objective

Hold architectural findings that workpad-boundary reviews surface but that are
too large to apply inline. The campaign rule: apply review changes immediately
when small or important; defer larger architectural changes here with full
evidence and acceptance criteria.

## Scope Decision

This is a backlog workpad, not a phase in the main DAG. Items are scheduled by
priority and dependency. It exists because the `real-turn-loop` cumulative
review found two foundational "real-but-unwired" gaps (AI1, AI2) that are
important but larger than a boundary edit, and rushing them risked the green
substrate.

## Origin: real-turn-loop cumulative review (2026-05-30)

Three independent adversarial lenses (architecture, correctness, tests)
converged on the same verdict: the RTL substrate (AgentAdapter trait seam,
event-sourced model, resource ceiling, safety floor, orphan reaper) is
well-built and the bulk of the invariants hold, BUT:

- Fixed inline (committed on `feat/real-turn-loop`): the 256-event truncation in
  `reconstruct_turn_finished` (broke replay-identity on long sessions),
  interrupt/stop turn-id event keying + command-path wiring, a safety-gate
  truth-table test, and a live wall-clock-timeout abort test.
- Deferred here (AI1, AI2): the loop's "single orchestration path"
  (`run_dispatch_turn`) and the real provider adapter are both real-but-unwired —
  production chat and dispatch still bypass the loop and resolve real output only
  through a separate stdout-parsing path. These are the crux of the daily-driver
  chat goal.

## Sequencing Note

AI1 and AI2 gate meaningful `goal-autonomy`: continuation must drive the real
production loop with a real adapter, not the fake handle. `streaming-transport`
and `tools-aci` can proceed first because they operate on the authoritative
event log and the tool registry regardless of which path writes the events.
Schedule AI1/AI2 before `goal-autonomy` (or fold them into its prerequisites).

## Non-Goals

- Do not treat this as a dumping ground for nits; only architecture-level items
  with evidence and acceptance belong here.
- Do not defer security/safety-critical fixes here; apply those inline.

## Open Questions

- Should AI1/AI2 run as a dedicated workpad pass right after the
  `streaming-transport || tools-aci` pair, or be merged into `goal-autonomy`'s
  prerequisites?
- Does AI2 need a real `ProviderConnector`/`MemoryBackend` variant, or can the
  Codex `AgentAdapter` wrap the existing live-provider path without them?
