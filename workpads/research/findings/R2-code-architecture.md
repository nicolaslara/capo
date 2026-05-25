# R2 Code Architecture Research

Observed 2026-05-25. Scope: direct source inspection of closest open-source coding-agent products, with local clones under `workpads/references/repos/`. This pass focuses on repository architecture, not product feature comparison.

## Executive Recommendation

Capo should be a controller with durable state, event projection, adapter boundaries, runtime supervision, and capability governance. It should not become another monolithic coding agent.

Adopt:

- A controller-owned session/event model with stable Capo IDs and external adapter IDs.
- Append-oriented event ingestion with read models for dashboard/CLI projections.
- A runtime boundary that can start/stop/health-check local processes first, then containers/remote sandboxes later.
- A permission boundary that records requests, decisions, grants, revocation, and tool-call audit data even when the prototype policy allows everything.
- Provider/agent adapters that normalize Codex, Claude Code, ACP agents, and other CLIs without making their internal session model authoritative.
- Checkpoint concepts from LangGraph and Cline/OpenHands, but implemented in Capo's own event/state store.

Reject or defer:

- Product-defined user-facing "modes" in Capo. Modes belong to subagents/adapters if those tools expose them. Capo is the controller: it may track agent state, strategy, status, capability profile, and adapter metadata, but should not define global Plan/Act/Architect/Ask modes as first-class user-facing product concepts.
- VS Code/webview-specific controller coupling from Cline.
- OpenHands' server/cloud-first conversation/sandbox stack for v0.
- Aider's in-process terminal chat architecture as Capo's core, though its git/history lessons are useful.
- OpenCode's single-process TUI-plus-agent model as Capo's architecture, though its small Go service boundaries are instructive.
- Crew/swarm abstractions as core state. LangGraph is useful only as a checkpointing/durable-execution reference.

## Repositories Observed

| Product | Source | Commit observed | Date | License notes |
| --- | --- | --- | --- | --- |
| OpenAI Codex | https://github.com/openai/codex, local `workpads/references/repos/openai-codex` | `9f42c89c0112771dc29100a6f3fc904049b2655f` | 2026-05-24 | Apache-2.0 (`LICENSE`, `docs/license.md`) |
| Cline | https://github.com/cline/cline, local `workpads/references/repos/cline` | `8a6441fddd3b4d372d086886ebe4ee11e78dc993` | 2026-05-22 | Apache-2.0 (`LICENSE`) |
| OpenHands | https://github.com/All-Hands-AI/OpenHands, local `workpads/references/repos/openhands` | `5e311f7f995008ffe4c74f8cf6f3085d4030c670` | 2026-05-22 | Main repo MIT, `enterprise/` PolyForm Free Trial (`LICENSE`, `enterprise/LICENSE`) |
| OpenCode | https://github.com/opencode-ai/opencode, local `workpads/references/repos/opencode` | `73ee493265acf15fcd8caab2bc8cd3bd375b63cb` | 2025-09-17 | MIT (`LICENSE`); repo README says archive note at observed commit |
| Aider | https://github.com/Aider-AI/aider, local `workpads/references/repos/aider` | `5dc9490bb35f9729ef2c95d00a19ccd30c26339c` | 2026-05-22 | Apache-2.0 (`LICENSE.txt`) |
| LangGraph docs | https://docs.langchain.com/oss/python/langgraph/persistence, https://docs.langchain.com/oss/python/langgraph/durable-execution, https://docs.langchain.com/oss/python/langgraph/human-in-the-loop | docs crawled/observed 2026-05-25 | 2026-05-25 | Reference only; do not bind Capo core to LangGraph |

## OpenAI Codex

Relevant source paths:

- `codex-rs/core/src/codex_thread.rs`
- `codex-rs/core/src/client.rs`
- `codex-rs/core/src/exec.rs`
- `codex-rs/core/src/safety.rs`
- `codex-rs/core/src/exec_policy.rs`
- `codex-rs/protocol/src/protocol.rs`
- `codex-rs/protocol/src/config_types.rs`
- `codex-rs/protocol/src/permissions.rs`
- `codex-rs/exec/src/exec_events.rs`
- `codex-rs/exec/src/event_processor_with_jsonl_output.rs`
- `sdk/typescript/src/exec.ts`
- `sdk/typescript/src/thread.ts`

Architecture observed:

- Strong Rust crate split: protocol types, core session/thread logic, exec CLI, sandboxing, MCP/tools, network proxy, app server protocol, SDK wrappers.
- Session/thread communication uses an explicit submission/event queue model in `codex-rs/protocol/src/protocol.rs`.
- `CodexThread` wraps a session, exposes submit/shutdown/resume hooks, and carries a `ThreadConfigSnapshot` containing model, provider, approval policy, permission profile, cwd, workspace roots, session source, and collaboration/personality metadata.
- `ModelClient` is session-scoped while per-turn settings are passed explicitly. This is a good boundary: stable provider/auth/transport state is separate from turn execution.
- Noninteractive `codex exec --experimental-json` emits typed JSONL events from `codex-rs/exec/src/exec_events.rs`: `thread.started`, `turn.started`, `turn.completed`, `turn.failed`, `item.started`, `item.updated`, `item.completed`, and `error`.
- Event projection code maps lower-level server notifications into stable exec item IDs and typed items. `EventProcessorWithJsonOutput` keeps `raw_to_exec_item_id`, running todo-list state, final message, token usage, and critical error state.
- Runtime/process handling in `codex-rs/core/src/exec.rs` is mature: command, cwd, env, timeout/cancellation, output caps, process-group cleanup, stdout/stderr drain timeout, sandbox selection, and network proxy integration.
- Permission handling is layered: `AskForApproval`, `ApprovalsReviewer`, `PermissionProfile`, filesystem/network sandbox policies, patch safety checks, and exec policy rules. Importantly, policy can reject, ask, or auto-approve, and policy conflicts have explicit reasons.
- SDKs wrap the CLI rather than reimplementing the agent. The TypeScript SDK spawns `codex exec --experimental-json`, writes prompt input to stdin, parses JSONL, handles abort signals, maps thread IDs, and exposes turn results.

What Capo should adopt:

- Treat Codex as an adapter initially via `codex exec --experimental-json`, not by linking to internals.
- Use a similar event vocabulary shape: run/thread/session started, turn started/completed/failed, item started/updated/completed, tool call, command execution, file change, reasoning/summary, usage, error.
- Preserve raw external item IDs plus Capo item IDs for dedupe/replay.
- Separate session-scoped provider/adapter configuration from per-turn command settings.
- Make cancellation, timeout, output limits, cwd, workspace roots, and environment policy part of the runtime contract.
- Model permission decisions with explicit request, decision, reason, scope, persistence duration, and decision source.

What Capo should reject/defer:

- Do not copy Codex's internal collaboration/personality/mode vocabulary into Capo's product model.
- Do not make MCP a core Capo dependency. Capo may expose or wrap tools later, but controller state should not depend on MCP-specific semantics.
- Do not rely on Codex's JSONL stream as authoritative persistence. It is adapter input; Capo persists its own normalized event log.

## Cline

Relevant source paths:

- `src/core/controller/index.ts`
- `src/core/task/index.ts`
- `src/core/task/TaskState.ts`
- `src/core/task/ToolExecutor.ts`
- `src/core/task/tools/handlers/*`
- `src/core/storage/StateManager.ts`
- `src/core/storage/disk.ts`
- `src/core/permissions/CommandPermissionController.ts`
- `src/core/ignore/ClineIgnoreController.ts`
- `src/core/controller/checkpoints/*`
- `src/core/webview/WebviewProvider.ts`
- `src/shared/storage/*`
- `src/shared/services/Session.ts`

Architecture observed:

- Cline is extension/controller centered. `Controller` owns the active `Task`, MCP hub, account/auth services, state manager, workspace manager, remote config, webview state posting, and UI command handlers.
- The task runtime is a large in-process object. `Task` imports context management, API streams, tools, terminal, browser, checkpoints, workspace roots, telemetry, hooks, state manager, and UI messaging.
- State is cache-first with debounced file persistence. `StateManager` loads global/task/secrets/workspace state once, reads from memory after initialization, writes asynchronously, and watches task history for external changes.
- Disk storage uses atomic write patterns and a global storage directory. Conversation history and UI messages are file-backed.
- Cline has explicit command permission parsing via `CLINE_COMMAND_PERMISSIONS`, including allow/deny patterns, redirect checks, shell operator handling, and recursive segment validation.
- `.clineignore` controls LLM/tool file access and is watched live. It filters paths and command access, but comments show that paths outside cwd may be allowed.
- Checkpoints are surfaced as events to subscribers and restore cancels the active task before altering history/files.
- UI/controller split is pragmatic but VS Code/webview heavy. gRPC/proto handlers and webview subscriptions dispatch into the same controller/task state.
- Cline exposes user-facing Plan/Act modes and model/provider selection per mode.

What Capo should adopt:

- A state manager with in-memory read models plus durable persistence is useful, but Capo should back it with SQLite events/read models instead of only file caches.
- Debounced persistence is useful for UI settings; operational events should be appended synchronously or transactionally before side effects are considered durable.
- Watchable ignore/capability files are useful, but Capo should represent their effect as capability/profile events.
- Checkpoint restore should interrupt/cancel active execution before mutating workspace or message history.
- Cline's command permission parser is a good warning that command policy needs structured parsing and explicit deny reasons.

What Capo should reject/defer:

- Do not make the active task a giant object that imports every subsystem.
- Do not couple controller state to a single UI host such as VS Code webviews.
- Do not copy Plan/Act as Capo modes. If a Cline adapter exposes Plan/Act, Capo records it as adapter metadata or subagent strategy, not as a Capo-wide mode system.
- Do not rely on prompt-level read-only semantics without runtime enforcement or audit.

## OpenHands

Relevant source paths:

- `openhands/app_server/config.py`
- `openhands/app_server/app_conversation/app_conversation_service.py`
- `openhands/app_server/app_conversation/app_conversation_models.py`
- `openhands/app_server/event/event_service.py`
- `openhands/app_server/event/event_store.py`
- `openhands/app_server/event/filesystem_event_service.py`
- `openhands/app_server/sandbox/sandbox_service.py`
- `openhands/app_server/sandbox/process_sandbox_service.py`
- `openhands/app_server/sandbox/docker_sandbox_service.py`
- `openhands/app_server/sandbox/remote_sandbox_service.py`
- `openhands/app_server/file_store/files.py`
- `openhands/app_server/file_store/local.py`

Architecture observed:

- OpenHands currently exposes a server-side application architecture with explicit services and injectors: conversation service, event service, sandbox service, sandbox spec service, file store, settings, secrets, user context, web client config, and DB session.
- `AppConversation` is metadata-plus-status: conversation ID, sandbox ID, repository/branch/provider, trigger, parent/sub-conversations, metrics, sandbox status, execution status, URL, and session API key.
- Conversation start is modeled as an async status stream. Status progresses through sandbox wait, repository prep, setup scripts, skills/hooks, conversation start, ready/error.
- Event service is abstract and has filesystem implementation. Events are persisted per conversation as JSON via a service layer.
- File store is abstract with local/S3/GCS-like backends; local writes use temp file plus replace.
- Sandbox service is the strongest relevant prior art: process, Docker, and remote implementations share search/get/start/resume/pause/delete/wait contracts.
- Process sandbox launches an agent server as a separate Python process in a dedicated directory, with a unique port, session API key, health check, and log file to avoid pipe-buffer deadlocks.
- Docker sandbox maps container status to sandbox status, exposes named URLs, manages session API keys, ports, mounts, CORS, and old-sandbox cleanup.
- Remote sandbox service persists sandbox records in SQL, hashes session API keys, updates keys on resume, clears hashes on pause, and maps runtime API state back to app sandbox info.

What Capo should adopt:

- Define runtime/sandbox as an interface from day one: `start`, `resume`, `pause`, `stop/delete`, `health`, `logs`, `exposed endpoints`, `workspace`, and `status`.
- Keep runtime placement separate from conversation/session state.
- Use status-stream task creation for long startup flows.
- Store secrets/session keys as privileged runtime connector material; store hashes or references where possible.
- Use a file-store abstraction only for artifacts/logs if needed; keep operational truth in SQLite.

What Capo should reject/defer:

- Do not start with OpenHands' cloud/server/app-conversation stack. It is too broad for Capo v0.
- Do not expose session API keys or runtime URLs in ordinary UI state unless redacted and scoped.
- Do not make repository integrations, setup scripts, SaaS storage, or multi-user org features part of v0 architecture.
- Do not import OpenHands' `AgentType.DEFAULT` / `AgentType.PLAN` concept into Capo modes.

## OpenCode

Relevant source paths:

- `internal/session/session.go`
- `internal/db/migrations/20250424200609_initial.sql`
- `internal/db/querier.go`
- `internal/db/models.go`
- `internal/pubsub/events.go`
- `internal/pubsub/broker.go`
- `internal/llm/agent/agent.go`
- `internal/llm/agent/tools.go`
- `internal/llm/provider/provider.go`
- `internal/llm/tools/tools.go`
- `internal/llm/tools/bash.go`
- `internal/permission/permission.go`
- `internal/tui/tui.go`

Architecture observed:

- Small Go service boundaries: session service, message/file DB service, pubsub broker, agent service, permission service, provider abstraction, tools, TUI.
- SQLite schema has `sessions`, `messages`, and `files`, with parent session IDs, summary message ID, token/cost fields, timestamps, and cascading deletes.
- Session service wraps SQLC queries and publishes created/updated/deleted events through a generic pubsub broker.
- Agent service prevents concurrent requests per session, stores cancellation functions in `activeRequests`, streams provider/tool events, creates title and summarizer sub-sessions/providers, and emits agent events.
- Provider abstraction is simple: `SendMessages`, `StreamResponse`, `Model`; provider clients cover Anthropic, OpenAI, Gemini, Bedrock, Copilot, Azure, Vertex, OpenRouter, local OpenAI-compatible endpoint, etc.
- Tool abstraction is clean: `Info()` returns schema-ish metadata and `Run(ctx, ToolCall)` returns typed response.
- Permission service publishes permission requests and blocks on a response channel; it supports persistent grants per session/action/path/tool and auto-approve sessions.
- TUI subscribes to app/session/agent/permission events and owns dialogs/status/widgets.
- Bash tool uses a persistent shell-session concept and policy prompt text; the implementation leans more on agent instructions and permission requests than hard sandboxing.

What Capo should adopt:

- Small services with typed interfaces are a good shape for Capo's Rust modules.
- SQLite read models for sessions/messages/files are a useful v0 complement to an event log.
- Per-session concurrency guard and cancellation handles should exist in Capo's runtime/session manager.
- Permission requests should be durable Capo events, not just in-memory channels. The request/decision/grant shape is useful.
- Provider adapters should normalize event streams, but Capo should prefer agent adapters over direct model-provider calls for the first prototype.

What Capo should reject/defer:

- Do not block permission requests only on an in-memory channel; restart recovery needs persisted pending approvals.
- Do not make TUI state the controller state.
- Do not rely on prompt text to enforce banned commands.
- Treat archived/stale OpenCode code as pattern evidence, not a dependency target.

## Aider

Relevant source paths:

- `aider/main.py`
- `aider/coders/base_coder.py`
- `aider/coders/*`
- `aider/commands.py`
- `aider/io.py`
- `aider/repo.py`
- `aider/history.py`
- `aider/run_cmd.py`
- `aider/watch.py`
- `aider/models.py`

Architecture observed:

- Aider is a single-agent CLI application. `main.py` wires args/config/env, git repo detection, IO, model selection, commands, coder construction, chat history, file watcher, and run loop.
- `Coder` is the central in-process conversation/editing object. It carries tracked editable files, read-only files, repo map, history, summarizer, commands, token/cost state, auto-lint/test settings, edit format, and model config.
- Chat modes and edit formats are implemented by switching coder classes or prompts. `/chat-mode` can switch ask/code/architect/context/help and edit formats.
- History is markdown/file-oriented: input history, chat history, LLM history, and summarization through `ChatSummary`.
- Git is first-class: repo discovery, `.gitignore` checks, dirty-file handling, commits, author/committer attribution, and aider-edited file tracking.
- Shell commands are local subprocess/pexpect helpers with interactive output, not a sandboxed runtime.
- Watch mode monitors file changes and converts AI comments into prompts.

What Capo should adopt:

- Git/worktree awareness and explicit dirty-state handling are central for coding-agent orchestration.
- Read-only file references are useful as a capability/profile field, but enforcement must be stronger than "which files are in prompt context."
- Markdown chat/history is useful for human audit, but operational state should be normalized and queryable.
- Summarization is useful as a derived memory/checkpoint, never as the only durable record.

What Capo should reject/defer:

- Do not put Capo's controller, IO, model selection, command system, and editing strategy into one in-process object.
- Do not make edit formats/chat modes a Capo product abstraction. Aider modes are adapter/subagent behavior.
- Do not use shell execution without explicit process supervision, timeout, output caps, workspace/env policy, and audit events.

## LangGraph Checkpointing Reference

Useful source URLs:

- https://docs.langchain.com/oss/python/langgraph/persistence
- https://docs.langchain.com/oss/python/langgraph/durable-execution
- https://docs.langchain.com/oss/python/langgraph/human-in-the-loop
- https://docs.langchain.com/oss/javascript/langgraph/persistence

Architecture lesson:

- LangGraph's useful contribution for Capo is not graph-agent orchestration itself. It is the explicit checkpointing model: thread IDs, checkpoints after execution steps, pending writes, interrupt/resume, human-in-the-loop approval, time travel, and durable stores.
- Capo should adopt the concept of restartable steps and approval interrupts, but express them as Capo events plus read models. Capo should not require tasks to be LangGraph graphs.
- If Capo later embeds a LangGraph sidecar for a specialized workflow, it should be an adapter/subagent with its own checkpoint IDs mapped into Capo's event log.

## Cross-Product Architecture Lessons

### Module Boundaries

Best pattern: Codex and OpenHands both separate protocol/runtime/provider/state enough to inspect and replace pieces. OpenCode's smaller services are also understandable.

Capo target modules:

- `controller`: orchestration policy, command handling, session lifecycle, approvals, recovery.
- `state`: SQLite event log, transactions, read models, snapshots, migrations.
- `runtime`: local process runner first; later container/SSH/remote runtime.
- `adapter`: Codex CLI, Claude Code CLI, ACP, and future agent adapters.
- `capability`: grants, scopes, permission prompts, revocation, audit.
- `tools`: Capo-exposed tools and wrappers for adapter/runtime tools.
- `ui_api`: CLI/TUI/web/mobile surfaces subscribe to read models and submit command envelopes.
- `memory`: derived, rebuildable memory and summaries with provenance.
- `evaluation`: review/test/outcome records.

### Event, Session, And State Model

Adopt a hybrid event-log plus read-model architecture:

- Append events before or atomically with state transitions.
- Persist raw adapter events separately from normalized Capo events.
- Assign Capo IDs for sessions, turns, items, tool calls, commands, runtime processes, approvals, artifacts, and checkpoints.
- Store external IDs from Codex/ACP/Claude/OpenHands as adapter references.
- Project dashboard state from events, not from live process memory.
- Use snapshots/checkpoints for fast recovery, but keep event provenance.

Minimum event categories:

- `session.created`, `session.started`, `session.paused`, `session.resumed`, `session.stopped`, `session.failed`
- `turn.started`, `turn.completed`, `turn.failed`, `turn.cancelled`
- `adapter.raw_event_observed`, `adapter.status_changed`
- `item.started`, `item.updated`, `item.completed`
- `tool.call_requested`, `tool.call_started`, `tool.call_output`, `tool.call_completed`, `tool.call_failed`
- `runtime.process_started`, `runtime.output_delta`, `runtime.exited`, `runtime.killed`, `runtime.health_changed`
- `permission.requested`, `permission.decided`, `permission.granted`, `permission.revoked`, `permission.expired`
- `artifact.created`, `artifact.updated`
- `checkpoint.created`, `checkpoint.restored`, `checkpoint.failed`
- `summary.created`, `memory.reference_added`
- `review.requested`, `review.completed`, `evidence.recorded`

### Runtime And Process Handling

Capo v0 should be closer to Codex `exec` and OpenHands `ProcessSandboxService` than to Aider's subprocess helper:

- command/args, cwd, workspace roots, env allowlist, secret redaction
- process group/session tracking
- stdout/stderr streaming and bounded retention
- timeout and cancellation token
- health state and heartbeat if adapter supports it
- kill escalation
- log/artifact paths
- PTY optional, not default
- no hard sandbox claims until OS/container enforcement exists

### Permission And Tool Handling

Capo should combine the best pieces:

- Codex-style explicit approval policy, permission profile, filesystem/network policy, and safety decisions.
- Cline/OpenCode-style human permission queues.
- Tool metadata with name/schema/description and structured result.
- Durable pending approvals so restart does not lose blocked work.
- Policy source field: static policy, user approval, adapter-native approval, security subagent, inherited trust, test override.

Initial all-allowed policy is acceptable only if every decision still flows through this boundary and emits audit events.

### Adapter And Provider Boundary

Capo should not start as a direct model provider framework. First target should be agent adapters:

- `CodexExecAdapter`: launches/parses `codex exec --experimental-json`.
- `ClaudeCodeAdapter`: launches/parses supported Claude Code noninteractive/headless output when implementation begins.
- `AcpAdapter`: maps ACP sessions/events/tool/permission concepts into Capo events.

Direct model providers can come later for Capo-native agents. Provider metadata belongs behind adapters until Capo needs its own agent implementation.

### UI And Controller Split

Capo should avoid Cline's controller/webview/task entanglement and OpenCode's TUI-owned app state. UI surfaces should:

- submit command envelopes
- subscribe to read-model/event streams
- render state
- issue approvals/interrupts

They should not own session truth, runtime handles, or policy decisions.

### Persistence And Checkpointing

Recommended v0:

- SQLite event log for operational truth.
- SQLite read models for sessions, turns, items, tool calls, approvals, runtime processes, artifacts, and checkpoints.
- Markdown workpads as human-readable planning/evidence, referenced by IDs/paths in SQLite.
- File artifact store for raw logs/transcripts with redaction metadata.
- Checkpoint records that can represent adapter-native checkpoints, git/worktree checkpoints, summary checkpoints, and Capo read-model snapshots.

Capo should make restart recovery a design invariant:

- pending approvals survive restart
- running processes become `unknown`, `orphaned`, `reconnected`, or `terminated` with audit evidence
- adapter replay/dedupe uses external event IDs plus Capo idempotency keys
- UI can rebuild from persisted state without a live agent process

## Capo-Specific Architecture Decisions To Carry Forward

1. Capo is the controller, not a modeful coding agent.
2. User-facing modes are out of scope for Capo core. Adapter/subagent modes may be represented as metadata.
3. ACP is an adapter boundary, not the Capo domain model.
4. Codex and Claude Code should be first-class adapters, not merely provider examples.
5. Runtime, adapter, provider/auth, capability policy, state store, memory, and UI must remain separate.
6. Tool calls and permission requests must be auditable even before enforcement is strict.
7. Capo's event log is authoritative; external streams are inputs.
8. Checkpoints are state-management primitives, not necessarily LangGraph graphs.
9. Local process runtime comes first; Docker/remote runtime comes after local recovery works.
10. Licensing is compatible for study across these repos, but code reuse needs per-file review, especially OpenHands `enterprise/`.

## Open Questions For Architecture A2/A3/A5

- What is Capo's exact event identity scheme for adapter raw events that lack stable IDs?
- Should raw adapter events be stored inline in the event log or in a side table/artifact store referenced by normalized events?
- How much of Codex JSONL should be preserved verbatim for replay/debug?
- Should v0 runtime recovery attempt process reattach, or conservatively mark live child processes as orphaned after Capo restart?
- What is the minimum approval persistence schema for all-allowed v0 that does not paint Capo into a corner?
- Should checkpoint records point to git commits/worktree snapshots from v0, or start with event/read-model snapshots only?
