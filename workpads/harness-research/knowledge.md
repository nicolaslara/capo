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

## Codex Goals As A Harness Pattern

OpenAI's current Codex Goals documentation makes `/goal` directly relevant to
Capo. It describes a Goal as a durable objective for long-running work, with
set/view/pause/resume/clear controls, explicit validation, and a stop condition.
The feature is exposed as a Codex slash command, not as a top-level `codex`
subcommand. Local observation on 2026-05-28 matched that shape: `codex --help`
and `codex exec --help` did not list a `goal` subcommand.

Documented Codex mechanics:

- A Goal is persisted thread state, not global memory and not project-level
  instructions.
- The state records objective, lifecycle, budget, and progress accounting.
- States include active, paused, complete, and budget-limited.
- Continuation is event-driven, not a simple local loop.
- Codex checks continuation only at safe boundaries: after a turn finishes,
  when no other work is pending, no user input is queued, and the thread is
  idle.
- The dispatcher is intentionally conservative: plan-only work should not
  trigger continuation, interruptions pause the objective, and a continuation
  that makes no tool call suppresses the next automatic continuation to avoid
  spin.
- Completion requires an audit against concrete evidence: files, commands,
  tests, benchmark output, generated artifacts, or research citations.
- Budget exhaustion is not completion. The system should stop substantive work,
  summarize progress and blockers, and identify the next useful step.
- Lifecycle authority is bounded. The model can mark an existing goal complete
  only when the evidence supports completion; pause/resume/clear and
  budget-limited transitions remain user- or system-controlled.

Public issue and reverse-engineering evidence adds useful failure modes but
should be treated as lower confidence than official docs:

- GitHub issue reports around `/goal` describe cases where compaction can lose
  or weaken the active-goal continuation prompt, especially the requirement to
  audit completion against actual current state.
- The proposed remedy in that public discussion is to reattach goal context
  from persisted state after compaction, rather than trusting a compacted
  summary to preserve the objective and audit contract.
- Other issues show discoverability and surface-parity problems: `/goal` existed
  in CLI builds before all docs and desktop surfaces exposed the same controls.

Capo implications:

- Treat the objective as structured Capo state and reinject it after resume,
  compaction, adapter restart, or provider transcript loss.
- Keep a requirement/evidence ledger outside the model transcript. The final
  answer is not the proof; the proof is the ledger of files, commands, tests,
  review findings, and citations.
- Make completion a state transition guarded by evidence, not by model
  confidence.
- Add an explicit no-progress guard. If a continuation does not inspect or
  change anything material, Capo should stop automatic continuation until the
  user or planner changes strategy.
- Feature-probe provider-native goal support. Do not assume `/goal` is available
  across Codex CLI, desktop, mobile, remote, or future API surfaces.

## Where Capo's Loop Lives

The loop should live in the Capo server/controller, not in the CLI, not in ACP,
not inside an adapter, and not solely inside a vendor-specific agent.

That loop is not a tight `while true`. It is an event-driven continuation
dispatcher over Capo-owned state:

1. A `CapoGoal` record stores the objective, success criteria, constraints,
   budget, lifecycle state, progress summary, and evidence requirements.
2. Capo records events such as `TurnFinished`, `PermissionRequested`,
   `PermissionResolved`, `RuntimeIdle`, `UserInputQueued`, `BudgetUpdated`,
   `CheckpointCreated`, and `VerificationResult`.
3. The continuation scheduler wakes only at safe boundaries: active goal,
   runtime idle, no queued user input, no pending permission, no conflicting
   workspace lock, budget available, and no recent no-progress suppression.
4. The planner builds the next command envelope: target session, adapter mode,
   context packet, continuation prompt, allowed tools, permission profile, and
   requested validation.
5. The runtime runner and agent adapter execute that envelope and stream raw
   provider updates back into Capo.
6. The auditor updates the requirement/evidence ledger and chooses the next
   state: continue, paused, blocked, budget-limited, or complete.

The CLI, dashboard, mobile UI, or remote-control surface should only create
commands and display state. They should not own continuation policy. ACP should
remain a transport/protocol adapter. It can carry prompts, updates, tool calls,
and permission requests, but Capo should still own goal lifecycle and evidence.

Provider-native loops are nested loops. Codex `/goal`, Claude Code routines,
OpenCode subagents, or future provider task modes can be useful execution
strategies, but Capo should treat them as delegated runtimes:

- Default mode: Capo-owned goal. Capo sends ordinary turns with Capo's
  continuation context and verifies completion itself. This works across
  providers.
- Delegated mode: Capo asks a provider-native goal/task mode to pursue a
  bounded subgoal. Capo still mirrors the objective, watches events, and audits
  completion before marking the Capo goal complete.
- Hybrid mode: Capo uses provider-native goal state for deep local coding while
  preserving a parent Capo goal, checkpoints, permission policy, and external
  evidence ledger.

The design rule is simple: the vendor agent may loop internally, but Capo owns
the outer loop and the decision that the work is actually done.

## Controller And Orchestrator Functions For Capo

Capo's controller/orchestrator should serve these functions:

- Goal lifecycle: create, view, pause, resume, clear, block, budget-limit, and
  complete objectives as durable events.
- Work scheduling: decide when another turn may run, which session gets it, and
  whether to use a primary agent, subagent, or provider-native goal mode.
- Evidence and completion audit: maintain requirement checklists, verification
  commands, manual smoke records, review findings, artifacts, and citations.
- Context assembly: build sourced packets from workpads, architecture docs,
  memory, recent events, files, and evidence without dumping whole transcripts.
- Capability and permission policy: map adapter/tool requests through Capo
  scopes, grants, revocations, approvals, and audit logs.
- Runtime supervision: start, stop, restart, reconnect, and recover agent
  processes, containers, worktrees, terminals, and tunnels.
- Tool mediation: prefer Capo-owned wrappers for file, search, test, patch,
  browser, network, and artifact tools when Capo needs redaction, provenance, or
  policy enforcement.
- Checkpoint and rollback: create reviewable snapshots before broad unattended
  changes and tie them to sessions, turns, and tool events.
- Provider abstraction: feature-probe Codex, Claude Code, OpenCode, ACP agents,
  and others; normalize their events without erasing provider-specific evidence.
- Observability: expose active objective, current turn, pending decisions,
  progress evidence, blockers, budgets, and raw-update links to every client.
- Stop policy: suppress spin, surface repeated blockers, stop on unsafe state,
  and require user input when the next action would exceed the granted scope.

## Harness Comparison Notes

| System | What matters for Capo | Confidence |
| --- | --- | --- |
| ACP | Good protocol for client-agent turns, tool updates, permission requests, file/terminal capabilities. Not a controller. | High |
| Claude Code | Strong official docs around settings scopes, permissions, hooks, MCP, memory, subagents, remote surfaces, and enterprise controls. Leaked source not used. | High for documented behavior |
| OpenCode | Open-source, server/client split, OpenAPI surface, sessions, permissions, agents/subagents, MCP, default local server. Directly relevant to Capo. | High |
| Codex CLI | Strong safety/control guidance plus `/goal` prior art: durable thread-scoped objectives, event-driven continuation, budgets, and evidence-based completion. | High |
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
3. Add Capo-owned goal/objective state and an event-driven continuation
   dispatcher before relying on provider-native long-running task modes.
4. Treat Codex `/goal` as a useful prior-art and optional delegation mode, not
   as Capo's source of truth.
5. Prioritize Capo-owned tool wrappers before advertising broad ACP file or
   terminal client capabilities.
6. Add checkpoint/rollback semantics before increasing auto-approve or
   unattended source-writing behavior.
7. Treat context selection as a product feature: memory packets should be
   scoped, sourced, and budgeted.
8. Build a verification/evaluation layer around test/lint/smoke/review evidence
   and outcome scoring.
9. Make observability agent-native from the start, with prompt/output raw-data
   policies and redaction states.
10. Include OpenCode and Cline as the closest inspectable product comparables for
   Capo's server/client plus agent-core direction.
