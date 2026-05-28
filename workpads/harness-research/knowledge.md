# Harness Research Knowledge

## Objective

Capture what strong coding-agent harnesses do beyond model prompting, and turn
that into Capo design guidance.

## Executive Answer

ACP is useful and should remain Capo's preferred agent/protocol boundary, but it
is not enough to be the harness.

ACP gives a shared client-agent interaction shape: initialize, authenticate,
create or load sessions, send prompts, stream session updates, report tool
calls, request permissions, and optionally expose file and terminal client
capabilities. That is the right layer for interoperability with editors and
agent CLIs.

The best harnesses do more. They own or explicitly model runtime isolation,
permission policy, tool execution, checkpoints, state recovery, context
selection, memory, evaluation, audit logs, provider identity, multi-agent
delegation, and user-facing control surfaces. If Capo delegates those duties to
ACP, Capo becomes a thin client. If Capo owns them and uses ACP as an adapter,
Capo remains the control plane.

## Source Policy

Claude Code source has reportedly been leaked. This spike does not inspect or
depend on leaked proprietary source. Claude Code findings come from official
Anthropic documentation and public secondary reports only. OpenCode, Gemini CLI,
OpenHands, SWE-agent, Aider, Cline, and Goose are covered through public docs
and open-source project surfaces.

## What ACP Covers Well

- Session lifecycle between a client and an agent.
- Prompt turns and streaming progress updates.
- Tool-call reporting with stable tool-call IDs inside a session.
- Permission request/response UX primitives.
- Optional client-side file and terminal capabilities.
- Capability negotiation and protocol extensibility.

Capo implication:

- Use ACP to normalize external agent streams and to interoperate with ACP
  clients/agents.
- Do not make ACP updates directly authoritative. Keep the existing Capo event
  log, raw-update retention, replay/dedupe, and projection model.

## What ACP Does Not Provide

- A durable controller state machine for task/session/run/turn ownership.
- Runtime sandboxing, process supervision, restart recovery, or artifact policy.
- Provider identity, subscription-vs-API credential policy, or enterprise auth
  enforcement.
- Tool wrapper execution, output redaction, and audited permission grants.
- Project memory, context-packet construction, or stale-source invalidation.
- Rollback/checkpoint semantics for code changes.
- Verification loops, benchmark harnesses, lint/test repair, or outcome scoring.
- Multi-agent scheduling, subagent trees, queueing, resource budgets, or
  conflict control.
- Cross-client product surfaces such as dashboards, mobile, voice, CI, or
  remote-control workflows.

Capo implication:

- Keep ACP below the `AgentAdapter` boundary.
- Keep controller, runtime, tool, memory, capability, state, and evaluation as
  Capo-owned records and policies.

## Best-Known Harness Practices

### 1. Make The Harness The Source Of Truth

Strong harnesses do not treat the model's transcript as authoritative state.
They maintain typed sessions, messages, tool events, artifacts, permissions,
and run status. OpenCode exposes a server with sessions, messages, diffs,
permissions, status, and event streams. Cursor exposes checkpoints, queueing,
rules, MCP, CLI, and cloud-agent handoff. Capo is already on the right path
with server-owned state and CLI-through-server control.

Design rule for Capo:

- Every agent observation becomes raw adapter data plus normalized Capo events.
- Read models rebuild from Capo events, not vendor transcript state.

### 2. Separate Runtime From Protocol

OpenHands and SWE-agent make the runtime environment a first-class component.
OpenHands recommends Docker sandboxes for isolation and reproducibility.
SWE-agent/SWE-ReX places a shell session inside a managed deployment, often a
Docker container or remote runtime. ACP can describe terminal and file methods,
but it does not enforce the runtime boundary.

Design rule for Capo:

- `RuntimeRunner` owns process/container lifecycle.
- `AgentAdapter` maps protocol and provider events.
- `ConnectivityTunnel` only owns reachability.

### 3. Permissions Are Policies, Not UI Prompts

ACP has permission request primitives. OpenCode has allow/ask/deny permission
rules with per-tool and per-agent overrides, external-directory guards, and
loop detection. Claude Code has hierarchical managed/user/project/local
settings, managed-only security settings, permission rules, hooks, and MCP
allow/deny controls. Codex safety guidance emphasizes sandbox plus approvals,
managed network policy, identity controls, and agent-native audit logs.

Design rule for Capo:

- Store durable permission requests, decisions, grants, and revocations.
- Treat adapter permission options as input to Capo policy, not the policy.
- Prefer wrapper execution for governed tools.

### 4. Tool Design Is Part Of Agent Quality

SWE-agent's Agent-Computer Interface work is the clearest lesson: specialized
file viewers, concise search output, syntax checks on edit, and explicit empty
output messages materially change performance. Aider similarly leans on repo
maps, lint/test loops, and edit formats. Mature harnesses optimize the
agent-computer interface, not only the model.

Design rule for Capo:

- Build small, typed tools with narrow output.
- Make file/search/test tools summarize for decision quality, not just expose
  raw shell.
- Record tool input/output as artifacts with confidence and redaction state.

### 5. Checkpoints And Rollback Lower Autonomy Risk

Cursor and Cline both make checkpoints a product primitive. Cline uses a shadow
Git repository and captures snapshots after tool use. Cursor stores local
checkpoints separate from Git. This makes higher-autonomy execution safer
because mistakes can be reviewed and rolled back without corrupting the user's
real history.

Design rule for Capo:

- Add Capo-owned checkpoints before broad auto-approve behavior.
- Keep checkpoints separate from Git commits.
- Tie every checkpoint to session/run/turn/tool events and artifact hashes.

### 6. Context And Memory Must Be Scoped

Claude Code and Cursor both formalize project instructions and path-scoped
rules. Aider warns against adding entire repositories and uses repo maps and
ignore files to control context. OpenCode uses project `AGENTS.md`
initialization and specialized agents. The pattern is consistent: scoped
context beats dumping everything into the model.

Design rule for Capo:

- Keep markdown-backed project memory, but inject only fractional, sourced
  packets.
- Prefer path/task-scoped context over broad global memory.
- Track source hashes and invalidation.

### 7. Verification Is A Harness Duty

Aider can auto-lint and auto-test after edits. SWE-bench evaluates patches in
Docker and records per-instance logs and resolution metrics. OpenHands SDLC docs
emphasize tests, coverage, PR review, release notes, and rollback checks. The
harness should help the agent close the loop, not merely produce code.

Design rule for Capo:

- Treat verification commands, test logs, review findings, and manual smoke
  results as first-class evidence.
- Build evaluation/outcome reports from Capo events and artifacts.

### 8. Observability Is Agent-Native

Codex safety guidance argues that traditional process logs are not enough:
operators also need prompts, tool decisions, tool results, MCP usage, and
network policy decisions. Cline offers optional OpenTelemetry integration.
OpenCode exposes global event streams. Good harnesses make agent behavior
inspectable in the agent's own terms.

Design rule for Capo:

- Emit agent-native telemetry: goal, prompt hash/policy, plan, tool decisions,
  tool outputs, permission decisions, network/runtime policy decisions, and
  evidence refs.
- Keep raw prompts and provider output bounded, redacted, or hash-only where
  policy requires it.

### 9. Multi-Agent Is Useful Only With Session Structure

Claude Code, OpenCode, Cline, Cursor, and Roo Code all expose some form of
subagents, background agents, modes, agent teams, or cloud agents. The common
mistake is treating this as just "spawn more agents." The useful pattern is a
session tree with parent/child navigation, scoped tools, separate workspaces or
worktrees, and clear merge/review points.

Design rule for Capo:

- Model subagent sessions explicitly.
- Give each subagent a capability profile, memory packet, workspace/checkpoint,
  and result contract.
- Do not broaden concurrent active sessions until the dashboard/control model
  can steer them safely.

### 10. Public APIs And Multiple Clients Matter

OpenCode is notable because the TUI is a client of a server with OpenAPI and SDK
surfaces. Cursor and Claude Code both span editor, CLI, desktop/web/mobile or
cloud surfaces. Capo's server/control-plane direction matches this.

Design rule for Capo:

- Keep local CLI as one client.
- Design server APIs around product entities: project, task, agent, session,
  run, turn, memory, context, evidence.
- Add ACP exposure later only where it improves interoperability.

## Harness Comparison Notes

| System | What matters for Capo | Confidence |
| --- | --- | --- |
| ACP | Good protocol for client-agent turns, tool updates, permission requests, file/terminal capabilities. Not a controller. | High |
| Claude Code | Strong official docs around settings scopes, permissions, hooks, MCP, memory, subagents, remote surfaces, and enterprise controls. Leaked source not used. | High for documented behavior |
| OpenCode | Open-source, server/client split, OpenAPI surface, sessions, permissions, agents/subagents, MCP, default local server. Directly relevant to Capo. | High |
| Codex CLI | Strong safety/control guidance: sandbox, approvals, network policy, identity, credential stores, rules, OpenTelemetry/compliance logs. | High |
| Cursor | Product docs show tools, checkpoints, queued messages, rules, MCP, ACP mode, CLI worktrees, command approval, and cloud-agent handoff. Internals closed. | Medium-high |
| OpenHands | Sandbox/runtime discipline, Docker isolation, custom images, SDLC/CI workflows. Good for runtime and workflow patterns. | High |
| SWE-agent / SWE-ReX | Best source on ACI design, shell-in-container execution, history processors, custom tools, and benchmark-driven ACI tuning. | High |
| SWE-bench harness | Evaluation discipline: Dockerized patch application, logs, metrics, per-instance results. | High |
| Aider | Repo maps, focused context, git integration, lint/test repair loops, edit formats. | High |
| Cline | Agent core SDK, approval-first UX, checkpoints, shadow Git, MCP, subagents, monitoring options. | High |
| Gemini CLI | Open-source terminal agent with ReAct loop, built-in tools, MCP, checkpointing, and large-context model path. | High |
| Goose | MCP-based extension architecture, built-in developer/memory/task extensions, permissions, ignore files, malware checks. | Medium-high |
| Roo Code | Historical open-source VS Code agent with modes, orchestrator, MCP, auto-approve; extension shut down in 2026, so use only as historical/comparative evidence. | Medium |
| Cursor/Windsurf/Devin-style cloud agents | Useful for worktree/cloud-handoff patterns, but internals are mostly closed. Use public docs only. | Medium-low |

## Other Harnesses To Consider Next

- Continue.dev: open-source IDE assistant; useful for context providers,
  indexing, model/provider abstraction, and enterprise policy.
- Zed Agent: important because Zed is closely associated with ACP client
  adoption; useful for editor/ACP ergonomics.
- JetBrains AI Assistant / Junie: useful for IDE workflow and code-review UX,
  but mostly closed.
- Windsurf Cascade: useful for product-level agent UX and flow-state patterns,
  public-docs only.
- Devin and cloud task agents: useful for remote execution, worktree, PR, and
  async task semantics, public-docs only.
- Sourcegraph Cody: useful for codebase indexing and enterprise code-search
  context, public/open-source components vary.
- GitHub Copilot coding agent: useful for issue-to-PR automation, policy, and
  repository integration, public-docs only.

## Capo Recommendations

1. Keep ACP as an adapter/protocol layer, not the Capo domain model.
2. Keep Capo server/controller authoritative for task, session, run, turn,
   permission, tool, memory, evidence, and recovery state.
3. Prioritize Capo-owned tool wrappers before advertising broad ACP file or
   terminal client capabilities.
4. Add checkpoint/rollback semantics before increasing auto-approve or
   unattended source-writing behavior.
5. Treat context selection as a product feature: memory packets should be
   scoped, sourced, and budgeted.
6. Build a verification/evaluation layer around test/lint/smoke/review evidence
   and outcome scoring.
7. Make observability agent-native from the start, with prompt/output raw-data
   policies and redaction states.
8. Include OpenCode and Cline as the closest inspectable product comparables for
   Capo's server/client plus agent-core direction.
