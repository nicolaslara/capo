# Review when you return — open questions + observations

Captured 2026-06-06 before an autonomous execution run. These are **decisions/observations
that need your input** — I did NOT change behavior on them without asking. The autonomous run
itself (the action-bar / detached / reload / tree / steer-delivery items) is logged in
`EXECUTION-SUMMARY.md`.

## A. Inter-agent communication goes via files/text (a workaround) — do you want a structured channel?
Today a fan-out works like: each worker WRITES its result to `result-*.txt` (an on-wire
`fs/write` capo mediates) and narrates it in prose; the conductor then calls `collect_results`
to **read the files back**. That's because the only channel capo can reliably observe *between*
agents is the filesystem — a worker's chat messages aren't a structured value the conductor can
deterministically consume (capo content-hashed agent prose; we made it legible for *display*,
but it's still free text, not a return value).

**It works but it's a hack.** Proposed clean fix (needs your OK): a **`report_result(value)`
capo MCP tool** (or have `start_agent` return the worker's final structured output) so a worker
returns a value to the conductor directly — no file dance, no prose-parsing, and it tidies the
feed. **Decision needed:** add `report_result` / structured worker results? (I left this OUT of
the autonomous run — it changes the orchestration contract.)

## B. Fan-out has no randomness (all workers pick the same thing) — expected; want diversity injection?
"Pick a random fruit/veggie" sent to 3 identical model instances with the same prompt converges
on the **same modal answer** (mango, kohlrabi…). LLMs aren't random; identical inputs → identical
most-likely output, and nothing makes the three differ. This is inherent, not a bug.

**To get variety you must inject the difference:** per-worker seed/constraint (e.g. "pick one
starting with {A|M|Z}", or pass a nonce), tell each worker what's already taken, or have one
agent produce N distinct items. **Decision needed:** should fan-out tasks auto-inject per-worker
diversity when variety is the point? (Cheap conductor-goal tweak; left OUT of the autonomous run
pending your call — it's a product-behavior choice.)

## C. Things the autonomous run DID decide (see EXECUTION-SUMMARY.md for details)
Reasonable defaults I made while you were away — flagged here so you can veto:
- **D-WF1a:** reused `EventKind::ToolCallRequested` for the conductor tool event (no new codec variant).
- **D-WF1b:** arg allowlist + 120-char clip on the emitted conductor tool event (no secret/large args leak).
- **D-WF2a (A4 tree):** the dashboard read model has **no parent field**, so the conductor→worker
  tree is a fork-free FE heuristic — conductor = id/name matches `/conductor/i` (root), all other
  agents are workers (one-level children). If you want a true parent/child lineage in the tree,
  that needs the read model to carry a parent ref (small backend change) — flag it.
- **D-WF2b (A1 buttons):** Steer/Interrupt/Stop POST the real `/api/commands` kinds, but those only
  RECORD INTENT server-side today; the UI says so explicitly. Live delivery is WF3 (B1/B2). No faked
  delivery.
