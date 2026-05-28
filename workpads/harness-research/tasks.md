# Harness Research Tasks

## Objective

Document current best practices for building coding-agent harnesses, compare ACP
against modern harness expectations, and identify what Capo should keep owning
outside the ACP adapter boundary.

## Status

Completed on 2026-05-28.

## Tasks

### HRS-001 - Select comparison set

Status: completed on 2026-05-28.

Acceptance:

- Include ACP as the protocol baseline.
- Include inspectable or documented coding harnesses: Claude Code, OpenCode,
  Codex CLI, Cursor, OpenHands, SWE-agent, SWE-bench, Aider, Cline, Gemini CLI,
  Goose, and Roo Code.
- Separate primary sources from lower-confidence public reports.
- Do not use leaked proprietary source as evidence.

Evidence:

- `workpads/harness-research/references.md`
- `workpads/harness-research/knowledge.md`

### HRS-002 - Answer whether ACP is enough

Status: completed on 2026-05-28.

Acceptance:

- Identify which harness duties ACP covers.
- Identify which harness duties ACP does not cover.
- State a Capo-specific recommendation.

Evidence:

- `workpads/harness-research/knowledge.md`

### HRS-003 - Extract best-known harness practices

Status: completed on 2026-05-28.

Acceptance:

- Summarize recurring patterns across at least five comparable systems.
- Include safety, runtime, tool, memory/context, evaluation, observability, and
  product-surface implications.
- Record follow-up systems worth deeper inspection.

Evidence:

- `workpads/harness-research/knowledge.md`
- `workpads/harness-research/references.md`

## Verification

- Primary and official documentation links are recorded in
  `workpads/harness-research/references.md`.
- The spike intentionally uses Claude Code official docs and public reports only;
  leaked proprietary Claude Code source was not used.
- `git diff --check` is the docs validation gate for this workpad.
