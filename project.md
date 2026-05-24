# Capo

## Goal

Build **Capo**, a modular controller and harness for managing a set of coding LLM agents that humans can interact with through text, voice, mobile, dashboards, and other input surfaces.

Capo should make agent work observable, steerable, resumable, and portable across execution environments and model providers. The product should eventually be stable enough to dogfood: Capo should orchestrate the work on Capo itself.

## Name

**Capo**: the controller that directs agents, allocates authority, tracks state, and keeps the organization coherent.

## Source Of Truth

This project setup is based on the initial user prompt from 2026-05-24. When there is conflict between early generated docs and that prompt, preserve the prompt's intent and update the docs.

## Product Thesis

Coding agents are becoming powerful but fragmented across local CLIs, subscription products, APIs, cloud machines, local models, memory stores, and network boundaries. Capo should be the control plane that separates those concerns cleanly:

- How a human gives intent to Capo
- How Capo plans, delegates, and tracks work
- Where agents execute
- How connectivity reaches those machines
- Which model/provider/subscription backs an agent
- What capabilities an agent has
- How state, summaries, memory, and evidence are stored
- How results are reviewed and analyzed

## Desired Features

| Area | Desired capability |
| --- | --- |
| Agent lifecycle | Spawn, stop, pause, resume, inspect, and track multiple coding agents |
| Interaction | Chat with agents, steer goals, inject context, interrupt work, and review outputs |
| Capability control | Change agent tools, filesystem scope, network scope, model/provider, and permission profile |
| Introspection | See active goals, summaries, plans, recent actions, tool calls, blockers, confidence, and evidence |
| State tracking | Persist agent sessions, tasks, goals, workpads, summaries, artifacts, decisions, and review status |
| Remote control | Run Capo as a server and connect from multiple clients or locations |
| Dashboard | Visualize active agents, queue health, task state, costs, failures, and review needs |
| Voice input | Voice commands and dictation as first-class input methods |
| Mobile input | Mobile-friendly control and monitoring surface |
| Text input | CLI/TUI/web text control surfaces |
| Performance analysis | Analyze whether agents completed useful work, how long tasks took, where they failed, and what review found |
| Subscription connectors | Connect through products like ChatGPT Pro and Claude Code Max, not only API keys |
| API-key connectors | Support API-key based model providers where appropriate |
| Local models | Support actually local models and local-network/API-served models |
| ACP compatibility | Build on or interoperate with Agent Client Protocol where it fits the client/agent boundary |
| Memory | Start simple with markdown files or a database pointing to markdown; evolve toward a layered/fractional memory system |
| Memory integrations | Research Tana, Zep/Graphiti, mem0, Letta, Capacities, and similar systems |
| Workpads | Use project workpads as the initial planning and execution substrate |
| Dogfooding | After prototype stability, move Capo's own project execution into Capo |

## Boundary Model

Capo should keep these boundaries explicit and swappable:

| Boundary | Responsibility | Examples |
| --- | --- | --- |
| Input surfaces | Capture human intent and present state | CLI, TUI, web, mobile, voice |
| Capo controller | Own orchestration policy, task routing, permissions, state transitions | Scheduler, session manager, review gate |
| Agent protocol | Normalize interaction between controller/client and agents | ACP, custom adapters |
| Agent runtime | Execute coding agents in a controlled environment | Local machine, cloud VM, container, remote dev box |
| Connectivity/tunnel | Reach execution environments securely | Tailscale, SSH, reverse tunnel |
| Model/provider | Supply model intelligence | Subscription products, APIs, local models |
| Capability layer | Define what agents may do | Shell, git, browser, filesystem, MCP/tools |
| State store | Persist operational truth | SQLite/Postgres, files, event log |
| Memory layer | Persist distilled knowledge and reusable context | Markdown, vector DB, graph memory, external memory apps |
| Evaluation | Measure task quality and agent performance | Review findings, test evidence, outcome scoring |

## Stack Direction

Favor **Rust** for the durable controller, protocol, state, and safety-critical boundaries. Use **Python** when ecosystem leverage is decisive, especially for local-model, voice, memory, or research adapters. Mixed-language architecture is allowed when the boundary is explicit and testable.

Initial assumption:

- Rust core/controller and persistent service
- SQLite first for local state, with a path to Postgres if server mode needs it
- Markdown workpads for human-readable project state
- ACP-compatible adapters where possible
- Python sidecars only where they reduce risk or accelerate integrations

## Phases

| # | Phase | Workpad | Outcome |
| --- | --- | --- | --- |
| 1 | Research | `workpads/research/` | Prior art, ACP, stack, memory, tunnel, subscription connector, and local-model recommendations |
| 2 | Architecture | `workpads/architecture/` | Boundary definitions, data model, adapter contracts, security model, and prototype plan |
| 3 | Prototype | `workpads/prototype/` | Minimal e2e product for spawning, tracking, and steering at least one coding agent |
| 4 | Features | `workpads/features/` | Feature-specific workpads derived from the architecture |
| 5 | Dogfood | `workpads/dogfood/` | Capo manages its own workpads and agent work |

## Workflow

- [`TASKS.md`](./TASKS.md) - active workpad queue
- [`WORKING.md`](./WORKING.md) - agent loop
- [`workpads/WORKPADS.md`](./workpads/WORKPADS.md) - load lists per workpad
- [`AGENTS.md`](./AGENTS.md) - agent rules

## Initial References

| Resource | URL | Notes |
| --- | --- | --- |
| Agent Client Protocol | https://github.com/agentclientprotocol/agent-client-protocol | Protocol for connecting editors/clients and coding agents; current stable protocol version observed as `1` on 2026-05-24 |
| ACP docs | https://agentclientprotocol.com/ | Protocol docs and integration lists |
| Swarms | https://github.com/kyegomez/swarms | Prior-art multi-agent framework to review |
| Zep / Graphiti | https://github.com/getzep/graphiti | Prior-art memory graph system to review |
| mem0 | https://github.com/mem0ai/mem0 | Prior-art memory layer to review |
| Letta | https://github.com/letta-ai/letta | Prior-art stateful agent/memory system to review |

## Global Backlog

- Research ACP schema, SDKs, clients, and agents.
- Research prior-art orchestration systems and extract architecture lessons.
- Decide Rust-only vs Rust-plus-Python split for prototype.
- Define controller state machine and event log.
- Define agent runtime interface and capability profile model.
- Define subscription connector feasibility and security model.
- Define initial memory model.
- Build minimal local agent harness.
- Build first dashboard surface.
- Add voice input spike.
- Add performance analysis and review reports.
- Move project workpads into Capo once the dogfood gate passes.
