# R2 - Prior Art: Agent Orchestration

Observed: 2026-05-25

Scope: Swarms, multi-agent orchestration frameworks, and coding-agent controller/harness/dashboard systems relevant to Capo's goal of spawning, tracking, steering, persisting, and reviewing coding agents.

## Executive Recommendation

Capo should be a controller and harness, not another general-purpose agent framework.

Adopt:

- A durable event/session model with resumable checkpoints, inspired by LangGraph's thread/checkpoint model and Cline/OpenCode/Codex task sessions.
- Explicit runtime boundaries, inspired by OpenHands' runtime abstraction and Docker/local/remote runtime split.
- Human-in-the-loop permissions and reviewable diffs, as in Cline, OpenCode plan/build modes, Aider's git-first workflow, and Codex sandboxing.
- Controller-owned capability profiles and adapter/subagent-reported state. Capo should not introduce its own user-facing modes.
- Provider-agnostic adapters where possible, but treat subscription-backed CLIs as privileged connectors with isolated credentials.

Reject for v0:

- Framework-first "crew/swarm" abstractions as Capo's core state model. They are useful as plugin workloads, but Capo needs observable controller state, not only agent-role graphs.
- Implicit autonomous collaboration without durable audit trails, cost accounting, stop controls, and per-agent capability scopes.
- Cloud/dashboard features that require enterprise control-plane scope before local e2e dogfooding proves the controller.
- Tightly coupling Capo's product model to MCP, A2A, a specific model SDK, or a specific agent framework.
- Capo-owned "modes" as a product abstraction. Modes belong to the underlying subagent/adapter when that product exposes them; Capo should record, display, and route them without making them the controller model.

Confidence: medium-high. The architectural lessons are well supported by current primary sources, but the market is moving quickly and several systems changed materially between 2025 and 2026.

## Comparison Matrix

| Project | Primary role | Architecture signals | License notes | Capo lesson |
| --- | --- | --- | --- | --- |
| Swarms | Python multi-agent orchestration framework | Provides many swarm/workflow patterns: sequential, concurrent, hierarchical, graph, router, mixture-of-agents, group chat, AOP, MCP, and memory integrations. | Apache-2.0 per GitHub README. | Useful catalog of orchestration patterns, but too framework-heavy for Capo core. Capo should support these patterns above the controller boundary, not embed them as the controller model. |
| OpenHands | AI software-development platform: SDK, CLI, local GUI, cloud, runtimes | Separates SDK, CLI, GUI, cloud, and runtime environments. Runtime docs distinguish Docker, local, remote, and third-party runtimes. | Core repo is MIT except `enterprise/`; cloud/self-hosted enterprise material is source-available / Polyform Free Trial in adjacent cloud repo. | Strongest prior art for Capo's runtime boundary. Adopt explicit runtime adapters, action/event streams, and local GUI/API split; avoid mixing open-core enterprise surface into v0. |
| Cline | Coding agent as SDK, IDE extension, CLI, and Kanban board | Shared agent core across SDK, CLI, IDEs, and Kanban; supports human approval, checkpoints, long-running terminal output handling, multi-agent teams, scheduled agents, messaging connectors, and MCP/plugins. | Apache-2.0 in GitHub repo. | Very close to Capo's product direction. Adopt session/task board concepts, approval gates, checkpoints, cost visibility, and SDK/connector split. Treat Plan/Act as Cline adapter state, not a Capo mode model. |
| OpenCode | Terminal/desktop coding agent | TUI/desktop distribution; built-in `build` full-access agent and `plan` read-only agent; explicit install paths; broad package-manager distribution; plugin/MCP ecosystem. | GitHub repo says MIT; current public website footer says Apache 2.0, so license display is inconsistent and should be rechecked before reuse. | Strong evidence that local-first UX with multiple clients matters. Treat `plan`/`build` as OpenCode adapter state, not a Capo mode model. Do not depend on branding/docs claims without repo verification. |
| OpenAI Codex CLI | Local terminal coding agent | Runs locally, has Rust-heavy implementation, npm/Homebrew/binary installs, IDE and desktop app paths, ChatGPT-plan sign-in plus API-key mode. | Apache-2.0. | Strong proof that Rust is viable for durable local agent harness code. Adopt local-first sandbox/session patterns and support subscription sign-in as a privileged connector. Capo should orchestrate Codex rather than reimplement its inner agent loop initially. |
| Aider | Terminal AI pair programmer | Git-first workflow; repo map for large codebases; automatic commits; lint/test loop; voice-to-code; web chat copy/paste mode for non-API use. | Apache-2.0. | Adopt git-native evidence, repo mapping, lint/test feedback loops, and low-friction CLI ergonomics. Reject automatic commits by default in Capo unless gated by explicit policy. |
| CrewAI | Python multi-agent workflow framework | Separates `Flows` for state/control from `Crews` for autonomous role-based collaboration; includes YAML project scaffold, telemetry, cloud control plane, HITL support. | MIT. | The Flow/Crew separation is a useful warning: keep deterministic controller state separate from autonomous agent collaboration. Capo's controller should look more like Flow than Crew. |
| AutoGen | Multi-agent application framework and Studio GUI | Layered design: Core API for message passing/event-driven agents/distributed runtime; AgentChat for higher-level patterns; Studio GUI and Bench. Now in maintenance mode; users directed to Microsoft Agent Framework. | Repo content CC-BY-4.0, code MIT. | Adopt layered API thinking and benchmark/evaluation surface. Failure lesson: broad agent frameworks can be superseded quickly, so Capo should keep adapters replaceable and avoid betting core state on one framework. |
| LangGraph | Stateful graph runtime for LLM applications | Checkpointed graph state organized into threads; enables HITL, memory, time travel, replay, and fault tolerance; persists node writes within super-steps. | LangGraph project license should be checked directly before code reuse; docs are primary for architecture only here. | Best state-model precedent. Adopt thread/run/checkpoint vocabulary or equivalent; distinguish operational checkpoints from long-term memory. |

## Architecture Lessons

### 1. Controller State Must Be Durable And Queryable

LangGraph's persistence docs make checkpoints a prerequisite for human-in-the-loop, memory, time travel, and fault tolerance. That maps directly to Capo's requirements: inspect, interrupt, resume, and recover after restart.

Capo implication:

- Model `agent_session`, `run`, `event`, `checkpoint`, `task`, `capability_profile`, and `review_status` as first-class state, not incidental logs.
- Persist after each externally meaningful event: user instruction, model message, tool request, tool result, permission decision, file diff, test result, interruption, resume, and final evidence.
- Keep memory separate from execution checkpoints. Checkpoints answer "what happened and how can I resume"; memory answers "what should future agents know."

### 2. Runtime Is A Boundary, Not An Implementation Detail

OpenHands explicitly models runtime environments where agents edit files and run commands, with Docker, local, remote, and third-party runtime options. This is central to Capo because agent execution, filesystem scope, network scope, and credentials have different risk profiles.

Capo implication:

- Define a runtime adapter contract before binding to Docker, local shell, SSH, Tailscale, cloud VM, or hosted sandbox.
- Runtime adapters should declare capabilities: filesystem root, shell access, network policy, browser availability, secret mounts, process supervision, and teardown behavior.
- The controller should never infer permissions from "local vs remote" alone.

### 3. Coding-Agent Harnesses Are More Relevant Than Generic Agent Frameworks

Swarms, CrewAI, AutoGen, and LangGraph are useful for orchestration patterns, but Cline, OpenCode, Codex CLI, Aider, and OpenHands are closer to Capo's first product: coding work in real repos with file edits, shell commands, diffs, tests, and human steering.

Capo implication:

- The first prototype should orchestrate one real coding agent process through a narrow adapter and persist its session state.
- Generic multi-agent "team" support should come after the single-agent harness proves spawn, steer, inspect, interrupt, resume, and evidence recording.
- Agent frameworks can be workloads or plugins later; they should not dictate the core event schema.

### 4. Human Control Needs Capability Profiles And Adapter State

Cline has Plan/Act. OpenCode has `plan` read-only and `build` full-access agents. Aider uses git as a review surface. Codex has local execution and explicit sign-in/API-key modes.

Capo implication:

- Do not create Capo-owned user-facing modes. Capo is the controller.
- Store adapter/subagent-reported mode/state as metadata when the underlying agent exposes it.
- Capabilities should derive from Capo policy and explicit grants, not from the agent's own prompt.
- Every write-capable capability profile needs diff visibility and interrupt/stop controls.

### 5. Dashboard/Board Concepts Are Useful, But Only If Backed By Real Events

Cline Kanban and OpenHands GUI/Cloud show demand for visual task/session management. CrewAI AMP markets a control plane with tracing/observability. The risk is dashboard-first architecture where UI state is more durable than execution truth.

Capo implication:

- Build dashboard state from the event log and session store.
- Minimum dashboard state for dogfooding: active sessions, current goal, adapter/subagent-reported state, runtime, last event, pending permission, diff/test status, blocker, cost/token summary where available, confidence/review status, and stop/resume controls.
- Avoid a dashboard-only queue before the CLI/local controller can recover after restart.

## Failure Modes To Design Against

- Runaway autonomy: multi-agent frameworks can generate subtask cascades without clear stop conditions, budget limits, or ownership.
- Silent failure: background agents may stall on tool errors, permissions, auth prompts, rate limits, or long-running shell output unless the controller records liveness and blockers.
- Permission drift: prompts saying "read-only" are insufficient. Read-only must be enforced by tool/runtime policy.
- State loss: terminal-only agents often keep crucial context in process memory. Capo must persist instructions, actions, tool results, summaries, and checkpoints externally.
- Credential leakage: subscription-backed agents and messaging connectors can expose OAuth/browser/session state. Treat them as privileged connectors with isolated stores, redacted logs, and revocation paths.
- Unreviewable edits: automatic edits or commits can hide bad changes. Diffs, tests, and human review status should be first-class.
- Framework churn: AutoGen's move to maintenance mode is a concrete example. Capo needs adapter boundaries so replacement of agent frameworks or provider SDKs does not rewrite controller state.
- License/product ambiguity: OpenHands mixes MIT core with source-available enterprise code; OpenCode's website/repo license labels conflict. Verify before vendoring or copying code.
- Observability tax: broad control-plane products promise tracing, metrics, and dashboards, but v0 should record the small event set needed for dogfooding before adopting large observability stacks.

## What Capo Should Adopt

- Event-sourced or append-first local state with SQLite as the likely first durable store, plus markdown exports for human workpads.
- Runtime adapters with explicit capability declarations and lifecycle controls.
- Agent adapters that can wrap external CLIs first: Codex, OpenCode, Cline CLI, Aider, Claude Code, and ACP-compatible agents where available.
- Capability profiles that can enforce read-only, write-capable, review, or background-like behavior without making these Capo product modes.
- Adapter state capture for subagent modes when a wrapped agent exposes them.
- Session summaries and checkpoints that survive process restart.
- Cost/token accounting where the underlying agent exposes it; nullable otherwise.
- Board/dashboard state derived from stored events.
- Benchmark/review hooks later, inspired by AutoGen Bench and OpenHands evaluation work, but not as a v0 dependency.

## What Capo Should Reject Or Defer

- Rebuilding a full coding agent from scratch before proving orchestration of existing agents.
- Making Swarms/CrewAI/LangGraph/AutoGen the core Capo runtime.
- Treating MCP servers as the central architecture. MCP can be one tool/capability transport, but Capo's core is CLI/controller/runtime state.
- Browser-session automation for subscription products until the security and terms-of-service boundary is researched in R3.
- Autonomous multi-agent swarms before a single-agent local harness has durable state, permissions, stop/resume, and review evidence.
- Enterprise cloud control plane before local dogfood.

## Project Notes

### Swarms

Primary source: https://github.com/kyegomez/swarms

Current facts:

- Positions itself as an enterprise-grade multi-agent orchestration framework.
- Offers many prebuilt patterns: sequential, concurrent, hierarchical, graph workflows, routers, mixture-of-agents, group chat, AOP, MCP, memory, and coding-assistant guidance files.
- License: Apache-2.0.

Capo read:

- Good as a pattern library and possible plugin workload.
- Too broad and Python-framework-centric to be Capo's controller foundation.
- Failure risk: orchestration pattern proliferation can make it hard to understand which state is authoritative.

### OpenHands

Primary sources:

- https://github.com/All-Hands-AI/OpenHands
- https://docs.openhands.dev/openhands/usage/runtimes/overview
- https://github.com/All-Hands-AI/OpenHands-Cloud

Current facts:

- Provides SDK, CLI, local GUI with REST API/React SPA, cloud, and enterprise product surfaces.
- Runtime docs define Docker runtime, local runtime, remote runtime, and third-party runtimes.
- Core license: MIT except `enterprise/`; OpenHands Cloud repo uses Polyform Free Trial and is not open source.

Capo read:

- Strong precedent for separating runtime, UI, SDK, cloud, and issue-resolver surfaces.
- Adopt runtime adapter concepts and event/action separation.
- Avoid copying cloud/enterprise structure into v0.

### Cline

Primary source: https://github.com/cline/cline

Current facts:

- Shared agent engine across SDK, CLI, VS Code, JetBrains, and Kanban.
- Kanban runs many agents from a web task board; each card gets a worktree, auto-commit, and dependency chains.
- Supports Plan/Act, approval gates, checkpoints, terminal output watching, plugins/MCP, multi-agent teams, scheduled agents, messaging connectors, and headless JSON output.
- License: Apache-2.0.

Capo read:

- Most directly overlaps with Capo's controller/dashboard ambition.
- Adopt task board, worktree-per-card option, approval checkpoints, session persistence, and multi-surface architecture.
- Investigate whether Capo should control Cline via CLI/SDK rather than compete at the inner agent loop.

### OpenCode

Primary sources:

- https://github.com/anomalyco/opencode
- https://www.opencode.ai/

Current facts:

- Terminal-first coding agent with desktop beta.
- Built-in agents: `build` full-access development agent and `plan` read-only exploration/planning agent.
- Repo license: MIT. Website footer currently says Apache 2.0, which conflicts with the repo.

Capo read:

- Treat its `plan`/`build` split as adapter-reported state and capability hints, not as a Capo controller model.
- Good candidate for first external-agent adapter because it is local-first and has a clear CLI/TUI product.
- Recheck license and current integration surface before code reuse.

### OpenAI Codex CLI

Primary sources:

- https://github.com/openai/codex
- https://developers.openai.com/codex/

Current facts:

- Local coding agent; Rust-dominant repo.
- Installs via shell script, npm, Homebrew, and release binaries.
- Supports ChatGPT-plan sign-in and API-key setup.
- License: Apache-2.0.

Capo read:

- Strong evidence for Rust-first local controller viability.
- Important subscription-backed connector candidate.
- Capo should wrap and supervise Codex first, not fork its inner logic.

### Aider

Primary source: https://github.com/aider-ai/aider

Current facts:

- Terminal pair-programming agent for real git repos.
- Features repo map, cloud/local model support, automatic commits, lint/test loop, voice-to-code, and web-chat copy/paste mode.
- License: Apache-2.0.

Capo read:

- Adopt git-first evidence and repo-map ideas.
- Good baseline for single-agent local workflow.
- Auto-commit should be policy controlled in Capo; default should be reviewable diff before commit.

### CrewAI

Primary sources:

- https://github.com/crewAIInc/crewAI
- https://docs.crewai.com/en/introduction

Current facts:

- Python framework for autonomous agents and workflows.
- Distinguishes Flows for state/control/event-driven execution from Crews for autonomous role-based collaboration.
- Includes cloud/control-plane positioning and telemetry controls.
- License: MIT.

Capo read:

- Useful conceptual split: deterministic controller flow vs autonomous agent collaboration.
- Do not make role/backstory/task YAML the source of truth for Capo; use it only for optional agent workloads.

### AutoGen

Primary source: https://github.com/microsoft/autogen

Current facts:

- Multi-agent framework with Core API, AgentChat API, Studio GUI, and Bench.
- Current README says AutoGen is in maintenance mode and directs new users to Microsoft Agent Framework.
- Code license: MIT; docs/content license: CC-BY-4.0.

Capo read:

- Adopt layered API and evaluation/bench concepts.
- Treat framework churn as an architecture risk. Capo's durable state should outlive any one orchestration framework.

### LangGraph

Primary source: https://docs.langchain.com/oss/javascript/langgraph/persistence

Current facts:

- Persists graph state as checkpoints organized by threads.
- Persistence enables HITL, memory, time travel, replay, and fault tolerance.
- Distinguishes per-thread checkpoints from cross-thread memory store.

Capo read:

- Best source for durable state semantics.
- Adopt the concepts, not necessarily the implementation.
- Open question: verify LangGraph package license directly before any dependency or code reuse.

## Open Questions

- Which external coding agent should be the first adapter: Codex CLI, OpenCode, Cline CLI/SDK, Aider, Claude Code, or an ACP-compatible agent?
- Should Capo use `thread`/`checkpoint` vocabulary directly, or Capo-specific `session`/`snapshot` naming that can map to ACP and LangGraph concepts?
- Can Cline/OpenCode expose enough structured events for Capo to supervise them without brittle terminal scraping?
- Which agents expose cost/token/tool-call events reliably enough for dashboard accounting?
- What is the minimum runtime isolation for dogfooding: local shell with policy, Docker, or worktree-only separation?
- How should Capo represent worktree-per-task when multiple agents operate in one repository?
- Are subscription-backed connector terms compatible with process supervision and remote control? Defer to R3.

## Source Index

- Swarms GitHub: https://github.com/kyegomez/swarms
- OpenHands GitHub: https://github.com/All-Hands-AI/OpenHands
- OpenHands runtime docs: https://docs.openhands.dev/openhands/usage/runtimes/overview
- OpenHands Cloud license note: https://github.com/All-Hands-AI/OpenHands-Cloud
- Cline GitHub: https://github.com/cline/cline
- OpenCode GitHub: https://github.com/anomalyco/opencode
- OpenCode website: https://www.opencode.ai/
- OpenAI Codex GitHub: https://github.com/openai/codex
- OpenAI Codex docs: https://developers.openai.com/codex/
- Aider GitHub: https://github.com/aider-ai/aider
- CrewAI GitHub: https://github.com/crewAIInc/crewAI
- CrewAI docs: https://docs.crewai.com/en/introduction
- AutoGen GitHub: https://github.com/microsoft/autogen
- LangGraph persistence docs: https://docs.langchain.com/oss/javascript/langgraph/persistence
