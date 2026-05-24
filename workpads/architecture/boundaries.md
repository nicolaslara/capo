# Capo Boundary Definitions

## Objective

Define Capo's system vocabulary and adapter boundaries so implementation can stay modular as runtimes, tunnels, providers, input surfaces, state stores, and memory systems change.

This file defines the target architecture vocabulary. It is intentionally implementation-neutral until the architecture gate passes.

## Boundary Map

```text
Human input surfaces
  -> Capo controller
  -> Agent protocol adapter
  -> Agent runtime
  -> Model/provider/subscription connector

Capo controller
  -> State store
  -> Memory layer
  -> Evaluation/review layer
  -> Connectivity/tunnel manager
```

## Core Principle

Each boundary should be replaceable without forcing unrelated parts to change. A Tailscale tunnel should not know how memories are stored. A Claude subscription connector should not own task state. A mobile input surface should not know where an agent executes.

## Boundaries

### Input Surface

Captures user intent and renders Capo state.

Examples: CLI, TUI, web dashboard, mobile app, voice command processor.

Initial contract ideas:

- Submit command/message
- Subscribe to session/task updates
- Approve or deny capability requests
- Interrupt/pause/resume an agent

### Capo Controller

Owns orchestration policy and authoritative state transitions.

Responsibilities:

- Task/session creation
- Agent lifecycle orchestration
- Capability assignment
- Event ingestion
- Persistence coordination
- Review/evaluation hooks
- Recovery after restart

### Agent Protocol Adapter

Normalizes communication with agents.

Examples: ACP adapter, CLI adapter, browser/subscription adapter, custom JSON-RPC adapter.

Responsibilities:

- Session start/stop
- Message exchange
- Tool/capability negotiation
- Progress/event stream normalization
- Error mapping

### Agent Runtime

Executes the agent process or environment.

Examples: local process, tmux/session runner, cloud VM, container, remote dev box.

Responsibilities:

- Start/stop process
- Attach logs/events
- Provide workspace
- Enforce environment/capability constraints where possible
- Report health

### Connectivity/Tunnel

Connects Capo to remote runtimes.

Examples: Tailscale, SSH, reverse tunnel, local-only loopback.

Responsibilities:

- Reachability
- Authentication/authorization
- Network policy
- Connectivity health

Non-responsibilities:

- Agent task state
- Provider authentication
- Memory semantics

### Model/Provider Connector

Supplies model intelligence or connects to a subscription-backed product.

Examples: API provider, Claude Code, ChatGPT, local vLLM server, Ollama.

Responsibilities:

- Provider-specific auth/session management
- Provider capability metadata
- Cost/rate-limit metadata where available
- Provider error mapping

### Capability Layer

Defines and enforces what an agent may do.

Examples: shell, git, filesystem, browser, network, MCP tools, voice transcript access.

Initial fields:

- Capability ID
- Scope
- Grant source
- Expiry/revocation
- Audit events

### State Store

Persists operational truth.

Initial candidates:

- SQLite event log and read models
- Markdown workpad pointers for human-readable plans

State categories:

- Agents
- Sessions
- Tasks/goals
- Messages/events
- Capability grants
- Artifacts
- Reviews/evaluations
- Memory references

### Memory Layer

Stores distilled reusable context, separate from operational state.

Initial candidates:

- Markdown files
- SQLite index pointing to markdown
- Later: graph/vector/external memory systems

### Evaluation Layer

Assesses agent outcomes.

Inputs:

- Task acceptance criteria
- Tests and smoke evidence
- Review findings
- Human feedback
- Time/cost/retry data

Outputs:

- Completion quality
- Failure taxonomy
- Agent/provider performance notes
- Recommendations for future routing

## Architecture Gate Criteria

To pass architecture:

- Boundary contracts are concrete enough for prototype tasks.
- State model supports restart recovery.
- Capability model covers at least shell/filesystem/git/network grants.
- Runtime model covers at least local process execution.
- Protocol model explains ACP fit or deferral.
- Security model covers subscription sessions, tunnels, logs, and secrets.
- Prototype plan identifies the thinnest e2e product path.
