# R1 - Agent Client Protocol

Observed: 2026-05-25.

## Summary

ACP is a JSON-RPC 2.0 protocol for the boundary between a coding-agent client
and a coding agent. Its center of gravity is editor/IDE integration: a client
launches or connects to an agent, negotiates protocol version and capabilities,
creates or resumes sessions, sends prompt turns, receives streaming session
updates, displays tool-call progress, answers permission requests, and exposes
optional client-side filesystem and terminal capabilities.

Recommendation for Capo: implement ACP as an adapter boundary, not as Capo's
core domain model. Build a minimal ACP client adapter early so Capo can spawn
and steer ACP-compatible agents over stdio, but keep Capo-owned orchestration,
task state, permission policy, audit logs, runtime placement, tunnel handling,
provider/subscription state, memory, evaluation, and dashboard state outside
ACP. Defer Capo-as-an-ACP-agent, ACP registry automation, custom transports, and
HTTP transport until the local prototype proves session state and recovery.

## What ACP Standardizes Today

- Wire protocol: JSON-RPC 2.0 messages, UTF-8 encoded. stdio is the defined
  transport; streamable HTTP is documented as a draft proposal; custom
  transports are allowed if they preserve the ACP lifecycle and JSON-RPC shape.
- Version negotiation: `initialize` carries the client's latest supported
  integer protocol version and client capabilities; the agent responds with the
  chosen protocol version, agent capabilities, implementation info, and
  advertised authentication methods. The current stable ACP wire protocol is
  protocol version `1`.
- Capabilities: omitted capabilities mean unsupported. Standard client
  capabilities include text file read/write and terminal methods. Standard
  agent capabilities include session load, prompt content types, MCP transport
  support, auth/logout, and newer session capabilities such as list/resume/close
  when advertised.
- Authentication: agents advertise `authMethods`; clients call `authenticate`
  with a method ID. `logout` exists only when advertised. ACP explicitly leaves
  active-session behavior after logout unspecified.
- Sessions: `session/new` creates a conversation session with an absolute `cwd`
  and MCP server configurations. `session/load` replays prior conversation via
  `session/update`; `session/resume` reconnects without replay when supported;
  `session/close` frees active-session resources when supported.
- Prompt turns: `session/prompt` sends content blocks to a session. The agent
  streams `session/update` notifications for plans, message chunks, tool calls,
  and tool-call updates, then resolves the prompt request with a stop reason.
  `session/cancel` cancels an in-flight turn.
- Content and tools: prompt content supports text and resource links at
  baseline, with optional image, audio, and embedded resource content. Tool-call
  updates standardize display-oriented fields such as `toolCallId`, title, kind,
  status, produced content, locations, raw input, and raw output.
- Permissions: an agent may call `session/request_permission`; the client
  returns selected/cancelled outcomes. ACP standardizes one-time and remembered
  allow/reject option kinds, but not Capo's full policy engine.
- MCP handoff: session setup passes MCP server definitions to agents. ACP is
  MCP-friendly and reuses MCP types where possible, but ACP and MCP are separate
  sockets/protocols; a client that wants to expose tools can provide its own MCP
  server configuration.
- Extensibility: `_meta` is available on protocol types, extension JSON-RPC
  methods/notifications must start with `_`, and custom capabilities should be
  advertised through `_meta`.
- Schema artifacts: the canonical schema is published from the ACP repository
  and downloadable from GitHub releases. Artifact/package versions are distinct
  from wire compatibility; wire compatibility is determined by negotiated
  `protocolVersion`.

## Clients, Agents, SDKs, And Ecosystem

Primary ACP docs list a large and fast-moving ecosystem. Treat the lists as
discovery input, not proof that every integration is production-stable.

Clients and client-like surfaces observed in official ACP docs include Zed,
JetBrains IDEs, VS Code via extension, Neovim plugins, Emacs via
`agent-shell.el`, Obsidian via plugin, Unity clients, CLI/TUI clients, desktop
and web clients, mobile clients, messaging bridges, notebook/data tools, and
framework/connectors such as ACP-to-AG-UI and AgentRQ.

Agents observed in official ACP docs include AgentPool, Augment Code, AutoDev,
Blackbox AI, Claude Agent via Zed SDK adapter, Cline, Codex CLI via Zed adapter,
Docker cagent, fast-agent, Factory Droid, Gemini CLI, GitHub Copilot public
preview, Goose, JetBrains Junie, Kimi CLI, Kiro CLI, OpenCode, OpenHands, Pi via
adapter, Poolside, Qwen Code, and others.

Official libraries/repos observed:

| Artifact | Observed version/status | License/version notes |
| --- | --- | --- |
| `agentclientprotocol/agent-client-protocol` | GitHub latest release `v0.13.3`, published 2026-05-22; repo pushed 2026-05-22 | Apache-2.0; README says current stable ACP protocol version is `1`; schema artifact versions are not wire versions |
| `agent-client-protocol-schema` crate | crates.io `0.13.3`; Rust `1.88.0` noted by `cargo info` | Apache-2.0; canonical schema/types artifact |
| `agent-client-protocol` Rust crate | crates.io `0.12.1`; repo `agentclientprotocol/rust-sdk` | Apache-2.0; core protocol types/traits; has many `unstable_*` feature flags |
| Python SDK `agent-client-protocol` | PyPI `0.10.1`, GitHub release published 2026-05-24 | PyPI license field empty, GitHub repo Apache-2.0; generated Pydantic schema models plus async client/agent helpers |
| TypeScript SDK `@agentclientprotocol/sdk` | npm `0.22.1`, release published 2026-05-18 | Apache-2.0; repository `agentclientprotocol/typescript-sdk` |
| Kotlin SDK | GitHub release `v0.23.0`, published 2026-05-20 | Apache-2.0 repo; JVM supported, other targets in progress per ACP README |
| Java SDK | GitHub release `v0.11.0`, published 2026-05-12 | Apache-2.0 repo |

## Protocol Boundaries Relevant To Capo

ACP covers:

- Client-to-agent subprocess/session communication.
- Capability negotiation for the protocol interaction.
- Prompt content and session update event shapes.
- Agent-visible session IDs and current working directories.
- Agent-reported plans, messages, tool calls, tool statuses, and permission
  prompts.
- Optional client methods for filesystem and terminal integration.
- MCP server configuration handoff.
- Extension metadata and custom JSON-RPC methods.

ACP does not cover, or only lightly touches:

- Multi-agent scheduling, task queues, review gates, or workpad state.
- Capo's authoritative event log, restart recovery model, and read models.
- Durable capability grants with owners, expiry, revocation, audit evidence, and
  cross-agent policy.
- Runtime isolation: local process vs container vs VM vs remote devbox.
- Connectivity/tunnel management, network policy, or remote-control auth.
- Provider and subscription-session management.
- Cost tracking, performance evaluation, reviewer findings, or outcome scoring.
- Memory extraction, summaries, long-term project knowledge, or external memory
  integrations.
- Dashboard/mobile/voice UX state beyond events that can be rendered from one
  ACP session.
- Strong sandbox guarantees. ACP says `cwd` should serve as a filesystem
  boundary, but actual enforcement is a client/runtime responsibility.

## Capo Mapping

- Capo `AgentProtocolAdapter`: implement ACP client behavior over stdio first.
  It should launch or attach to an ACP agent process, call `initialize`, create
  or resume a session, map `session/update` into Capo events, and forward
  prompts/cancel/close where supported.
- Capo `Session`: store both Capo session ID and external ACP session ID.
  Never let ACP IDs become the sole durable identity because Capo needs to track
  work across adapters and non-ACP agents.
- Capo `Message/Event`: map ACP content blocks, plans, message chunks, tool
  calls, tool-call updates, permission requests, stop reasons, auth errors, and
  cancellation into an append-only Capo event log.
- Capo `CapabilityGrant`: treat ACP capabilities as adapter capabilities, not
  as policy grants. Capo should decide whether to expose `fs`, `terminal`, and
  MCP server configurations per session based on its own grant model.
- Capo `Permission`: translate `session/request_permission` into Capo approval
  events and user-facing prompts. Store the policy decision and expiry in Capo;
  return only the selected ACP option to the agent.
- Capo `Runtime`: stdio subprocess management belongs to the runtime layer; ACP
  should not own process supervision, workspace allocation, logs, or restart
  policy.
- Capo `Provider/Subscription`: an ACP agent may hide provider auth internally,
  but Capo still needs a privileged connector model for subscription-backed
  products and must avoid logging tokens/session material.

## Recommendation

Use adapter-first ACP compatibility.

1. Prototype: implement a minimal ACP client adapter in Rust. Prefer the
   canonical schema/types crate for payloads; use the higher-level Rust SDK only
   if it materially reduces subprocess/JSON-RPC plumbing without forcing Capo's
   domain model to match ACP.
2. Internal model: define Capo sessions, tasks, messages, tool calls,
   permissions, capabilities, runtime state, and evaluation as Capo-owned
   concepts. ACP events should be one source of normalized adapter events.
3. Compatibility: support protocol version `1`, stdio transport, initialize,
   auth when advertised, `session/new`, `session/prompt`, `session/cancel`,
   `session/load` only if advertised, and permission requests. Defer
   `session/list`, `session/resume`, `session/close`, modes/config options, and
   registry install flows until the first e2e prototype works.
4. Not direct-as-core: do not make ACP the Capo controller API. Capo needs a
   richer orchestration/control plane than ACP's client-agent session protocol.
5. Not deferred: do not postpone ACP entirely. It is already the clearest
   current interoperability boundary for coding agents, with active Zed,
   JetBrains, GitHub/Copilot, SDK, and community adoption signals.

## Follow-Up Decisions From User

Recorded 2026-05-25:

- First concrete targets should be Claude Code and Codex.
- Capo should expose tools to agents. Start with easy tools, but architect the
  tool boundary so more tools can be added later.
- Capo should wrap existing agent tools where possible so tool calls are
  tracked, instrumented, audited, and eventually governed by policy.
- ACP streaming replay and Capo restart recovery dedupe require deeper
  architecture research before the event model is locked.
- Initial permissions can be permissive for trusted local dogfooding, but the
  decision mechanism should be modular enough to later use static policy, user
  approval, or a fast security agent.
- Do not prioritize Capo-as-ACP-agent/editor-backend mode right now. Capo should
  remain the user's entrypoint for the prototype.

## Confidence

Medium-high. The core protocol shape, protocol versioning rule, stdio transport,
schema release, and SDK availability are directly sourced from ACP docs, GitHub,
crates.io, PyPI, and npm as observed on 2026-05-25. Confidence is lower on the
operational quality of the long integration lists because official docs list
many clients/agents with mixed maturity labels and some adapter-based support.

## Open Questions

- How should Capo persist and replay partial streaming updates so ACP
  `session/load` replay and Capo restart recovery do not produce duplicate UI
  state?
- Which first Capo-exposed tools are useful enough for the prototype while
  remaining simple to instrument?
- Should v0 implement Claude Code, Codex, and ACP in one prototype pass, or
  build Claude/Codex first and add ACP after the event model is proven?

## Sources

- ACP repository README and versioning notes, observed 2026-05-25:
  https://github.com/agentclientprotocol/agent-client-protocol
- ACP protocol overview, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/overview
- ACP architecture, observed 2026-05-25:
  https://agentclientprotocol.com/get-started/architecture
- ACP initialization and capabilities, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/initialization
- ACP authentication, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/authentication
- ACP session setup/load/resume/close, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/session-setup
- ACP prompt turn, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/prompt-turn
- ACP tool calls and permission requests, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/tool-calls
- ACP transports, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/transports
- ACP extensibility, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/extensibility
- ACP schema page, observed 2026-05-25:
  https://agentclientprotocol.com/protocol/schema
- ACP clients page, observed 2026-05-25:
  https://agentclientprotocol.com/get-started/clients
- ACP agents page, observed 2026-05-25:
  https://agentclientprotocol.com/get-started/agents
- Python SDK docs, observed 2026-05-25:
  https://agentclientprotocol.github.io/python-sdk/
- TypeScript SDK package metadata, observed 2026-05-25:
  https://www.npmjs.com/package/@agentclientprotocol/sdk
- Rust crates, observed 2026-05-25:
  https://crates.io/crates/agent-client-protocol
  https://crates.io/crates/agent-client-protocol-schema
- Python package metadata, observed 2026-05-25:
  https://pypi.org/project/agent-client-protocol/
- Zed ACP ecosystem page, observed 2026-05-25:
  https://zed.dev/acp
- JetBrains ACP page, observed 2026-05-25:
  https://www.jetbrains.com/acp/
- GitHub Copilot CLI ACP server docs, observed 2026-05-25:
  https://docs.github.com/en/enterprise-cloud@latest/copilot/reference/copilot-cli-reference/acp-server
