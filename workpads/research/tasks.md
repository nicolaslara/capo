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

Status: pending

Questions:

- What does ACP standardize today?
- Which clients, agents, and SDKs exist?
- What boundaries does ACP cover, and what remains Capo-specific?
- How should Capo model sessions, messages, tool calls, permissions, and capabilities relative to ACP?

Acceptance:

- Primary-source summary in `knowledge.md`.
- Links and SDK/version notes in `references.md`.
- Recommendation: direct ACP implementation, adapter, or deferred compatibility.

## R2 - Prior Art: Agent Orchestration

Status: pending

Scope:

- Swarms and similar multi-agent systems.
- Open-source coding-agent harnesses and dashboards.
- Existing task/session/state abstractions worth copying or avoiding.

Acceptance:

- Compare at least 4 projects.
- Record architecture lessons, license notes, and failure modes.
- Recommend which ideas Capo should adopt or reject.

## R3 - Subscription-Backed Agent Connectors

Status: pending

Questions:

- What is feasible for products like ChatGPT Pro and Claude Code Max?
- Which integrations are supported, tolerated, brittle, or disallowed?
- How should Capo isolate browser/session credentials?
- What are the audit and revocation requirements?

Acceptance:

- Feasibility matrix with risk levels.
- Security boundary proposal.
- Explicit unknowns and product/legal caveats.

## R4 - Stack Choice: Rust, Python, Or Hybrid

Status: pending

Questions:

- Which parts should be Rust by default?
- Which parts benefit materially from Python libraries?
- What IPC/plugin boundary is acceptable for mixed-language components?

Acceptance:

- Recommendation for prototype stack.
- Dependency candidates and license notes.
- Build/test implications recorded.

## R5 - Memory Systems

Status: pending

Scope:

- Markdown/file-backed memory baseline.
- SQLite/event-log memory baseline.
- Tana, Zep/Graphiti, mem0, Letta, Capacities, and adjacent systems.

Acceptance:

- Shortlist for v0 and v1.
- Data ownership and export story.
- Recommendation for layered/fractional memory direction.

## R6 - Runtime And Tunnel Options

Status: pending

Scope:

- Local machine execution.
- Cloud VM/devbox execution.
- Tailscale, SSH, reverse tunnels, and remote daemon patterns.
- Capability and filesystem sandboxing.

Acceptance:

- Runtime/tunnel matrix with security and operational tradeoffs.
- Recommendation for prototype and near-term v1.

## R7 - Input Surfaces: CLI, Dashboard, Mobile, Voice

Status: pending

Questions:

- What is the first usable control surface?
- How should voice commands be represented in the same command model as text?
- What dashboard state is required for dogfooding?

Acceptance:

- Input-surface sequence recommendation.
- Voice pipeline options and privacy notes.
- Dashboard minimum state list.

## R8 - Research Gate Review

Status: pending

Acceptance:

- `knowledge.md` contains research gate decision.
- Open questions are listed with owners or defer decisions.
- Architecture workpad has enough evidence to start.
