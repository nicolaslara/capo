# Capo Tool Exposure And Instrumentation

## Objective

Define the prototype tool architecture: which Capo tools exist first, how Capo instruments tool calls, how wrapper tools relate to existing agent tools, and how ACP client capabilities, MCP servers, local runtime tools, and provider-native tools map into one auditable model.

This is the A5a architecture artifact. It keeps Capo as the control plane for tool visibility and policy while accepting that some first adapters expose only partial native-tool detail.

## Design Rules

- Capo-exposed tools are controller capabilities, not agent identities.
- Every tool call that Capo can see becomes a `ToolCall` record and durable event sequence.
- Tool execution is gated by `PermissionPolicy`, even when the trusted local profile allows it.
- Tool wrappers should be thin, typed, and auditable. They should call existing runtime/filesystem/git/workpad/state APIs instead of duplicating business logic.
- Agent-native and provider-native tools are never marked fully governed unless Capo either executes them through a wrapper or receives enough structured lifecycle data to prove what happened.
- Unstructured or partial tool observations are recorded as `observed_only` with instrumentation confidence.
- ACP filesystem and terminal client handlers are tool-facing surfaces; they must still pass through Capo capability and tool instrumentation.
- MCP is an exposure mechanism, not the internal Capo tool model. Capo may later publish tools through MCP or pass configured MCP servers to agents, but v0 stores tool identity in Capo records.
- Raw tool inputs/outputs that may contain secrets become artifacts with redaction state, not inline event blobs.

## Static Dispatch Shape

Prototype enum:

```text
enum ToolExposure {
  Capo(CapoToolRegistry),
  Runtime(RuntimeToolWrappers),
  AdapterNative(AdapterNativeToolObserver),
  ProviderNative(ProviderNativeToolObserver),
  Mcp(McpToolBridge),
  Fake(FakeToolExposure),
}
```

Prototype implementation order:

1. `FakeToolExposure` for controller/adapter e2e tests.
2. `CapoToolRegistry` for read-only status/summary/workpad tools plus evidence recording.
3. `RuntimeToolWrappers` for shell/git/filesystem wrappers that call `RuntimeRunner` or local workspace APIs.
4. `AdapterNativeToolObserver` for Codex/Claude/ACP structured tool updates.
5. `McpToolBridge` only after the prototype can track native Capo tool calls end to end.

## Tool Categories

| Category | Origin | Capo executes? | Prototype handling |
| --- | --- | --- | --- |
| Capo tools | `capo` | Yes | Fully instrumented typed handlers. |
| Runtime wrapper tools | `runtime` | Yes, through `RuntimeRunner` or workspace APIs | Fully instrumented when wrapper is used. |
| ACP client tools | `capo` or `runtime` | Yes, when advertised | Map `fs/*` and `terminal/*` requests into wrappers. |
| MCP tools provided by Capo | `capo` | Later | Deferred publication surface over the same registry. |
| MCP tools passed to agents | `adapter_native` | Usually no | Record config and observed updates when adapter reports them. |
| Adapter-native tools | `adapter_native` | No, unless routed back to Capo | Observe structured lifecycle when available; otherwise observed-only. |
| Provider-native tools | `provider_native` | No | Observe usage only when adapter/provider exposes safe metadata. |

## First Capo Tools

These tools are enough for a dogfood-oriented controller loop before broad filesystem/shell control:

| Tool ID | Purpose | Scope | Mutates state? |
| --- | --- | --- | --- |
| `capo.task_status` | Return task status, active sessions, evidence refs, and blockers. | `tool:invoke:capo.task_status`, `state:read:task`, `state:read:session`, `state:read:evidence`. | No |
| `capo.agent_status` | Return agent health, adapter/runtime/provider binding, current run, and limitations. | `tool:invoke:capo.agent_status`, `state:read:agent`, `state:read:session`, `state:read:runtime`, `state:read:provider`. | No |
| `capo.session_summary` | Return latest session summary, active turn, recent tool calls, and pending permissions. | `tool:invoke:capo.session_summary`, `state:read:session`, `state:read:tool`, `state:read:permission_queue`. | No |
| `capo.project_memory_read` | Read markdown-backed project memory sections by path/heading with source authority labels. | `tool:invoke:capo.project_memory_read`, `filesystem:read:workspace`, `state:read:task`. | No |
| `capo.workpad_read` | Read selected workpad sections by path/heading with source authority labels. | `tool:invoke:capo.workpad_read`, `filesystem:read:workspace`, `state:read:task`. | No |
| `capo.evidence_record` | Attach test/review/manual evidence to a task/session/run. | `tool:invoke:capo.evidence_record`, `state:write:evidence`, `state:read:task`. | Yes |
| `capo.capability_request` | Request a scoped capability change or grant review. | `tool:invoke:capo.capability_request`, `state:read:capability`, `state:write:capability_request`. | Yes |

State scopes carry `resource_ref` constraints such as project ID, task ID, agent ID, session ID, run ID, or permission queue ID. A broad `tool:invoke:*` grant is not enough to read arbitrary Capo state.

Deferred but expected soon:

- `capo.memory_search`
- `capo.git_status`
- `capo.git_diff`
- `capo.shell_run`
- `capo.file_read`
- `capo.file_write`
- `capo.test_run`

The deferred tools should be added as wrappers only when the corresponding runtime/state/artifact paths are implemented enough to produce durable events and redacted artifacts.

## Core Records

### ToolDefinition

Registered callable surface.

Fields:

- `tool_definition_id`
- `tool_id`
- `display_name`
- `origin`: `capo`, `runtime`, `adapter_native`, `provider_native`, `mcp`
- `handler_kind`: `capo_registry`, `runtime_wrapper`, `adapter_observer`, `provider_observer`, `mcp_bridge`, `fake`
- `schema_json`
- `required_scopes_json`
- `risk`: `low`, `medium`, `high`, `critical`
- `redaction_policy_json`
- `exposure`: `internal`, `agent_visible`, `input_surface_visible`, `mcp_visible`
- `instrumentation_level`: `full`, `structured_observed`, `text_observed`, `none`
- `status`: `available`, `disabled`, `unhealthy`
- `created_at`
- `updated_at`

### ToolInvocation

Execution attempt for a visible tool call.

Fields:

- `tool_invocation_id`
- `tool_call_id`
- `tool_definition_id?`
- `session_id?`
- `run_id?`
- `turn_id?`
- `adapter_config_id?`
- `provider_connector_id?`
- `runtime_process_ref_id?`
- `external_tool_ref?`
- `actor_id`
- `subject`
- `permission_decision_id?`
- `capability_grant_use_id?`
- `correlation_id`
- `instrumentation_level`
- `input_artifact_id?`
- `output_artifact_id?`
- `status`
- `started_at?`
- `completed_at?`

`ToolInvocation` is the execution projection; `ToolCall` remains the user/session timeline record.

### ToolObservation

Structured or partial observation from an adapter/provider/runtime stream.

Fields:

- `tool_observation_id`
- `tool_call_id?`
- `tool_invocation_id?`
- `source`: `adapter_event`, `runtime_output`, `provider_usage`, `mcp_message`, `manual`
- `external_tool_ref?`
- `observed_status?`
- `confidence`: `high`, `medium`, `low`
- `raw_event_id?`
- `artifact_id?`
- `observed_at`

## ToolExposure Contract

Implementation-facing contract:

```text
list_tools(ToolListContext) -> Vec<ToolDefinition>
describe_tool(ToolId) -> ToolDefinition
authorize_tool_call(ToolCallRequest, CapabilityProfile, PolicyContext) -> PermissionDecision
invoke_tool(ToolCallRequest) -> ToolCallResult
observe_external_tool(AdapterToolObservation) -> ToolObservationResult
deliver_tool_result(AdapterSessionRef, ToolCallResult) -> DeliveryResult
```

Rules:

- `authorize_tool_call` creates a `PermissionRequest` and uses `PermissionPolicy`; it does not make policy decisions internally.
- `invoke_tool` runs only after an allow decision is persisted.
- `observe_external_tool` never upgrades a native tool to `full` instrumentation without a registered wrapper or structured lifecycle evidence.
- `deliver_tool_result` calls `AgentAdapter.deliver_tool_result(...)` only when the adapter supports structured result delivery.
- All calls carry a `correlation_id` tying command, turn, permission, tool, artifact, and adapter events together.
- Variants may return `unsupported` for methods outside their role. Registries list and dispatch tools, executors invoke tools, observers record native-tool observations, and publishers/bridges expose tools through transports such as MCP.

## Invocation Lifecycle

1. Agent, Capo, or an input surface requests a tool call.
2. Controller appends `tool.call_requested`; this creates the timeline `ToolCall`.
3. Controller creates a `PermissionRequest` for required scopes; `permission.requested` and `permission.decided` own authorization state.
4. If denied or canceled, controller appends `tool.call_failed`, `tool.call_canceled`, or a denied status and does not invoke the handler.
5. If allowed, `ToolExposure` appends `tool.invocation_started`; this creates `ToolInvocation` with actor, subject, permission decision, grant-use, and correlation IDs.
6. `tool.call_started` is a timeline status event for the user/session view. It does not create the invocation projection.
7. Inputs and outputs become artifacts when non-trivial or sensitive.
8. `ToolExposure` appends `tool.output_artifact_recorded` for artifact metadata and `tool.output_observed` for timeline-visible output.
9. `ToolExposure` appends terminal `tool.call_completed` or `tool.call_failed`.
10. Controller attempts `tool.result_delivered` if the adapter can accept structured results.
11. Read models expose status, artifacts, confidence, permission decision, grant use, actor, and delivery status.

## Wrapper Strategy

### Runtime Wrappers

Runtime wrappers are the preferred way to govern shell and process work:

- `capo.shell_run` builds a `RuntimeRequest`, starts it through `RuntimeRunner`, captures bounded output artifacts, and records exit status.
- `capo.test_run` is a specialized shell wrapper with test/evidence metadata.
- Terminal-like ACP requests map to runtime wrappers and `RuntimeProcessRef` records rather than bypassing runtime supervision.

### Filesystem Wrappers

Filesystem wrappers start as workspace-bound text operations:

- `capo.file_read` canonicalizes paths, records line ranges, and stores large reads as artifacts.
- `capo.file_write` records before/after hashes and diff artifacts.
- ACP `fs/read_text_file` and `fs/write_text_file` map to these wrappers only when Capo advertises the matching client capability.

### Git Wrappers

Git wrappers are separate from generic shell:

- `capo.git_status` and `capo.git_diff` are read-only and low risk.
- `capo.git_commit` commits already-staged changes only, requires an explicit commit message, runs through `RuntimeRunner`, and is high risk.
- `git push` stays out of prototype scope.

### State And Workpad Tools

State/workpad tools are pure controller tools:

- Status tools read SQLite read models.
- `capo.project_memory_read` is the preferred product-language reader for markdown-backed project memory.
- `capo.workpad_read` remains a compatibility alias for current workpad/source-file flows.
- Both readers must keep path constraints, source authority labels, and task/source provenance.
- `capo.evidence_record` appends evidence events and does not silently edit workpad task status.

## ACP Relationship

ACP has two different tool-relevant surfaces:

1. Agent-reported tool calls through `session/update`.
2. Agent-to-client requests such as `session/request_permission`, `fs/read_text_file`, `fs/write_text_file`, and `terminal/*`.

Capo behavior:

- Reported ACP tool calls become `ToolObservation` plus normalized `ToolCall` records.
- `session/request_permission` maps through A3 permission rules.
- Capo advertises ACP `fs` and `terminal` client capabilities only when the selected `CapabilityProfile` and `ToolDefinition` set enable the backing wrappers.
- V0 advertises no ACP `fs` or `terminal` client capabilities until backing `ToolDefinition` rows exist and wrapper tests pass.
- ACP terminal requests are not direct shell execution; they route through `RuntimeRunner`.
- ACP file requests are not raw filesystem access; they route through canonicalized workspace file wrappers.
- MCP server configs passed in ACP session setup are recorded as adapter configuration and capability context. They are not equivalent to Capo exposing its own tool registry.

## MCP Relationship

Prototype stance:

- Capo may pass user-approved MCP server configs to an ACP-compatible agent when the adapter supports it.
- Capo does not need to expose its own tools as MCP in v0.
- When Capo later exposes MCP, the MCP server is a transport over `ToolExposure`, not a second tool registry.
- MCP calls must produce the same `ToolCall`, permission, artifact, and observation records as native Capo tools.

Deferred:

- MCP server lifecycle management.
- MCP auth/secret negotiation.
- MCP over ACP draft v2 message bridging.
- Remote MCP exposure through tunnels.

## Provider And Adapter Native Tools

Codex and Claude Code may execute native tools internally. Capo handles this conservatively:

- If structured JSONL/stream JSON exposes a tool call ID, title, status, input, output, or location, Capo records a structured `ToolObservation`.
- If only text/log output implies a tool ran, Capo may record a low-confidence `ToolObservation` but not a governed `ToolInvocation`.
- Provider-native web/file/shell tools remain outside Capo enforcement unless routed through Capo wrappers.
- Dashboard and evaluation must label observed-only native tools as partial visibility.

## State Model Additions

Add tables:

```text
tool_definitions(tool_definition_id, tool_id, display_name, origin, handler_kind, schema_json, required_scopes_json, risk, redaction_policy_json, exposure, instrumentation_level, status, created_at, updated_at)
tool_invocations(tool_invocation_id, tool_call_id, tool_definition_id, session_id, run_id, turn_id, adapter_config_id, provider_connector_id, runtime_process_ref_id, external_tool_ref_json, actor_id, subject_json, permission_decision_id, capability_grant_use_id, correlation_id, instrumentation_level, input_artifact_id, output_artifact_id, status, started_at, completed_at)
tool_observations(tool_observation_id, tool_call_id, tool_invocation_id, source, external_tool_ref_json, observed_status, confidence, raw_event_id, artifact_id, observed_at)
```

Add events:

- `tool.definition_registered`
- `tool.definition_updated`
- `tool.invocation_started`
- `tool.output_artifact_recorded`
- `tool.observation_recorded`
- `tool.instrumentation_downgraded`

Existing A2 events remain canonical for timeline status:

- `tool.call_requested`
- `tool.call_started`
- `tool.output_observed`
- `tool.call_completed`
- `tool.call_failed`
- `tool.call_canceled`
- `tool.result_delivered`

## Read Model Additions

`AgentReadModel`:

- tool definitions visible to the agent
- current and recent native-tool limitations
- instrumentation confidence summary

`SessionReadModel`:

- ordered tool calls with permission decision, artifacts, output, status, delivery state, and instrumentation level
- observed-only native tools clearly labeled

`ToolCatalogReadModel`:

- registered tools, schemas, required scopes, exposure, status, and risk

`EvaluationRecord`:

- tool counts by origin
- failed/denied/canceled/observed-only calls
- redaction or delivery failures that affect confidence

## Prototype Scope

In scope:

- Tool registry with the first six Capo tools.
- Fake tool that exercises request, permission, invocation, output, completion, and delivery.
- ACP permission and reported tool-call mapping.
- Read-only status/workpad tools.
- Evidence recording as a state mutation.
- Observed-only records for native Codex/Claude tools.

Deferred:

- Publishing Capo tools as MCP.
- Full shell/file/git write wrappers.
- Remote runtime tool execution.
- Provider-native enforcement.
- Browser tools.
- Secret-handling tools.
- Voice approval for privileged tool grants.

## Test Strategy

Prototype tests should prove:

1. `FakeToolExposure` emits request, permission, started, output, completed, and result-delivered events.
2. A denied tool call records permission denial and does not invoke the handler.
3. `capo.task_status`, `capo.agent_status`, and `capo.session_summary` read from projections without mutating state.
4. `capo.workpad_read` preserves markdown authority and records artifacts for large reads.
5. `capo.evidence_record` appends evidence records without editing markdown task status.
6. ACP `session/request_permission` maps to A3 policy and chosen option IDs.
7. ACP `fs/*` and `terminal/*` requests are refused unless matching wrappers and scopes are enabled.
8. Adapter-native tool updates with stable external IDs dedupe during replay.
9. Low-confidence text-only native tool observations appear as `observed_only`.

## Recommendation

Build the first prototype around Capo-owned tools and fake tool calls before adding shell/file/git wrappers. This gives the controller, state store, permission policy, read models, and adapter result-delivery path an e2e test without depending on Codex or Claude native tool visibility.

Confidence: high for the boundary and event shape. Confidence is medium for exact native-tool observability in Codex and Claude until fixture captures prove which tool fields are available.
