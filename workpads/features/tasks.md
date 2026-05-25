# Feature Tasks

## Objective

Turn the prototype gate findings into independently executable feature work, while keeping each feature behind explicit dependencies, evidence standards, and review gates.

Feature work starts after the prototype gate. Dedicated feature files hold the detailed backlogs; this file is the routing index.

## Feature Workpads

| Workpad | Focus | First dependency |
| --- | --- | --- |
| `agent-connectors.md` | Real local Codex/Claude connector proof | Prototype P7 harness |
| `dogfood-bridge.md` | Import/index Capo workpads and write reviewed evidence/update artifacts | Prototype P11/P12 evidence export |
| `dashboard.md` | Reusable query surface plus richer TUI/web dashboard | Prototype P13 text dashboard |
| `permissions-tools.md` | Capability profile hardening, approval policy variants, tool wrappers | Prototype P8 audit path |
| `memory-eval.md` | Source-linked memory records, performance reports, review outcomes | Prototype P9/P11 |
| `voice.md` | Conversational Capo loop from P14 contract | Prototype P14 |
| `remote-runtime.md` | Remote runtime/tunnel adapters | Local real-agent semantics |

## F0 - Split Feature Workpads

Status: completed

Acceptance:

- Each selected feature has its own workpad or clearly scoped section.
- Dependencies and gates are recorded.
- Prototype learnings are reflected in the task order.

Evidence:

- `workpads/prototype/knowledge.md` Prototype Gate section
- `workpads/features/agent-connectors.md`
- `workpads/features/dogfood-bridge.md`
- `workpads/features/dashboard.md`
- `workpads/features/permissions-tools.md`
- `workpads/features/memory-eval.md`
- `workpads/features/voice.md`
- `workpads/features/remote-runtime.md`

## F1 - Real Local Agent Connector Proof

Status: pending

Source workpad: `agent-connectors.md`

Acceptance:

- Run the opt-in Codex local smoke through the existing restrictive harness or record why it cannot safely run.
- Verify no credential/session material is read, persisted, or exported.
- Decide whether Claude Code smoke is ready or needs a separate restricted-CLI compatibility slice.

## F2 - Workpad Dogfood Bridge

Status: in_progress

Source workpad: `dogfood-bridge.md`

Acceptance:

- Index/import `TASKS.md`, `project.md`, and workpad files into Capo-readable task records.
- Write Capo-owned evidence/update artifacts without corrupting user-authored markdown.
- Preserve markdown as the source-of-truth fallback.

Progress:

- DB1 workpad index is completed.
- DB2 task import is completed. DB3 reviewed artifacts remain pending.

Evidence:

- `crates/capo-workpads/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `cargo test -p capo-workpads`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- Focused review subagent: blockers found and fixed
- `capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID]`
- `cargo test -p capo-cli workpad_index_imports_markdown_refs_without_modifying_sources`
- Focused DB2 review subagent: source-fingerprint recurrence and task overwrite blockers found and fixed

## F3 - Query Surface And Dashboard Upgrade

Status: pending

Source workpad: `dashboard.md`

Acceptance:

- Extract dashboard/read-model aggregation out of `capo-cli`.
- Keep CLI/dashboard/voice/web consumers on the same query contract.
- Add richer dashboard view only after the query boundary is reusable.

## F4 - Capability And Tool Hardening

Status: pending

Source workpad: `permissions-tools.md`

Acceptance:

- Add stricter policy variants beyond trusted-local allow-all.
- Expand instrumented wrappers for tools Capo can execute directly.
- Keep provider-native tools observed-only unless Capo receives structured lifecycle evidence.

## F5 - Memory And Evaluation Reports

Status: pending

Source workpad: `memory-eval.md`

Acceptance:

- Promote source-linked memory records beyond packet-only evidence.
- Add outcome/performance reports for completed agent work.
- Keep provenance and review state visible in read models.

## F6 - Voice Control Integration

Status: pending

Source workpad: `voice.md`

Acceptance:

- Route P14 voice command plans through the controller/query/permission boundaries.
- Use dummy transcripts until retention/redaction paths are proven.
- Require visible confirmation for privileged voice actions.

## F7 - Remote Runtime And Tunnel

Status: pending

Source workpad: `remote-runtime.md`

Acceptance:

- Add a non-local runtime/tunnel adapter only after local real-agent behavior is stable.
- Keep runtime process ownership separate from connectivity exposure.
- Require explicit permission/audit for public or remote access.
