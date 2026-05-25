# Agent Connectors Feature

## Objective

Prove that Capo can safely dispatch work to real local subscription-backed coding agents, starting with Codex and then Claude Code, without reading or persisting credential material.

## Prototype Inputs

- P6 parsed Codex, Claude Code, and ACP fixtures into normalized adapter events.
- P7 built restrictive local smoke plans and artifact scanning, but real provider smoke tests were not run.
- P12 proved the controller/evidence path with fake agents.

## Dependencies

- Use `LocalProcessRunner`; do not spawn provider CLIs directly from adapter code.
- Keep subscription connectors local-only and user-owned.
- Preserve read-model ownership: provider streams are adapter inputs, not controller truth.

## Tasks

### AC1 - Codex Opt-In Smoke

Status: pending

Acceptance:

- Run `CAPO_RUN_CODEX_LOCAL_SMOKE=1 cargo test -p capo-adapters local_codex_adapter_smoke -- --ignored --nocapture` only after explicit user opt-in.
- Use restrictive defaults: isolated workspace, read-only sandbox, ephemeral mode, ignored user config/rules, no provider-native write/network tools.
- Scan stdout/stderr artifacts and state/evidence trees for credential/session markers.
- Record whether the local Codex connector is safe enough for first dogfood.

### AC2 - Claude Code Restricted Args Verification

Status: pending

Acceptance:

- Verify installed Claude Code CLI restricted permission/tool arguments before running a subscription-backed smoke.
- Keep empty MCP config and disallowed tools unless the user explicitly scopes more access.
- Record unsupported or drifting CLI args as a compatibility issue, not a product failure.

### AC3 - Real-Agent Controller Path

Status: pending

Acceptance:

- Route at least one successful real local adapter event stream through Capo state/read models.
- Export markdown evidence with no credential material.
- Keep fake fixtures available as deterministic regression tests.
