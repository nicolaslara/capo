# Research Tasks

## Objective

Turn the initial Capo product prompt into sourced, reviewable recommendations for ACP fit, prior art, stack choice, subscription-backed connectors, local models, memory systems, runtime/tunnel options, and input surfaces. This workpad should unblock the architecture gate without starting broad implementation.

Research establishes the facts and recommendations needed before committing to the first architecture and prototype.

## R0 - Capture Source Prompt

Status: completed

Acceptance:

- Product objective and desired features captured in `project.md`.
- Workpad structure created.
- Research gate defined.

Evidence:

- Created from user prompt on 2026-05-24.

## R1 - Agent Client Protocol

Status: completed

Questions:

- What does ACP standardize today?
- Which clients, agents, and SDKs exist?
- What boundaries does ACP cover, and what remains Capo-specific?
- How should Capo model sessions, messages, tool calls, permissions, and capabilities relative to ACP?

Acceptance:

- Primary-source summary in `knowledge.md`.
- Links and SDK/version notes in `references.md`.
- Recommendation: direct ACP implementation, adapter, or deferred compatibility.

Evidence:

- `workpads/research/findings/R1-acp.md`
- `workpads/research/knowledge.md` R1 section
- `workpads/research/references.md` ACP section

## R2 - Prior Art: Agent Orchestration

Status: completed

Scope:

- Swarms and similar multi-agent systems.
- Open-source coding-agent harnesses and dashboards.
- Existing task/session/state abstractions worth copying or avoiding.

Acceptance:

- Compare at least 4 projects.
- Record architecture lessons, license notes, and failure modes.
- Recommend which ideas Capo should adopt or reject.

Evidence:

- `workpads/research/findings/R2-prior-art.md` compares 9 projects.
- `workpads/research/knowledge.md` R2 section
- `workpads/research/references.md` Prior Art section

## R2a - Prior Art Code Architecture Follow-Up

Status: completed

Context:

- Initial R2 used primary docs and repo metadata.
- We also need to inspect actual source architecture for the closest products.
- Capo should not introduce its own user-facing modes; modes belong to subagents/adapters when present.

Acceptance:

- Inspect code architecture for OpenAI Codex, Cline, OpenHands, OpenCode, and Aider where feasible.
- Record module boundaries, event/session/state model, runtime/process handling, tool/permission handling, adapters/providers, UI/controller split, and persistence/checkpointing.
- Record what Capo should adopt or reject.
- Include source paths, commit/hash/date observed, and license notes.

Evidence:

- `workpads/research/findings/R2-code-architecture.md`

## R3 - Subscription-Backed Agent Connectors

Status: completed

Questions:

- What is feasible for products like ChatGPT Pro and Claude Code Max?
- Which integrations are supported, tolerated, brittle, or disallowed?
- How should Capo isolate browser/session credentials?
- What are the audit and revocation requirements?

Acceptance:

- Feasibility matrix with risk levels.
- Security boundary proposal.
- Explicit unknowns and product/legal caveats.

Evidence:

- `workpads/research/findings/R3-subscriptions.md`
- `workpads/research/knowledge.md` R3 section
- `workpads/research/references.md` Subscription Connectors section

## R4 - Stack Choice: Rust, Python, Or Hybrid

Status: completed

Questions:

- Which parts should be Rust by default?
- Which parts benefit materially from Python libraries?
- What IPC/plugin boundary is acceptable for mixed-language components?

Acceptance:

- Recommendation for prototype stack.
- Dependency candidates and license notes.
- Build/test implications recorded.

Evidence:

- `workpads/research/findings/R4-R6-stack-runtime.md`
- `workpads/research/knowledge.md` R4 section
- `workpads/research/references.md` Stack, Runtime, Tunnel, Sandboxing section

## R5 - Memory Systems

Status: completed

Scope:

- Markdown/file-backed memory baseline.
- SQLite/event-log memory baseline.
- Tana, Zep/Graphiti, mem0, Letta, Capacities, and adjacent systems.

Acceptance:

- Shortlist for v0 and v1.
- Data ownership and export story.
- Recommendation for layered/fractional memory direction.

Evidence:

- `workpads/research/findings/R5-memory.md`
- `workpads/research/knowledge.md` R5 section
- `workpads/research/references.md` Memory section

## R6 - Runtime And Tunnel Options

Status: completed

Scope:

- Local machine execution.
- Cloud VM/devbox execution.
- Tailscale, SSH, reverse tunnels, and remote daemon patterns.
- Capability and filesystem sandboxing.

Acceptance:

- Runtime/tunnel matrix with security and operational tradeoffs.
- Recommendation for prototype and near-term v1.

Evidence:

- `workpads/research/findings/R4-R6-stack-runtime.md`
- `workpads/research/knowledge.md` R6 section
- `workpads/research/references.md` Stack, Runtime, Tunnel, Sandboxing section

## R7 - Input Surfaces: CLI, Dashboard, Mobile, Voice

Status: completed

Questions:

- What is the first usable control surface?
- How should voice commands be represented in the same command model as text?
- What dashboard state is required for dogfooding?

Acceptance:

- Input-surface sequence recommendation.
- Voice pipeline options and privacy notes.
- Dashboard minimum state list.

Evidence:

- `workpads/research/findings/R7-input-surfaces.md`
- `workpads/research/knowledge.md` R7 section
- `workpads/research/references.md` Input Surfaces section

## R8 - Research Gate Review

Status: completed

Acceptance:

- `knowledge.md` contains research gate decision.
- Open questions are listed with owners or defer decisions.
- Architecture workpad has enough evidence to start.

Evidence:

- Research gate passed in `workpads/research/knowledge.md`.
- Architecture inputs and open questions recorded in `workpads/research/knowledge.md`.
- `TASKS.md` active queue advanced to `architecture`.
