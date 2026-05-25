# Feature Knowledge

## Objective

Capture cross-feature decisions and sequencing until individual feature workpads exist.

Feature phase is ready after prototype gate P15.

## F0 - Split Feature Workpads

Status: completed on 2026-05-25.

Decisions:

- Split the post-prototype backlog by boundary rather than by UI milestone. This keeps real-agent connectors, workpad dogfood, dashboard/query, permissions/tools, memory/eval, voice, and remote runtime independently reviewable.
- The first feature priority should be either `agent-connectors.md` if the goal is real Codex/Claude execution, or `dogfood-bridge.md` if the goal is importing Capo's own workpads before real-agent execution.
- Real local agent execution remains the main product constraint from the prototype gate. Fake agents prove controller/state/evidence semantics, not useful coding output.
- Workpad import/update safety is the main dogfood bridge constraint. Evidence export is safe, but Capo cannot yet manage source workpad files directly.
- Dashboard and voice should share a reusable query surface before adding richer UI or conversational clients.

Follow-up:

- `agent-connectors.md` should start with Codex opt-in smoke because Codex is already wired through restrictive smoke-plan code.
- `dogfood-bridge.md` should preserve the source-of-truth distinction between markdown task status and Capo execution status.
