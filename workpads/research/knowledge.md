# Research Knowledge

## Objective

Establish the facts and recommendations Capo needs before choosing durable architecture: what to build on, what to avoid, which integrations are realistic, and which unknowns require user decisions.

## Research Gate

Status: passed on 2026-05-25.

Confidence: medium-high. The main architecture direction is well-supported by primary/current sources and focused subagent findings. Some provider/subscription details are volatile and must be rechecked before implementation.

Architecture may start. The research gate passes because it now records:

- ACP fit and integration recommendation.
- Prior-art lessons from agent orchestration systems.
- Prototype stack recommendation.
- Subscription connector feasibility and security boundary.
- Memory baseline recommendation.
- Runtime/tunnel recommendation.
- Input surface sequence recommendation.
- Top open questions and decisions required from the user.

## Executive Recommendation

Build Capo as a local-first controller and harness, not as another agent framework.

For the first prototype:

- Rust owns controller authority: state machine, event log, capability grants, process supervision, ACP/JSON-RPC boundaries, API surface, and audit logging.
- Python is allowed only as supervised sidecars for ecosystem-heavy adapters such as voice, local-model experiments, and memory experiments.
- ACP is an adapter boundary, not Capo's core domain model.
- First agent path should be a local CLI/stdio agent adapter, with Codex `codex exec --json` and an ACP-compatible agent as the strongest first candidates.
- Subscription-backed products should be integrated through vendor-supported local CLIs/SDKs, not browser scraping.
- Memory v0 should be markdown workpads plus a SQLite event log; derived semantic/graph memory must be rebuildable.
- Runtime v0 should be local process execution with explicit workspace, environment, process, and capability policy; remote/Tailscale/SSH comes after local recovery works.
- First input surface should be CLI, followed by a local dashboard, mobile as responsive/PWA, and voice as push-to-talk transcription into the same command model.

## R1 - Agent Client Protocol

Finding: ACP is the best current interoperability boundary for coding-agent clients and agents.

Recommendation: implement adapter-first ACP compatibility.

- Use ACP over stdio for a minimal client adapter early.
- Keep Capo-owned tasks, sessions, events, capability policy, runtime placement, tunnel handling, provider/subscription state, memory, evaluation, and dashboard state outside ACP.
- Store both Capo session IDs and external ACP session IDs.
- Map ACP `session/update`, tool calls, permission requests, stop reasons, and auth errors into Capo's append-only event log.
- Treat ACP capabilities as adapter capabilities, not durable Capo policy grants.
- Defer Capo-as-ACP-agent, registry install flows, custom transports, HTTP transport, and advanced session features until the first e2e prototype works.

Current facts observed 2026-05-25:

- ACP is a JSON-RPC 2.0 protocol for the client-to-agent boundary.
- Stable wire protocol version observed: `1`.
- Stdio is the defined transport; streamable HTTP is documented as draft.
- Official SDKs/artifacts exist for Rust/schema, Python, TypeScript, Java, and Kotlin.
- Official ecosystem pages list clients and agents including Zed, JetBrains, VS Code extensions, Neovim, GitHub Copilot public preview, Goose, OpenCode, OpenHands, Gemini CLI, and Codex/Claude adapters.

Confidence: medium-high. Protocol facts are strongly sourced; individual ecosystem integration maturity varies.

Detailed finding: `workpads/research/findings/R1-acp.md`.

## R2 - Prior Art

Finding: coding-agent harnesses are more relevant to Capo than generic multi-agent frameworks.

Studied systems include Swarms, OpenHands, Cline, OpenCode, OpenAI Codex CLI, Aider, CrewAI, AutoGen, and LangGraph.

Adopt:

- Durable event/session/checkpoint model inspired by LangGraph, Cline/OpenCode/Codex task sessions, and coding-agent checkpoints.
- Runtime boundary inspired by OpenHands.
- Human-in-the-loop permissions and reviewable diffs from Cline, OpenCode, Aider, and Codex.
- Controller-owned capability profiles plus adapter/subagent-reported state. Capo should not define its own user-facing modes.
- Provider-agnostic adapters where possible.

Reject or defer:

- Framework-first "crew/swarm" abstractions as Capo's core state model.
- Autonomous multi-agent swarms before a single-agent harness proves state, permissions, stop/resume, and review evidence.
- Dashboard/cloud-first architecture before local dogfooding works.
- Coupling Capo's state model to MCP, A2A, one model SDK, or one agent framework.
- Capo-owned "modes" as a product abstraction. If Claude Code, Codex, or another subagent has modes, Capo records and routes that state but remains the controller.

Key failure modes to design against:

- Runaway autonomy.
- Silent background stalls.
- Permission drift where prompts claim "read-only" without enforcement.
- State loss in terminal-only agents.
- Credential leakage from subscription connectors.
- Unreviewable edits.
- Framework churn.
- License/product ambiguity.

Confidence: medium-high.

Detailed finding: `workpads/research/findings/R2-prior-art.md`.

Code architecture follow-up: `workpads/research/findings/R2-code-architecture.md`.

Source-code architecture lessons:

- Capo should use controller-owned session/event IDs and store external adapter IDs separately.
- Persist raw adapter events separately from normalized Capo events, then project dashboard/CLI state from Capo events.
- Codex's JSONL event shape is a strong first adapter target, but not authoritative persistence.
- Cline shows useful checkpoint/permission ideas but also a warning against one giant task object and VS Code/webview coupling.
- OpenHands has the strongest runtime/sandbox abstraction, but its server/cloud stack is too broad for v0.
- OpenCode's small services and SQLite read models are useful, but permission requests must be durable rather than in-memory only.
- Aider reinforces git/worktree awareness and markdown history, but Capo should not adopt in-process terminal chat as its core.
- Checkpoints are state-management primitives in Capo, not LangGraph graphs.

## R3 - Subscription-Backed Connectors

Finding: subscription-backed agents are feasible only through vendor-supported local surfaces first.

Recommendation:

- Support Codex CLI / Codex SDK / Codex access tokens where available.
- Support Claude Code CLI / `claude -p` / Claude Agent SDK where the user's plan and terms permit it.
- Use API keys, workload identity, or enterprise tokens for hosted, shared, CI, or productized automation.
- Do not build ChatGPT/Claude web UI scraping as a first-class connector.
- Explicitly reject reverse-engineered private endpoints/session-token reuse.

Security boundary:

- Model subscription connectors as privileged local agent runtimes, not ordinary model providers.
- Capo must not read, copy, persist, log, or sync vendor OAuth tokens, cookies, keychain entries, `.credentials.json`, browser storage, or Playwright storage state.
- Store only non-secret connector metadata and audit events.
- Launch vendor CLIs with minimized environment and explicit credential scope.
- Make revocation a first-class connector action.

Prototype recommendation:

1. Build `LocalCliAgent` for `codex exec --json` and parse JSONL events.
2. Add a `claude -p` adapter after the event model is stable.
3. Do not implement web automation.
4. Add `credential_scope`: `user-local-subscription`, `api-key`, `wif`, `enterprise-access-token`, `browser-experiment`.
5. Mark consumer subscription connectors as local-only and not hosted/multi-tenant by default.

Confidence: medium-high for local CLI paths; medium for policy durability because vendor terms and plan entitlements change.

Detailed finding: `workpads/research/findings/R3-subscriptions.md`.

## R4 - Stack Choice

Finding: Capo should use a Rust-first hybrid stack.

Prototype stack:

- Rust core daemon/CLI: `tokio`, `axum`, `clap`, `serde`, `rusqlite`, official ACP schema/crate if stable enough for the selected adapter.
- SQLite event log and read models.
- Markdown workpads for human-auditable project state.
- Python sidecars only when needed, launched as subprocesses with newline-delimited JSON-RPC or ACP-compatible stdio.
- No embedded Python or dynamic native plugins in v0.

Why:

- Rust is the right authority layer for process supervision, restart recovery, state transitions, capability grants, and audit logging.
- Python has better leverage for voice, local-model, and memory experiments.
- Process boundaries make sidecars killable, versioned, and permission-scoped.

Build/test implications:

- Rust: `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
- Python sidecars when present: `uv lock`, `uv sync --locked`, `ruff format`, `ruff check`, `pytest`.
- Add golden transcript tests for JSON-RPC/ACP adapter contracts.

Confidence: medium-high.

Detailed finding: `workpads/research/findings/R4-R6-stack-runtime.md`.

## R5 - Memory Systems

Finding: memory should be layered, local-first, and auditable.

Recommendation:

- v0: markdown/file-backed workpads plus SQLite event log.
- v1: add a `MemoryBackend` adapter with local FTS, optional semantic search, and possibly temporal graph memory after real dogfood traces exist.
- Do not use Tana, Capacities, Zep Cloud, mem0 Cloud, or Letta Cloud as authoritative state.
- Treat external tools as optional import/export/sync targets.

Memory layers:

1. Raw event memory in SQLite.
2. Human project memory in markdown workpads.
3. Curated fact memory with provenance, confidence, validity, and review state.
4. Retrieval memory: FTS, embeddings, rerankers, graph traversal.
5. Prompt memory: task-specific fraction injected into an agent context.

Data ownership rule: every derived memory shown to an agent must include provenance back to a local event, local file, or explicit user-imported external record.

v1 candidates:

- Local FTS: SQLite FTS5.
- Local semantic index: Chroma, LanceDB, Qdrant, or later Postgres/pgvector if server mode needs Postgres.
- Temporal graph experiment: Graphiti or Graphiti-like model if timestamped fact invalidation is not enough.

Confidence: medium-high for v0, medium for v1 backend choice.

Detailed finding: `workpads/research/findings/R5-memory.md`.

## R6 - Runtime And Tunnel Options

Finding: local process runtime first; remote runners later through explicit runtime/tunnel boundaries.

Recommendation:

- v0 runtime: local process execution with explicit command, args, workspace root, environment allowlist, process group tracking, stdout/stderr capture, optional PTY, heartbeats, and kill escalation.
- v0 sandbox: capability profiles plus workspace scoping and audit. Do not claim hard sandboxing.
- v1 runtime: container/devcontainer, Linux sandbox profiles, SSH runner, and Tailscale-reached remote runners.
- Keep connectivity separate from runtime. Tailscale/SSH provide reachability; they do not own task state or runtime policy.

Tunnel sequence:

1. Local loopback only for v0.
2. SSH as first remote-runner primitive.
3. Tailscale as near-term private remote-control path.
4. Tailscale Funnel / Cloudflare Tunnel only for explicit demos or public webhook/dashboard exposure, off by default.

Capability profile fields should cover filesystem, shell, git, network, secrets, and browser/subscription access.

Confidence: medium-high for local-first runtime; medium for exact sandbox implementation because OS enforcement differs.

Detailed finding: `workpads/research/findings/R4-R6-stack-runtime.md`.

## R7 - Input Surfaces

Finding: CLI first for implementation, but voice is a first-class conversational interface to Capo, not merely dictation.

Recommendation:

1. `capo` CLI for create/list/show/send/cancel/approve.
2. Local web dashboard for inspection, approvals, interrupts, and steering.
3. Mobile as responsive authenticated web/PWA.
4. Voice as conversational interaction with Capo: ask what agents have done, get status and summaries, discuss next steps, and steer one or more agents.

Canonical input model:

- Every surface submits a controller-owned `CommandEnvelope`.
- Text and voice both lower into the same controller intents and conversational turns.
- Controller commands such as cancel, approve, spawn, and capability changes are first-class controller intents, not plain agent chat.
- Every accepted command emits durable events.

Voice model:

- Capo itself is the conversational counterpart.
- Voice sessions should be able to query Capo's read models: active agents, recent work, blockers, evidence, pending permissions, task status, and summaries.
- Voice sessions should steer agents through Capo commands: send messages, redirect goals, pause/cancel/resume, request summaries, and approve low-risk actions when policy allows.
- Voice should maintain a short conversational context over Capo state, but durable truth remains the controller event log and read models.
- Early implementation can still use push-to-talk for privacy, but the architecture should support back-and-forth conversation, not just one-shot command transcription.

Minimum dogfood dashboard state:

- Active workpad/task status, acceptance criteria, evidence links.
- Agent identity, runtime, provider/adapter, mode, health, capability profile.
- Session ID, cwd, title, active goal, status, current turn.
- Recent events, latest summary, plan, tool calls, pending operations.
- Permission queue and approve/deny controls.
- Interrupt controls.
- Recovery state.
- Review/evidence/confidence/blockers.
- Worktree/branch/dirty state.
- Metrics and audit trail where available.

Voice policy:

- Start with push-to-talk, not wake word.
- Default to no raw-audio persistence.
- Treat transcripts as sensitive.
- Privileged voice commands require visible confirmation.
- Conversation summaries may be stored as Capo events, but raw audio should be transient by default.

Confidence: high for CLI-first and shared command model; medium for exact ASR choice.

Detailed finding: `workpads/research/findings/R7-input-surfaces.md`.

## Architecture Inputs

Architecture should define these first:

- Capo event log and read model.
- `AgentProtocolAdapter` trait with Claude Code, Codex, ACP, and local CLI adapters.
- `RuntimeRunner` trait with local process implementation.
- `CapabilityProfile` and durable grant/approval model.
- `CommandEnvelope` and controller command validation.
- SQLite schema for sessions, runs, events, tasks, capability grants, artifacts, memory facts, and memory packets.
- Connector credential scope and productization policy.
- Local-only security stance for v0, with remote/tunnel interfaces defined but not required.
- Dogfood dashboard read model.
- Capo tool exposure and instrumentation model. Start with easy tools, and wrap existing agent tools so Capo can track/instrument them.
- ACP streaming replay and Capo restart recovery dedupe rules.
- Conversational voice interface to Capo over the same command/read-model boundary as CLI/dashboard.

## User Decisions - 2026-05-25

- Target Claude Code and Codex first.
- Capo should expose tools, beginning with easy tools and growing through a clear tool boundary.
- Capo should wrap existing agent tools where possible so tool usage is observable and instrumented.
- ACP streaming replay / Capo restart dedupe needs additional architecture research.
- Early permissions can allow everything, but the decision architecture must be modular enough for static policy, user approval, or a fast security agent later.
- Do not prioritize exposing Capo as an ACP agent/editor backend. Capo should be the entrypoint for now.
- Voice should support conversations with Capo about other agents' status, work completed, blockers, and steering decisions. It is not just a generic speech-to-text input.

## Open Questions For Architecture/User

- Should v0 implement Claude Code, Codex, and ACP in the same prototype, or sequence Claude/Codex first then ACP compatibility?
- First control surface after CLI: local web dashboard or TUI?
- v0 target OS: macOS local dogfood first, Linux runner first, or both?
- What is the default transcript/log retention policy for subscription-backed and voice sessions?
- Should generated memory require human review before it can become prompt-pinned?
- Should Capo use `session/checkpoint` vocabulary or different names that map to ACP/LangGraph concepts?

## Research Artifacts

- `workpads/research/findings/R1-acp.md`
- `workpads/research/findings/R2-prior-art.md`
- `workpads/research/findings/R2-code-architecture.md`
- `workpads/research/findings/R3-subscriptions.md`
- `workpads/research/findings/R4-R6-stack-runtime.md`
- `workpads/research/findings/R5-memory.md`
- `workpads/research/findings/R7-input-surfaces.md`
