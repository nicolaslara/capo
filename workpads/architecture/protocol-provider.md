# Capo Protocol And Provider Plan

## Objective

Define Capo's prototype agent/protocol/provider architecture: first concrete adapters, ACP client shape, provider connector records, subscription-backed connector policy, state additions, risks, and prototype/deferred choices.

This is the A5 architecture artifact. It keeps Capo as the user entrypoint and controller while treating Codex, Claude Code, and ACP-compatible agents as swappable adapter/provider integrations.

## Design Rules

- Capo owns tasks, sessions, runs, turns, permissions, state, memory, evaluation, dashboard, and voice control.
- Agent adapters normalize external agent surfaces into Capo events and commands. External sessions are references, not Capo truth.
- Provider connectors expose non-secret provider/auth/usage metadata and revocation instructions. They do not execute turns directly in v0.
- Runtime runners supervise processes. Adapters do not own process groups, tunnel identity, output retention, or restart recovery.
- Subscription-backed CLIs are privileged local connectors. Capo launches them through `RuntimeRunner` and never reads OAuth tokens, cookies, keychain entries, or vendor credential files.
- Codex and Claude Code are first-class prototype adapters.
- ACP is an interoperability adapter boundary, not Capo's internal controller API.
- Capo does not expose itself as an ACP agent/editor backend in the prototype.
- Adapter-native modes/config/options are recorded as metadata; Capo does not add product-level modes.

## Static Dispatch Shape

Prototype adapter enum:

```text
enum AgentAdapter {
  CodexExec(CodexExecAdapter),
  ClaudeCode(ClaudeCodeAdapter),
  Acp(AcpAdapter),
  Fake(FakeAdapter),
}
```

Prototype provider enum:

```text
enum ProviderConnector {
  CodexSubscription(CodexSubscriptionConnector),
  ClaudeSubscription(ClaudeSubscriptionConnector),
  OpenAiApi(OpenAiApiConnector),
  AnthropicApi(AnthropicApiConnector),
  LocalModel(LocalModelConnector),
  Unknown(UnknownProviderConnector),
  Fake(FakeProviderConnector),
}
```

Prototype implementation order:

1. `FakeAdapter` plus `FakeProviderConnector` for e2e controller tests.
2. `CodexExecAdapter` because `codex exec --json` provides a typed JSONL stream.
3. `ClaudeCodeAdapter` because Claude Code Max/Pro is a first target and `claude -p --output-format stream-json` exposes streamable output.
4. `AcpAdapter` once the controller event pipeline is proven with fake/Codex fixtures, unless a ready ACP-compatible agent becomes the fastest path for a smoke test.

## Core Records

### AdapterConfig

Reusable concrete agent/protocol integration template.

Fields:

- `adapter_config_id`
- `project_id?`
- `name`
- `adapter_kind`: `codex_exec`, `claude_code`, `acp`, `fake`
- `command_template`
- `default_args`
- `stdin_mode`: `prompt_once`, `stream_json`, `interactive`
- `stdout_format`: `jsonl`, `stream_json`, `json`, `text`
- `stderr_policy`: `events`, `logs`, `ignored`
- `adapter_capabilities_json`
- `version_observed?`
- `status`: `available`, `disabled`, `unhealthy`
- `created_at`
- `updated_at`

`command_template` and `default_args` are metadata and launch hints. The runtime receives a concrete `RuntimeRequest` with `program`, `argv`, cwd, env policy, and launch mode.

`AdapterConfig` is a reusable integration template. The concrete `Agent` owns `runtime_target_id`, `provider_connector_id`, and `capability_profile_id`. If an adapter template proposes defaults, controller validation must copy them into the agent at registration time or reject drift; adapter defaults do not override the agent binding at launch.

### Agent Binding

The `Agent` record binds reusable integration templates to concrete execution policy:

- `agent_id`
- `adapter_config_id`
- `runtime_target_id`
- `provider_connector_id`
- `capability_profile_id`

The controller resolves this binding before launch. `AdapterConfig` cannot override it. If the selected adapter config, provider connector, runtime target, and capability profile are incompatible, the controller rejects the launch before `runtime.start_requested`.

### AdapterSessionRef

External session identity and adapter lifecycle metadata.

Fields:

- `adapter_session_ref_id`
- `session_id`
- `adapter_config_id`
- `external_session_ref`
- `external_turn_ref?`
- `protocol_version?`
- `adapter_state`: `starting`, `active`, `streaming`, `waiting_for_permission`, `completed`, `failed`, `detached`, `closed`
- `raw_event_cursor?`
- `attach_supported`
- `resume_supported`
- `load_supported`
- `created_at`
- `updated_at`

### ProviderConnectorConfig

Non-secret provider/auth/usage metadata.

Fields:

- `provider_connector_id`
- `project_id?`
- `provider_kind`: `codex_subscription`, `claude_subscription`, `openai_api`, `anthropic_api`, `local_model`, `unknown`, `fake`
- `credential_scope`: `user_local_subscription`, `api_key`, `wif`, `enterprise_access_token`, `local_model`, `none`
- `productization_allowed`: `local_only`, `hosted_allowed`, `enterprise_only`, `experimental_only`
- `auth_ref_kind`: `none`, `vendor_cli_default_login`, `secret_handle`, `wif_identity`, `enterprise_token_handle`, `local_endpoint`
- `auth_ref?`
- `account_label?`
- `workspace_label?`
- `usage_capability`: `unknown`, `usage_events`, `cost_events`, `credit_events`
- `revocation_instructions`
- `redaction_policy`
- `status`: `available`, `needs_login`, `expired`, `disabled`, `unhealthy`
- `created_at`
- `updated_at`

`auth_ref_kind` is the authority type; `auth_ref` is present only when the type requires a non-secret handle or endpoint label. For `credential_scope = user_local_subscription`, `auth_ref_kind` must be `vendor_cli_default_login` and `auth_ref` must be empty. It must not include filesystem paths, keychain identifiers, credential filenames, tokens, cookies, or other credential material. `secret_handle`, `wif_identity`, and `enterprise_token_handle` are reserved for API/WIF/enterprise connectors.

### AdapterCapabilitySnapshot

Observed adapter/provider capabilities for routing and dashboard display.

Fields:

- `adapter_capability_snapshot_id`
- `adapter_config_id`
- `provider_connector_id`
- `observed_at`
- `adapter_version?`
- `provider_version?`
- `auth_mode_observed?`
- `supports_streaming`
- `supports_interrupt`
- `supports_resume`
- `supports_permission_prompts`
- `supports_structured_tool_results`
- `supports_usage_metadata`
- `native_tools_json`
- `limitations_json`

## Adapter Contract

The implementation-facing adapter contract from `boundaries.md` remains:

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

Additional A5 rules:

- `initialize` records version/capability/auth metadata, never secrets.
- `build_runtime_request` converts an adapter/session request into a concrete runtime launch request. It does not spawn directly.
- `attach_started_process` observes the runtime process that the controller already started and extracts/creates the external adapter session reference.
- `stream_events` emits raw adapter records plus normalized `AdapterEvent` candidates.
- `cancel` maps to the best adapter control: Codex process interrupt/cancel, Claude process interrupt/cancel, ACP `session/cancel`, or runtime termination.
- `deliver_tool_result` can return `unsupported`; Capo then marks the tool result observed-only.
- Adapter event parsers must preserve raw event artifacts for replay/debug and emit normalized events with stable idempotency keys where possible.

Controller orchestration sequence:

1. Controller resolves `Agent`, `AdapterConfig`, `ProviderConnectorConfig`, `RuntimeTarget`, and `CapabilityProfile`.
2. Controller calls `authorize_connector_use(...)` and appends a denial event if the connector cannot be used in the current deployment/runtime context.
3. Controller asks the adapter to `build_runtime_request(...)`.
4. Controller asks `RuntimeRunner` to start the process following A4 start-ordering rules.
5. Controller calls `attach_started_process(...)` to create or observe `AdapterSessionRef`.
6. Controller sends turns and consumes `stream_events(...)`.

If adapter/provider/runtime bindings disagree, the controller rejects the launch before `runtime.start_requested`.

## CodexExecAdapter

Prototype path:

```text
program: codex
argv: exec --json --cd <workspace> --sandbox <mode> [prompt or stdin]
stdout_format: jsonl
stderr_policy: logs
```

Local observation on 2026-05-25:

- `codex` path: `/Users/nicolas/.nvm/versions/node/v24.10.0/bin/codex`
- `codex --version`: `codex-cli 0.133.0`
- `codex exec --help` exposes `--json`, `--sandbox read-only|workspace-write|danger-full-access`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--output-schema`, `--profile-v2`, and `--cd`.

Absolute paths above are diagnostic observations only. The scaffold should resolve `codex` from configured program name/PATH unless the user explicitly pins a path.

Mapping:

- Codex JSONL events become raw adapter events plus normalized Capo items/turns/tool calls/usage records.
- `thread.started` and Codex session IDs map to `AdapterSessionRef.external_session_ref`.
- Codex item IDs map to adapter timeline keys, not Capo item IDs.
- Codex usage data, if present, maps to evaluation/usage artifacts and provider usage snapshots.
- Codex sandbox mode is adapter/runtime metadata and must also be reflected in Capo capability decisions.
- Capo should default Codex runs to read-only for analysis/review and workspace-write for implementation tasks only when the capability profile allows it.

Auth/provider connector:

- `CodexSubscriptionConnector` for ChatGPT-plan sign-in through the official Codex CLI.
- `OpenAiApiConnector` for API-key/CI/hosted use.
- `CodexSubscriptionConnector` records auth mode/category only; it does not inspect ChatGPT tokens or browser sessions.
- Enterprise access tokens are `enterprise_access_token` scope, not consumer subscription scope.

Prototype limitations:

- Do not link to Codex internals.
- Do not depend on Codex's persisted session store as Capo truth.
- Treat `--dangerously-bypass-approvals-and-sandbox` as unavailable unless a profile explicitly opts into an externally sandboxed runtime.
- Real Codex tool-call observability depends on JSONL event detail; gaps are recorded as adapter limitations.

## ClaudeCodeAdapter

Prototype path:

```text
program: claude
argv: -p --output-format stream-json --verbose [--include-partial-messages if needed] [prompt or stdin]
stdout_format: stream_json
stderr_policy: logs
```

Local observation on 2026-05-25:

- `claude` path: `/Applications/cmux.app/Contents/Resources/bin/claude`
- `claude --version`: `2.1.150 (Claude Code)`
- `claude -p --help` exposes `--output-format text|json|stream-json`, `--input-format text|stream-json`, `--permission-mode`, `--allowedTools`, `--disallowedTools`, `--tools`, `--mcp-config`, `--strict-mcp-config`, `--session-id`, `--resume`, `--continue`, `--no-session-persistence`, `--bare`, `--max-budget-usd`, and `--include-partial-messages`.

Absolute paths above are diagnostic observations only. The scaffold should resolve `claude` from configured program name/PATH unless the user explicitly pins a path.

Mapping:

- `stream-json` records become raw adapter events plus normalized Capo items/turns/tool calls/usage records.
- Claude `session-id` maps to `AdapterSessionRef.external_session_ref` when provided or observed.
- Claude tool/permission events map to Capo tool calls and permission requests when visible in stream JSON.
- Claude cost/usage fields, if present, map to provider usage snapshots and evaluation artifacts.
- Claude permission mode and allowed/disallowed tools are adapter metadata plus Capo capability decisions.

Auth/provider connector:

- `ClaudeSubscriptionConnector` for user-owned local Claude Code login through vendor-supported CLI auth.
- `AnthropicApiConnector` for API-key, WIF, CI, hosted, or organization-managed automation.
- Capo must avoid causing accidental auth precedence changes. Its env policy should scrub unrelated `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, and similar variables unless the selected connector explicitly needs them.
- `--bare` is useful for deterministic API-key/scripted mode, but it skips OAuth/keychain reads in the observed CLI help. Do not use `--bare` for subscription OAuth sessions unless current Claude docs and local smoke tests prove it is appropriate.

Prototype limitations:

- Interactive Claude Code sessions may need PTY support; v0 starts with noninteractive `-p` stream JSON.
- If stream JSON lacks stable IDs for some content, Capo uses content hashes and ordinal anchors with low/medium confidence, as in ACP message replay.
- Native tool result delivery back into Claude may be unsupported in v0; record observed-only tool results when needed.

## AcpAdapter

Prototype shape:

- Transport: stdio JSON-RPC 2.0.
- Capo is the ACP client.
- Adapter builds a runtime request for an ACP-compatible agent process, then attaches after `RuntimeRunner` starts it.
- Adapter calls `initialize`, records negotiated protocol version/capabilities/auth methods, then creates/resumes/loads sessions as supported.

Capo calls agent:

- `initialize`
- `authenticate` only when required and user-approved
- `session/new`
- `session/prompt`
- `session/cancel`
- `session/load` and `session/resume` only when advertised and when A2a dedupe rules are implemented

Capo implements client handlers:

- `session/request_permission`
- `fs/read_text_file` and `fs/write_text_file` only when `tool-exposure.md` wrappers and capability policy enable them
- `terminal/*` only through `RuntimeRunner`-backed wrappers when capability policy enables them
- MCP/client tool surfaces only when `tool-exposure.md` defines and enables them

Prototype default: advertise no ACP `fs` or `terminal` client capability until the corresponding Capo wrapper tools have `ToolDefinition` rows, permission scopes, and passing wrapper tests.

Mapping:

- ACP session IDs are external refs.
- ACP `session/update` messages are adapter inputs and never UI truth directly.
- ACP `ToolCallUpdate.toolCallId` is a stable timeline key.
- ACP permission options map through `capability-permissions.md`.
- ACP client capabilities such as filesystem, terminal, and MCP server config are exposed only when Capo capability policy permits them.
- ACP tool calls and client capability requests map through `tool-exposure.md`; raw ACP tool updates are observations until Capo executes the backing wrapper.

Deferred:

- Capo-as-ACP-agent/editor backend.
- ACP registry automation.
- HTTP transport.
- Custom extension methods beyond `_meta` capture.
- Treating ACP modes/config options as Capo product modes.

## Subscription Connector Policy

Allowed for local user-owned Capo:

- Codex CLI with ChatGPT-plan sign-in.
- Claude Code CLI with Claude Pro/Max or organization-supported login.
- Vendor API-key/WIF/enterprise token connectors when the user configures them through a secret manager or environment injection.

Not allowed as normal connectors:

- ChatGPT or Claude web UI scraping.
- Browser cookie capture/replay.
- Reverse-engineered private endpoints.
- Routing another user's consumer subscription through a hosted Capo broker.
- Reading vendor credential files or keychain entries.

Productization rule:

- `credential_scope = user_local_subscription` implies `productization_allowed = local_only`.
- Hosted/shared Capo must use API, WIF, enterprise access-token, or vendor-approved organization flows.
- Browser experiments, if ever added, are `experimental_only` and require separate security review.

Connector-use gate:

```text
authorize_connector_use(ProviderConnectorConfig, DeploymentContext, Actor, RuntimeTarget) -> ConnectorUseDecision
```

The controller must call this before creating a `RuntimeRequest`. It appends `provider.connector_use_denied` and stops before runtime launch when a local-only subscription connector is used in hosted/shared mode, when auth scope and deployment policy conflict, or when a connector would require reading vendor credential material.

## State Model Additions

Add tables:

```text
adapter_configs(adapter_config_id, project_id, name, adapter_kind, command_template_json, default_args_json, stdin_mode, stdout_format, stderr_policy, adapter_capabilities_json, version_observed, status, created_at, updated_at)
adapter_session_refs(adapter_session_ref_id, session_id, adapter_config_id, external_session_ref_json, external_turn_ref_json, protocol_version, adapter_state, raw_event_cursor, attach_supported, resume_supported, load_supported, created_at, updated_at)
provider_connectors(provider_connector_id, project_id, provider_kind, credential_scope, productization_allowed, auth_ref_kind, auth_ref, account_label, workspace_label, usage_capability, revocation_instructions, redaction_policy_json, status, created_at, updated_at)
adapter_capability_snapshots(adapter_capability_snapshot_id, adapter_config_id, provider_connector_id, observed_at, adapter_version, provider_version, auth_mode_observed, supports_streaming, supports_interrupt, supports_resume, supports_permission_prompts, supports_structured_tool_results, supports_usage_metadata, native_tools_json, limitations_json)
```

Add events:

- `adapter.config_registered`
- `adapter.config_updated`
- `adapter.capabilities_observed`
- `adapter.session_started`
- `adapter.session_attached`
- `adapter.session_detached`
- `adapter.session_closed`
- `adapter.raw_event_observed`
- `adapter.event_normalized`
- `adapter.auth_required`
- `adapter.auth_failed`
- `provider.connector_registered`
- `provider.connector_updated`
- `provider.auth_status_observed`
- `provider.usage_observed`
- `provider.revocation_requested`
- `provider.connector_use_denied`
- `provider.connector_disabled`

## Test Strategy

Prototype tests should prove:

1. `FakeAdapter` can start a session, stream events, request a tool, receive a result, and complete a turn through fake runtime/provider.
2. `CodexExecAdapter` fixture parsing maps JSONL events into Capo turns/items/tool calls/usage without treating Codex IDs as Capo IDs.
3. `ClaudeCodeAdapter` fixture parsing maps stream JSON into Capo turns/items/tool calls/usage and records low-confidence IDs where needed.
4. `AcpAdapter` fixture parsing maps initialize/session/prompt/update/permission/cancel flows according to A2a/A3.
5. Provider connectors record auth mode/category and revocation instructions without storing raw secrets.
6. Subscription CLI env policy scrubs conflicting provider variables unless explicitly selected by the connector.
7. `authorize_connector_use(...)` rejects `user_local_subscription` connectors in hosted/shared mode before any runtime request is created.
8. Adapter version/capability snapshots appear in session/agent read models for introspection.

## Recommendation

Implement the prototype with a fake adapter first, then Codex, then Claude Code. Keep ACP support in the scaffold and fixtures early, but do not let ACP drive Capo's controller API or Capo-as-agent direction.

Confidence: high for adapter/provider boundary shape and subscription policy. Confidence is medium for exact Claude stream JSON and Codex JSONL field mapping until real fixture captures are added from the installed CLIs.
