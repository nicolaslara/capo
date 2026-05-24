# Research Knowledge

## Objective

Establish the facts and recommendations Capo needs before choosing durable architecture: what to build on, what to avoid, which integrations are realistic, and which unknowns require user decisions.

## Initial Facts

- Capo is a controller/harness for managing coding LLM agents.
- Capo should be modular: input, controller, protocol, runtime, tunnel, provider, capabilities, state, memory, and evaluation are separate boundaries.
- Capo should build from research to architecture to e2e prototype, then dogfood itself.
- Favor Rust for durable controller/core work unless research shows Python materially reduces risk for a subsystem.
- Python is acceptable for local models, voice, memory integrations, and experiments.

## Research Gate

Status: not passed.

To pass, record:

- ACP fit and integration recommendation.
- Prior-art lessons from agent orchestration systems.
- Prototype stack recommendation.
- Subscription connector feasibility and security boundary.
- Memory baseline recommendation.
- Runtime/tunnel recommendation.
- Input surface sequence recommendation.
- Top open questions and decisions required from the user.

## Open Questions

- Should the first prototype manage existing CLI agents directly, ACP-compatible agents, or both?
- Which subscription-backed flow is most important first: Claude Code Max, ChatGPT Pro, or another local CLI?
- Is the first dashboard web, TUI, or both?
- How much remote control is needed before dogfooding?
