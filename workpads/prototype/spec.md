# Capo Prototype Spec

## Objective

Define the smallest useful Capo product: an end-to-end controller that can spawn or register an agent, send work, inspect progress, interrupt execution, persist state, and produce evidence good enough for dogfooding decisions.

## Vision

Capo should become a control plane for coding agents: humans give intent through text, voice, mobile, or dashboard surfaces; Capo delegates to agents running locally or remotely; state, summaries, memories, permissions, and review evidence remain inspectable and durable.

## Prototype Minimum

The first prototype is successful when a user can:

1. Start Capo locally.
2. Register or spawn at least one coding-agent runtime.
3. Send that agent a task.
4. See the agent's current goal, status, recent events, and latest summary.
5. Interrupt or stop the agent.
6. Persist state and recover it after restarting Capo.
7. Record task evidence in a workpad-like artifact.

## MVP v0

Beyond the minimum, MVP v0 should include:

- Multiple concurrent agents.
- A simple dashboard or TUI.
- Capability profiles for shell/filesystem/git/network.
- Basic memory references using markdown and/or SQLite.
- Agent performance report for completed tasks.
- Local runtime first; remote runtime/tunnel interface stubbed or minimally proven.
- ACP adapter or compatibility layer if research confirms fit.

## Deferred From Prototype

- Full mobile app.
- Production voice control.
- Production subscription connector automation.
- Advanced graph/vector memory.
- Cloud multi-tenant deployment.
- Full evaluation framework.

## Dogfood Gate

Capo can begin managing its own project when:

- It can track multiple work items and agent sessions.
- It persists state across restart.
- It can show active goal, blocker, confidence, and evidence for each session.
- It has a human-auditable fallback in markdown workpads.
- It can export or update project workpads without corrupting them.
- The user can interrupt or redirect a running agent reliably.

## Non-Goals

- Do not build an undifferentiated chat wrapper.
- Do not make provider-specific behavior leak into the controller core.
- Do not couple tunnel choice to runtime or memory design.
- Do not require API keys when subscription-backed workflows are a first-class goal.
