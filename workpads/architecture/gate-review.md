# Architecture Gate Review

## Objective

Decide whether Capo's architecture workpad is stable enough to begin prototype implementation, and record the remaining risks that prototype tasks must prove with code, fixtures, and smoke tests.

## Decision

Architecture gate passes on 2026-05-25.

The architecture is ready for prototype P0 because it defines:

- The core boundaries and naming vocabulary.
- Controller-owned state/event identity.
- ACP replay and dedupe handling.
- Capability and permission policy routing.
- Runtime and connectivity separation.
- Codex, Claude Code, ACP, and fake adapter plan.
- Capo tool exposure and instrumentation model.
- v0 memory packet/provenance model.
- Ordered prototype tasks and an e2e smoke path.

The pass is not a claim that Capo is implemented. It means the docs are sufficiently concrete to scaffold code and let prototype tests refine unclear provider/runtime details.

## Gate Matrix

| Requirement | Evidence | Gate result | Notes |
| --- | --- | --- | --- |
| Research ingested | `knowledge.md` A0, `references.md` research inputs | Pass | Research findings are mapped into architecture direction, risks, and source links. |
| Boundary contracts | `boundaries.md`, `knowledge.md` A1 | Pass | Core boundaries are explicit and use static dispatch for known in-tree variants. |
| State/event model | `state-model.md`, `knowledge.md` A2 | Pass | SQLite/event log, read models, artifacts, recovery, and workpad authority are defined. |
| ACP replay/dedupe | `acp-replay-dedupe.md`, `state-model.md`, `knowledge.md` A2a | Pass with prototype risk | Stable ACP tool-call identity is usable; message-boundary dedupe remains fixture-backed. |
| Capability/permissions | `capability-permissions.md`, `knowledge.md` A3 | Pass | Trusted-local all-allowed policy is allowed only through auditable `PermissionPolicy` routing. |
| Runtime/tunnel | `runtime-tunnel.md`, `knowledge.md` A4 | Pass | Runtime process ownership is separate from tunnel/connectivity; local runtime is first. |
| Protocol/provider | `protocol-provider.md`, `knowledge.md` A5 | Pass with prototype risk | Codex and Claude Code are first targets; exact streams require non-secret fixtures. |
| Tools/instrumentation | `tool-exposure.md`, `knowledge.md` A5a | Pass | First Capo-owned tools and observed-only native tool boundaries are defined. |
| Memory | `memory-architecture.md`, `knowledge.md` A6 | Pass | v0 memory is source-linked markdown/SQLite packet evidence, not an external memory system. |
| Prototype plan | `prototype-plan.md`, `prototype/tasks.md`, `knowledge.md` A7 | Pass | P0-P15 are ordered; P0-P12 form the prototype gate path. |
| Dogfood prerequisites | `prototype-plan.md`, `prototype/spec.md` | Pass | Dogfood remains blocked until prototype smoke proves persistence, inspection, interruption, evidence export, and recovery. |

## User-Sensitive Decisions

- Capo remains the entrypoint/controller for the prototype; Capo-as-ACP-agent/editor-backend is deferred.
- Codex and Claude Code are first concrete adapter targets, with fake adapters used first for deterministic tests.
- Subscription-backed connectors are local-only user-owned integrations; Capo must not read, copy, persist, log, or sync vendor credential material.
- The first permission policy may allow local actions broadly, but every decision must still go through `PermissionPolicy` and durable audit events.
- Capo-exposed tools should be instrumented wrappers when feasible; provider-native and adapter-native tools are `observed_only` until Capo executes or receives structured lifecycle evidence.
- Voice is a future conversational control surface over command envelopes and read models, not just speech-to-text input, but production voice capture is deferred.
- Markdown workpads remain the human-auditable fallback; SQLite owns operational execution state.
- Static dispatch is the default for the first Rust scaffold. Dynamic dispatch/plugin loading is deferred until third-party extension or runtime-loaded adapters require it.

## Residual Prototype Risks

- Codex `exec --json` and Claude Code stream-json field mapping can drift; P6 must capture non-secret fixtures before real adapter claims.
- ACP message replay without stable message IDs remains medium confidence; P10 must include replay/dedupe fixtures before broad ACP compatibility claims.
- Local process execution is not a sandbox; UI/docs must not imply hard isolation until a sandbox/container task proves it.
- All-allowed trusted-local permission policy can hide missing enforcement paths; P8 and P12 must prove audit events, grant use, and result delivery.
- Raw provider/runtime artifacts are fail-closed: persistent artifacts must be classified `safe` or `redacted`; unknown or sensitive raw streams are quarantined or rejected and cannot feed read models/evidence.
- Real Codex/Claude smoke must use restrictive launch defaults before any workspace-write or provider-native tool behavior is attempted.
- `trusted-local-dev` excludes critical scopes such as credential material, raw transcripts, public exposure, remote runtime, external memory sync, and hosted/shared subscription use.
- Dashboard/TUI timing remains a product choice after CLI smoke; P13 records whether it is needed before dogfood.
- Production voice remains deferred until P14 records transcript retention, redaction, and memory-ingestion behavior using dummy inputs.
- Workpad update/export can corrupt human-authored docs if implemented naively; P11 must preserve markdown fallback and evidence references.

## Prototype Start Conditions

Prototype work may begin at P0 when:

- `TASKS.md` marks architecture complete and prototype active.
- `workpads/WORKPADS.md` lists prototype as active.
- `workpads/prototype/tasks.md` remains the executable queue.
- `workpads/architecture/prototype-plan.md` is loaded for prototype work.

## Review Confidence

Confidence: medium-high.

The architecture is coherent enough to implement. Confidence is not high because provider streams, ACP replay fixtures, and runtime process details still need executable tests. Those are appropriately assigned to prototype tasks rather than architecture tasks.
