# Capo Boundary Definitions

## Objective

Define Capo's system vocabulary and adapter boundaries so implementation can stay modular as runtimes, tunnels, providers, input surfaces, state stores, and memory systems change.

This file is the architecture contract surface for A1. It is concrete enough to scaffold code, but still allows implementation details to evolve during prototype work.

## Design Rules

- Capo is the controller. It is not a modeful coding agent.
- User-facing modes are not a Capo product concept. Adapter/subagent modes may be recorded as metadata.
- Every external system is behind an adapter boundary.
- Capo-owned IDs are authoritative. External IDs are adapter references.
- Capo's event log is authoritative. Raw external streams are inputs.
- UI, voice, and mobile surfaces submit commands and render read models; they do not own orchestration state.
- Tools exposed through Capo should be instrumented wrappers when feasible.
- The local prototype may allow trusted local profile actions broadly, but permission decisions still flow through a policy boundary and produce audit events.
- Prefer static dispatch for the initial Rust scaffold where the variant set is known: `enum AgentAdapter`, `enum RuntimeRunner`, `enum ConnectivityTunnel`, `enum ProviderConnector`, `enum PermissionPolicy`, `enum ToolExposure`, `enum StateStore`, `enum MemoryBackend`, and `enum EvaluationLayer`. Use trait objects only when plugin loading or third-party extension genuinely needs dynamic dispatch.

## Naming Vocabulary

| Term | Meaning |
| --- | --- |
| `Project` | A repo/workspace Capo manages, including workpads and settings. |
| `Task` | A user-visible unit of work, usually backed by a workpad item. |
| `Agent` | A configured worker identity Capo can start, address, and inspect. |
| `Adapter` | Boundary that normalizes a concrete agent/protocol/provider surface into Capo events and commands. |
| `Runtime` | Environment/process runner that executes an adapter or agent process. |
| `Session` | A durable Capo conversation/execution context with one agent. |
| `Run` | One execution attempt within a session, including process/runtime lifecycle. |
| `Turn` | One user/controller prompt or steering command and the resulting agent activity. |
| `Item` | A streamable unit inside a turn: message, plan, tool call, command output, summary, diff, etc. |
| `ToolCall` | An invocation of a Capo-exposed, adapter-native, or runtime tool. |
| `Artifact` | A file/log/report/diff/transcript generated or referenced by a run. |
| `Checkpoint` | A restart/recovery point: event sequence, adapter checkpoint, git/worktree point, or read-model snapshot. |
| `CapabilityProfile` | The set of allowed scopes for a session/run: filesystem, shell, git, network, tools, secrets, voice, browser. |
| `CommandEnvelope` | Normalized command submitted by CLI/dashboard/mobile/voice/API into the controller. |

## Boundary Map

```text
InputSurface
  -> CapoController
    -> StateStore
    -> AgentAdapter
      -> RuntimeRunner
      -> ProviderConnector
      -> ToolExposure
    -> PermissionPolicy
    -> MemoryLayer
    -> EvaluationLayer
    -> ConnectivityTunnel
```

## Cross-Cutting Event Contract

All boundaries communicate durable facts through Capo events.

Minimum event envelope:

```text
CapoEvent {
  event_id,
  sequence,
  occurred_at,
  actor,
  project_id,
  task_id?,
  agent_id?,
  session_id?,
  run_id?,
  turn_id?,
  item_id?,
  kind,
  payload,
  idempotency_key?,
  external_ref?,
  redaction_state,
}
```

Rules:

- `event_id` and `sequence` are Capo-owned.
- `external_ref` stores adapter IDs such as ACP session IDs, Codex item IDs, Claude message IDs, process IDs, and provider request IDs.
- `idempotency_key` is required for replay-prone inputs when the adapter can derive one.
- Raw adapter events may be persisted as artifacts or side records, but normalized Capo events drive read models.
- Read models are rebuildable from events plus referenced artifacts.

## Input Surface

Captures user intent and renders Capo state.

Examples: CLI, TUI, web dashboard, mobile app, voice conversation with Capo.

### Contract

Inputs submit:

```text
CommandEnvelope {
  command_id,
  origin,
  actor_id,
  project_id,
  target,
  intent,
  text?,
  structured_args,
  attachments,
  risk,
  idempotency_key,
}
```

Inputs subscribe to:

- `ProjectReadModel`
- `AgentReadModel`
- `SessionReadModel`
- `TaskReadModel`
- `PermissionQueue`
- `EventStream`

### Responsibilities

- Submit commands and steering messages.
- Query agent/task/session status and summaries.
- Render pending permissions and confirmations.
- Interrupt, pause, resume, or stop through controller commands.
- For voice: maintain conversational context and lower decisions into command envelopes.

### Non-Responsibilities

- Owning session truth.
- Holding runtime process handles.
- Mutating state without the controller.
- Bypassing policy for convenience.

### Failure Modes

- Duplicate command submission after reconnect.
- Stale UI approving an old permission request.
- Voice ambiguity selecting the wrong agent/task.
- Remote/mobile session loss during a privileged confirmation.

### Test Strategy

- Command envelope idempotency tests.
- Read-model subscription replay tests.
- Stale permission approval rejection tests.
- Voice parser fixture tests for status, summary, and steering intents.

## Capo Controller

Owns orchestration policy and authoritative state transitions.

### Contract

Primary operations:

```text
handle_command(CommandEnvelope) -> CommandResult
start_session(ProjectId, AgentId, CapabilityProfileId) -> SessionId
send_turn(SessionId, PromptContent) -> TurnId
interrupt(SessionId | RunId | TurnId) -> InterruptResult
apply_adapter_event(AdapterEvent) -> Vec<CapoEvent>
decide_permission(PermissionRequest) -> PermissionDecision
```

### Responsibilities

- Resolve targets: project, task, agent, session, run, turn.
- Validate command intent, actor, state, and policy.
- Create sessions, runs, turns, permission requests, and checkpoints.
- Dispatch to adapter/runtime/tool/memory/evaluation boundaries.
- Append events and update read models through the state store.
- Recover from restart by reconciling persisted state with runtime status.

### Non-Responsibilities

- Provider-specific auth flows.
- Terminal/PTY implementation details.
- Tunnel connection mechanics.
- Long-term memory ranking internals.
- UI rendering.

### Failure Modes

- Partial side effect before event append.
- Conflicting commands against the same session.
- Adapter event arrives after session cancellation.
- Restart during pending permission or running process.
- Ambiguous target selection from voice or dashboard.

### Test Strategy

- Controller command state-machine tests.
- Crash/restart recovery tests with fake runtime/adapter.
- Concurrent command ordering tests.
- Adapter event normalization golden tests.

## Agent Adapter

Normalizes concrete agents/protocols into Capo commands and events.

Detailed Codex, Claude Code, ACP, provider connector, and subscription-backed connector design lives in `protocol-provider.md`.

Initial variants:

```text
AgentAdapter =
  CodexExecAdapter
  | ClaudeCodeAdapter
  | AcpAdapter
  | FakeAdapter
```

### Contract

```text
initialize(AdapterConfig) -> AdapterInfo
build_runtime_request(AdapterSessionConfig, AgentBinding, ProviderConnectorConfig) -> RuntimeRequest
attach_started_process(RuntimeProcessRef, AdapterSessionConfig, AdapterConfig) -> ExternalSessionRef
send_turn(ExternalSessionRef, AdapterPrompt) -> AdapterTurnRef
deliver_tool_result(ExternalSessionRef, AdapterToolResult) -> DeliveryResult
cancel(ExternalSessionRef, AdapterTurnRef?) -> CancelResult
stream_events(ExternalSessionRef) -> AdapterEventStream
shutdown(ExternalSessionRef) -> ShutdownResult
```

### Responsibilities

- Build runtime requests and attach to runtime-started agent surfaces.
- Translate Capo prompts/commands into adapter-specific messages.
- Normalize adapter output into `AdapterEvent`.
- Surface adapter-requested tool calls as events and accept Capo tool results back when the underlying adapter supports it.
- Preserve external IDs and raw event metadata for replay/dedupe.
- Surface adapter capabilities, version, auth mode, and known limitations.

### Non-Responsibilities

- Owning Capo session IDs.
- Deciding durable permission policy.
- Managing tunnels directly.
- Writing read models.
- Treating provider-specific state as Capo truth.

### Failure Modes

- CLI output schema changes.
- Adapter emits events without stable IDs.
- Adapter requests a tool call but cannot accept structured tool results.
- Authentication expires mid-run.
- Agent process exits while Capo thinks it is active.
- ACP `session/load` replay duplicates previously seen updates.

### Test Strategy

- Golden transcript tests for Codex JSONL, Claude output, and ACP JSON-RPC.
- Adapter version/capability snapshot tests.
- Replay/dedupe fixture tests with repeated raw events.
- Fake adapter e2e tests for controller/runtime integration.

## Runtime Runner

Executes local or remote agent processes.

Detailed local runtime, remote runtime, tunnel separation, lifecycle, and recovery design lives in `runtime-tunnel.md`.

Initial variants:

```text
RuntimeRunner =
  LocalProcessRunner
  | RemoteProcessRunner
  | ContainerRunner
  | FakeRuntimeRunner
```

Only `LocalProcessRunner` and `FakeRuntimeRunner` are prototype requirements.

### Contract

```text
prepare(RuntimeTarget) -> RuntimePrepared
start_process(RuntimeRequest) -> RuntimeStartResult
write_stdin(RuntimeProcessRef, Bytes) -> WriteResult
interrupt(RuntimeProcessRef, InterruptKind) -> InterruptResult
terminate(RuntimeProcessRef, TerminationKind) -> TerminateResult
kill(RuntimeProcessRef) -> KillResult
stream_output(RuntimeProcessRef) -> RuntimeOutputStream
health(RuntimeProcessRef) -> RuntimeHealth
cleanup(RuntimeProcessRef, CleanupPolicy) -> CleanupResult
```

### Responsibilities

- Start programs with explicit argv, launch mode, cwd, workspace roots, environment allowlist, and redaction policy.
- Track process group/session and child process cleanup.
- Capture stdout/stderr/PTY output with bounded retention.
- Report exit status, health, and liveness.
- Implement kill escalation.

### Non-Responsibilities

- Claiming sandboxing it does not enforce.
- Provider authentication.
- Capo task/session policy.
- Parsing adapter protocol semantics.

### Failure Modes

- Child process escapes process group.
- Output contains secrets before redaction.
- Process survives Capo restart.
- PTY behavior differs from stdio behavior.
- Workspace path policy is bypassed by shell commands.

### Test Strategy

- Fake process runner tests.
- Spawn/interrupt/terminate/kill smoke tests.
- Output cap and redaction tests.
- Restart reconciliation tests for orphaned processes.

## Connectivity / Tunnel

Connects Capo to remote runtimes and clients.

Detailed endpoint, exposure, local loopback, SSH, Tailscale, reverse-tunnel, and runtime separation design lives in `runtime-tunnel.md`.

Initial variants:

```text
ConnectivityTunnel =
  LocalLoopback
  | SshTunnel
  | TailscaleTunnel
  | ReverseTunnel
  | FakeTunnel
```

Prototype requirements: `LocalLoopback` and `FakeTunnel`.

### Contract

```text
resolve_endpoint(ConnectivityEndpoint, EndpointOwner, ChannelKind) -> ResolvedEndpoint
check_reachability(ConnectivityEndpoint) -> ConnectivityHealth
open_channel(ResolvedEndpoint) -> ChannelRef
close_channel(ChannelRef) -> CloseResult
exposure_report(ConnectivityEndpoint) -> ExposureReport
```

### Responsibilities

- Reachability and routing.
- Authentication metadata and policy hooks.
- Health/status of private connectivity.
- Clear separation between reachability and runtime execution.

### Non-Responsibilities

- Agent task state.
- Provider auth/session management.
- Memory, evaluation, or tool semantics.

### Failure Modes

- Lost tunnel during active run.
- Stale remote endpoint identity.
- Public exposure by mistake.
- Remote runtime unreachable during recovery.

### Test Strategy

- Local loopback health tests.
- Fake tunnel lost/recovered tests.
- Endpoint identity validation fixtures.
- Public exposure policy checks before enabling reverse/funnel modes.

## Provider Connector

Represents model/provider/subscription metadata behind adapters.

Detailed provider records, credential scopes, auth metadata, usage observation, and subscription policy live in `protocol-provider.md`.

Initial variants:

```text
ProviderConnector =
  CodexSubscriptionConnector
  | ClaudeSubscriptionConnector
  | OpenAiApiConnector
  | AnthropicApiConnector
  | LocalModelConnector
  | UnknownProviderConnector
  | FakeProviderConnector
```

### Contract

```text
describe_provider() -> ProviderInfo
auth_status() -> AuthStatus
authorize_connector_use(DeploymentContext, Actor, RuntimeTarget) -> ConnectorUseDecision
usage_snapshot(SessionId?) -> UsageSnapshot?
redaction_rules() -> RedactionPolicy
revocation_instructions() -> RevocationPlan
```

### Responsibilities

- Report non-secret provider/auth metadata.
- Reject disallowed connector use before runtime launch.
- Describe rate limits, cost fields, and capability hints when available.
- Provide revocation instructions.
- Keep credentials in vendor/OS/secret-manager storage.

### Non-Responsibilities

- Reading OAuth tokens or browser cookies.
- Owning Capo sessions.
- Dispatching agent turns directly in v0.

### Failure Modes

- Subscription entitlement changes.
- CLI chooses API key instead of subscription auth due to environment.
- Provider usage/cost unavailable.
- Token/session expires mid-run.

### Test Strategy

- Environment scrubbing tests.
- Auth metadata redaction tests.
- Provider capability snapshot tests.
- Revocation instruction rendering tests.

## Permission Policy

Defines what an agent may do and how decisions are made.

Detailed capability profile, scope, grant, approval, revocation, and ACP permission mapping design lives in `capability-permissions.md`.

Initial variants:

```text
PermissionPolicy =
  AllowTrustedLocalProfilePolicy
  | StaticPolicy
  | UserApprovalPolicy
  | SecurityAgentPolicy
  | FakePermissionPolicy
```

Prototype starts with `AllowTrustedLocalProfilePolicy`, but every decision still emits events.

### Contract

```text
evaluate(PermissionRequest, CapabilityProfile, Context) -> PermissionDecision
grant(CapabilityGrantRequest) -> CapabilityGrant
revoke(CapabilityGrantId, Reason) -> RevocationResult
explain(PermissionDecisionId) -> DecisionExplanation
```

### Responsibilities

- Represent scopes for filesystem, shell, git, network, tools, secrets, browser, and voice transcript access.
- Map adapter-native requests such as ACP `allow_once` / `allow_always` / `reject_once` / `reject_always`.
- Persist request, decision, source, scope, expiry, and revocation.
- Support future static/user/security-agent decision sources.

### Non-Responsibilities

- Implementing OS sandbox mechanics directly.
- Hiding policy decisions inside prompts.
- Treating broad trusted-local permission as "no audit needed."

### Failure Modes

- Permission decision lost on restart.
- Over-broad grant has no expiry or scope.
- Adapter-native approval bypasses Capo policy.
- Security-agent policy is unavailable or slow.

### Test Strategy

- Trusted-local policy still emits decision events.
- Scope/expiry/revocation unit tests.
- ACP permission option mapping tests.
- Restart persistence tests for pending approvals.

Naming note:

- `CapabilityProfile` names the scopes granted to a session/run.
- `PermissionPolicy` names the decision boundary that evaluates requests against a capability profile.
- Avoid naming code as if capability profile data and permission decisions are one boundary.

## Tool Exposure

Defines tools Capo exposes and instruments.

Detailed tool registry, wrapper, ACP client capability, MCP, native-tool observation, and instrumentation design lives in `tool-exposure.md`.

Initial variants:

```text
ToolExposure =
  CapoToolRegistry
  | RuntimeToolWrappers
  | AdapterNativeToolObserver
  | ProviderNativeToolObserver
  | McpToolBridge
  | FakeToolExposure
```

Initial Capo tool set:

- `capo.task_status`
- `capo.agent_status`
- `capo.session_summary`
- `capo.workpad_read`
- `capo.evidence_record`
- `capo.capability_request`

Wrapper tools later:

- shell command wrapper
- git status/diff wrapper
- file read/write wrapper
- memory search wrapper
- test/smoke runner wrapper

### Contract

```text
list_tools(Context) -> Vec<ToolInfo>
describe_tool(ToolId) -> ToolInfo
authorize_tool_call(ToolCallRequest, CapabilityProfile, PolicyContext) -> PermissionDecision
invoke_tool(ToolCallRequest) -> ToolCallResult
observe_external_tool(AdapterToolObservation) -> ToolObservationResult
deliver_tool_result(AdapterSessionRef, ToolCallResult) -> DeliveryResult
```

### Responsibilities

- Provide schema, description, scope, and risk metadata.
- Emit events for requested/started/output/completed/failed tool calls.
- Redact inputs/outputs according to policy.
- Correlate provider-native or adapter-native tools with Capo-visible tool calls when possible.

### Non-Responsibilities

- Reimplementing every native tool in Claude/Codex.
- Silently bypassing provider-native tool safety.
- Treating uninstrumentable native tools as fully observed.

### Failure Modes

- Provider-native tool call is visible only as text/log output.
- Tool output contains secrets.
- Tool call finishes after session cancellation.
- Duplicate tool events during adapter replay.

### Test Strategy

- Fake tool invocation tests.
- Redaction tests.
- Event correlation tests.
- Tool replay/dedupe tests.

### Adapter Tool-Call Loop

The prototype e2e flow should be explicit:

1. `AgentAdapter` emits an `AdapterEvent::ToolCallRequested` with external IDs and raw metadata.
2. `CapoController` maps it to `tool.call_requested` and asks `PermissionPolicy` for a decision.
3. `PermissionPolicy` emits request/decision/grant events, even under `AllowTrustedLocalProfilePolicy`.
4. `ToolExposure` invokes or wraps the requested tool and emits started/output/completed/failed events.
5. `CapoController` persists the result and calls `AgentAdapter.deliver_tool_result(...)` when the adapter can accept structured results.
6. If the adapter cannot accept structured results, Capo records the result as observed-only and exposes that limitation in the session read model.

## State Store

Persists operational truth.

Detailed entity, event, read-model, SQLite, artifact, and restart-recovery design lives in `state-model.md`.

Prototype store:

- SQLite event log
- SQLite read models
- file artifact store for logs/raw transcripts
- markdown workpads referenced by path

Initial variants:

```text
StateStore =
  SqliteStateStore
  | InMemoryStateStore
  | FakeStateStore
```

### Contract

```text
append_event(CapoEvent) -> Sequence
append_events(Vec<CapoEvent>) -> SequenceRange
load_events(EventQuery) -> Vec<CapoEvent>
update_read_models(SequenceRange) -> ProjectionResult
read_model(Query) -> ReadModelResult
record_artifact(ArtifactRecord) -> ArtifactId
checkpoint(CheckpointRecord) -> CheckpointId
```

### Responsibilities

- Transactional event append.
- Rebuildable read models.
- Idempotency enforcement.
- Artifact pointers and content hashes.
- Migration and schema versioning.
- Backup/export path.

### Non-Responsibilities

- Deciding policy.
- Ranking memory.
- Owning markdown as hidden state.

### Failure Modes

- Event append succeeds but projection fails.
- Duplicate idempotency key.
- Markdown and SQLite facts diverge.
- Artifact path points to missing file.
- Migration breaks old state.

### Test Strategy

- Migration tests.
- Event append/projection transaction tests.
- Idempotency tests.
- Rebuild read models from events.
- Export/import smoke tests.

## Memory Layer

Stores distilled reusable context, separate from operational state.

Initial variants:

```text
MemoryBackend =
  MarkdownMemory
  | SQLiteFtsMemory
  | ExternalMemoryAdapter
```

Prototype starts with markdown pointers plus optional SQLite FTS later.

### Contract

```text
ingest(MemorySourceRef) -> MemoryRecordId
search(MemoryQuery) -> Vec<MemoryHit>
build_packet(TaskContext, MemoryBudget) -> MemoryPacket
explain(MemoryHitId) -> MemoryExplanation
invalidate(MemoryRecordId, Reason) -> InvalidationResult
```

### Responsibilities

- Keep derived memory rebuildable from events/files.
- Preserve provenance, confidence, scope, and invalidation metadata.
- Build task-specific memory packets.
- Prevent raw voice transcripts or secrets from entering long-term memory by default.

### Non-Responsibilities

- Operational event recovery.
- Replacing workpads as human-readable source.
- Sending project memory to external SaaS without explicit connector policy.

### Failure Modes

- Memory poisoning from unreviewed summaries.
- Stale facts retrieved as current.
- Prompt bloat.
- External memory leaks project data.

### Test Strategy

- Provenance-required tests.
- Invalidated fact exclusion tests.
- Memory packet budget tests.
- Redaction/no-raw-voice-transcript tests.

## Evaluation Layer

Assesses agent outcomes.

Initial variants:

```text
EvaluationLayer =
  LocalEvaluationLayer
  | FakeEvaluationLayer
```

### Contract

```text
record_evidence(EvidenceRecord) -> EvidenceId
request_review(ReviewRequest) -> ReviewId
record_review(ReviewResult) -> ReviewId
score_run(RunId, Criteria) -> EvaluationResult
summarize_performance(ProjectId | AgentId | TaskId) -> PerformanceSummary
```

### Responsibilities

- Store tests, smoke evidence, manual review, confidence, blockers, and outcomes.
- Compare acceptance criteria to evidence.
- Feed performance summaries and routing recommendations.
- Keep evaluation separate from agent execution.

### Non-Responsibilities

- Deciding implementation strategy.
- Mutating work without controller command.
- Acting as long-term memory without provenance.

### Failure Modes

- Evidence record references missing artifact.
- Review result attached to wrong task/session.
- Automated score overstates completion.
- Agent performance summary ignores failed hidden work.

### Test Strategy

- Evidence/artifact linkage tests.
- Review attachment tests.
- Completion audit fixtures.
- Performance summary aggregation tests.

## Static Dispatch Guidance

Use static dispatch for known in-tree boundaries in the first scaffold:

```text
enum AgentAdapter { CodexExec(CodexExecAdapter), ClaudeCode(ClaudeCodeAdapter), Acp(AcpAdapter), Fake(FakeAdapter) }
enum RuntimeRunner { LocalProcess(LocalProcessRunner), RemoteProcess(RemoteProcessRunner), Container(ContainerRunner), Fake(FakeRuntimeRunner) }
enum ConnectivityTunnel { LocalLoopback(LocalLoopbackTunnel), Ssh(SshTunnel), Tailscale(TailscaleTunnel), Reverse(ReverseTunnel), Fake(FakeTunnel) }
enum ProviderConnector { CodexSubscription(CodexSubscriptionConnector), ClaudeSubscription(ClaudeSubscriptionConnector), OpenAiApi(OpenAiApiConnector), AnthropicApi(AnthropicApiConnector), LocalModel(LocalModelConnector), Unknown(UnknownProviderConnector), Fake(FakeProviderConnector) }
enum PermissionPolicy { AllowTrustedLocalProfile(AllowTrustedLocalProfilePolicy), Static(StaticPolicy), UserApproval(UserApprovalPolicy), SecurityAgent(SecurityAgentPolicy), Fake(FakePermissionPolicy) }
enum ToolExposure { Capo(CapoToolRegistry), Runtime(RuntimeToolWrappers), AdapterNative(AdapterNativeToolObserver), ProviderNative(ProviderNativeToolObserver), Mcp(McpToolBridge), Fake(FakeToolExposure) }
enum StateStore { Sqlite(SqliteStateStore), InMemory(InMemoryStateStore), Fake(FakeStateStore) }
enum MemoryBackend { Markdown(MarkdownMemory), SqliteFts(SqliteFtsMemory) }
enum EvaluationLayer { Local(LocalEvaluationLayer), Fake(FakeEvaluationLayer) }
```

Why:

- Keeps control flow explicit and readable.
- Makes match arms reveal missing boundary handling.
- Avoids early plugin/dynamic-dispatch complexity.
- Works well while Capo's first adapters are known: Claude Code, Codex, ACP, fake test adapters.

Controller composition should start as an owned dependency bundle:

```text
CapoController {
  state: StateStore,
  adapters: AgentAdapterRegistry,
  runtimes: RuntimeRunnerRegistry,
  tunnel: ConnectivityTunnel,
  providers: ProviderConnectorRegistry,
  policy: PermissionPolicy,
  tools: ToolExposure,
  memory: MemoryBackend,
  evaluation: EvaluationLayer,
}
```

Registries can be simple static maps keyed by configured IDs in the prototype. They do not need plugin loading.

Use trait objects later when:

- Third-party plugin loading is real.
- Adapter set is not known at compile time.
- Dynamic composition materially simplifies a subsystem.

## A1 Gate Evidence

A1 is complete when:

- Every boundary has a contract.
- Every boundary lists responsibilities and non-responsibilities.
- Every boundary lists failure modes.
- Every boundary lists test strategy.
- Static-dispatch guidance is recorded for the implementation scaffold.
