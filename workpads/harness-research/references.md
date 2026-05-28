# Harness Research References

## Objective

Record the source material used for the 2026-05-28 coding-harness research
spike. Dated claims reflect the observed state on 2026-05-28.

## Primary And Official Sources

### ACP

- https://agentclientprotocol.com/protocol/overview
  - Observed 2026-05-28.
  - Key facts: JSON-RPC communication model; initialize/authenticate;
    session/new, session/load, session/prompt, session/cancel; client-side
    permission, file, and terminal capabilities; session/update notifications.
- https://agentclientprotocol.com/protocol/tool-calls
  - Observed 2026-05-28.
  - Key facts: tool calls are reported through session/update notifications;
    toolCallId is unique within a session; agents may request permission through
    session/request_permission; statuses are pending, in_progress, completed,
    and failed.

### OpenCode

- https://opencode.ai/docs
  - Observed 2026-05-28.
  - Key facts: OpenCode is an open-source AI coding agent available as TUI,
    desktop app, or IDE extension; `/init` creates project `AGENTS.md`.
- https://github.com/anomalyco/opencode
  - Observed 2026-05-28.
  - Key facts: primary open-source repository for OpenCode; useful for future
    source-level comparison after this docs-first spike.
- https://opencode.ai/docs/server
  - Observed 2026-05-28.
  - Key facts: TUI is a client of a server; standalone `opencode serve`;
    OpenAPI 3.1 endpoint; session, message, diff, permission, config, provider,
    file, LSP, formatter, and tool APIs.
- https://opencode.ai/docs/permissions
  - Observed 2026-05-28.
  - Key facts: allow/ask/deny permissions; granular per-tool patterns; external
    directory guard; per-agent overrides; `.env` reads denied by default; loop
    repeat guard.
- https://opencode.ai/docs/agents
  - Observed 2026-05-28.
  - Key facts: primary agents, subagents, built-in Build/Plan/Explore/Scout,
    child session navigation, per-agent permissions and prompts.
- https://opencode.ai/docs/mcp-servers
  - Observed 2026-05-28.
  - Key facts: local and remote MCP support; warns MCP tool context can exceed
    context limits; supports OAuth for remote MCP.

### Claude Code

- https://code.claude.com/docs/en/overview
  - Observed 2026-05-28.
  - Key facts: CLI, editor, desktop, web, JetBrains, CI, Slack, remote-control,
    routines, MCP, hooks, skills, memory, subagents, and Agent SDK surfaces.
- https://code.claude.com/docs/en/settings
  - Observed 2026-05-28.
  - Key facts: managed/user/project/local settings scopes; managed settings can
    enforce security policy; settings include permissions, hooks, MCP allowlists
    and denylists, remote-control disablement, model restrictions, session
    cleanup, auth constraints, and telemetry helpers.
- https://code.claude.com/docs/en/hooks
  - Observed 2026-05-28.
  - Key facts: lifecycle hooks for session, prompt, tool, permission, subagent,
    task, compaction, file, worktree, and config events; PreToolUse hooks can
    block tool calls.
- https://code.claude.com/docs/en/mcp
  - Observed 2026-05-28.
  - Key facts: local stdio, remote HTTP, remote SSE MCP servers; dynamic tool
    updates; reconnection behavior; channels; project-root environment passed to
    stdio MCP servers.
- https://code.claude.com/docs/en/memory
  - Observed 2026-05-28.
  - Key facts: CLAUDE.md and auto memory; memory is context, not enforcement;
    hooks are needed to block actions; path-scoped rules and imports.

Source policy:

- Leaked Claude Code proprietary source was not inspected or used. Public docs
  and non-source public reports may inform lower-confidence comparison only.

### OpenAI Codex

- https://openai.com/index/running-codex-safely/
  - Observed 2026-05-28.
  - Key facts: sandboxing and approvals are paired; managed network policies;
    identity and credential controls; command rules; OpenTelemetry and
    compliance logs for prompts, approval decisions, tool execution, MCP usage,
    and network decisions.
- https://github.com/openai/codex
  - Observed 2026-05-28.
  - Key facts: open-source Codex CLI repository; useful for adapter/source
    comparison in later implementation-specific research.

### Cursor

- https://cursor.com/docs/agent/overview
  - Observed 2026-05-28.
  - Key facts: agent has instructions, tools, and model; tools include codebase
    search, file reads/edits, shell, browser, web, and questions; checkpoints
    store local snapshots; queued and immediate messages.
- https://cursor.com/docs/cli/using
  - Observed 2026-05-28.
  - Key facts: CLI modes, MCP support, ACP server mode, rules, AGENTS.md and
    CLAUDE.md loading, review shortcut, worktree isolation, resume, command
    approvals, and non-interactive mode.
- https://cursor.com/docs/rules
  - Observed 2026-05-28.
  - Key facts: project/user/team/AGENTS.md rules; scoped application through
    globs and descriptions; best practices emphasize focused, actionable,
    scoped rules.
- https://cursor.com/docs/mcp
  - Observed 2026-05-28.
  - Key facts: stdio, SSE, and Streamable HTTP MCP transports; OAuth;
    resources/prompts/tools/roots/elicitation/apps capability support.

### OpenHands

- https://docs.openhands.dev/openhands/usage/sandboxes/docker
  - Observed 2026-05-28.
  - Key facts: Docker sandbox is default and recommended; isolation and
    reproducibility are the reasons; mounts can be read-write or read-only.
- https://github.com/All-Hands-AI/OpenHands
  - Observed 2026-05-28.
  - Key facts: primary open-source repository for OpenHands; useful for future
    source-level runtime and sandbox comparisons.
- https://docs.openhands.dev/openhands/usage/advanced/custom-sandbox-guide
  - Observed 2026-05-28.
  - Key facts: sandbox is where the agent performs tasks instead of running
    commands directly on the host; custom Docker images and runtime dependency
    setup are supported.
- https://docs.openhands.dev/openhands/usage/v0/runtimes/V0_docker
  - Observed 2026-05-28.
  - Key facts: legacy docs still capture runtime hardening lessons: host
    filesystem mounts are dangerous, loopback binding and network isolation are
    recommended for safer deployments.
- https://docs.openhands.dev/openhands/usage/essential-guidelines/sdlc-integration
  - Observed 2026-05-28.
  - Key facts: OpenHands guidance spans planning, development, testing, review,
    deployment, CI/CD, audit logging, and human review requirements.

### SWE-agent / SWE-bench

- https://swe-agent.com/latest/background/architecture/
  - Observed 2026-05-28.
  - Key facts: SWE-agent initializes an environment, local/remote deployment,
    shell session, custom tools, history processing, output parsing, and command
    execution through a container server.
- https://github.com/SWE-agent/SWE-agent
  - Observed 2026-05-28.
  - Key facts: primary open-source repository for SWE-agent; useful for
    source-level ACI and runtime comparison.
- https://swe-agent.com/latest/background/aci/
  - Observed 2026-05-28.
  - Key facts: Agent-Computer Interface quality materially affects results;
    helpful patterns include linting on edit, custom file viewer, concise
    directory search, and explicit empty-output messages.
- https://www.swebench.com/SWE-bench/guides/evaluation/
  - Observed 2026-05-28.
  - Key facts: SWE-bench applies generated patches to real repos and runs tests
    in Docker; outputs include results, per-instance JSONL, and logs.

### Aider

- https://aider.chat/docs/usage/lint-test.html
  - Observed 2026-05-28.
  - Key facts: Aider can lint edited files and run tests after edits, then try
    to fix failures; lint/test commands are configurable.
- https://github.com/Aider-AI/aider
  - Observed 2026-05-28.
  - Key facts: primary open-source repository for Aider; useful for future
    source-level repo-map, edit-format, and git workflow comparison.
- https://aider.chat/docs/faq.html
  - Observed 2026-05-28.
  - Key facts: Aider recommends focused context rather than adding all files;
    repo maps compactly represent the broader codebase; `.aiderignore` and
    subtree-only options help large repos; Aider is open source under Apache
    2.0.

### Cline

- https://docs.cline.bot/cline-overview
  - Observed 2026-05-28.
  - Key facts: Cline has editor and terminal applications, explicit approval for
    actions, agent core SDK, CLI, Kanban, VS Code/JetBrains plugins, ACP mode,
    enterprise governance, and observability.
- https://github.com/cline/cline
  - Observed 2026-05-28.
  - Key facts: primary open-source repository for Cline; useful for source-level
    checkpoint, SDK, and tool-loop comparisons.
- https://docs.cline.bot/core-workflows/checkpoints
  - Observed 2026-05-28.
  - Key facts: checkpoints are enabled by default; a shadow Git repository
    captures snapshots after file edits and commands; restore can affect files,
    task messages, or both.
- https://docs.cline.bot/sdk/overview
  - Observed 2026-05-28.
  - Key facts: Cline SDK is the same harness behind IDE extensions and CLI;
    package boundaries include core, agents, llms, and shared utilities.
- https://docs.cline.bot/enterprise-solutions/monitoring/overview
  - Observed 2026-05-28.
  - Key facts: optional telemetry, prompt storage, and OpenTelemetry integration;
    metrics include task completion, errors, and performance.

### Gemini CLI

- https://developers.google.com/gemini-code-assist/docs/gemini-cli
  - Observed 2026-05-28.
  - Key facts: Gemini CLI is open source, terminal-based, uses a ReAct loop,
    has built-in tools and MCP, supports quotas/API-key usage, and powers a
    subset of Gemini Code Assist agent mode.
- https://github.com/google-gemini/gemini-cli
  - Observed 2026-05-28.
  - Key facts: open-source repository; README lists file operations, shell,
    web fetch, MCP support, checkpointing, custom context files, and GitHub
    workflow automation.

### Goose

- https://goose-docs.ai/docs/getting-started/using-extensions/
  - Observed 2026-05-28.
  - Key facts: extensions are MCP-based; built-in developer, memory, todo,
    task/subagent, app, and extension-manager tools; external extension malware
    checks; permissions and ignore files control access.

### Roo Code

- https://roocodeinc.github.io/Roo-Code/
  - Observed 2026-05-28.
  - Key facts: Roo Code extension shut down on 2026-05-15; historical docs
    remain useful for modes, model-agnostic providers, MCP, auto-approve, and
    orchestrator patterns.

## Lower-Confidence Or Follow-Up Sources

- Cursor, Windsurf, Devin, JetBrains AI Assistant / Junie, and GitHub Copilot
  coding agent are useful product comparables, but their internals are mostly
  closed. Use only official public docs or local observed behavior.
- Continue.dev, Zed Agent, and Sourcegraph Cody deserve a follow-up pass if Capo
  needs deeper evidence on code indexing, editor integration, and ACP client UX.
