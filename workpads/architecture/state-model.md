# Capo State Model And Event Log

## Objective

Define the prototype state model Capo will implement first: durable entities, event types, read models, restart recovery behavior, and SQLite/filesystem layout.

This is the A2 architecture artifact. It refines the `StateStore` boundary from `boundaries.md` into an implementation-facing schema plan without locking exact SQL migrations yet.

## Design Rules

- SQLite is the prototype source of truth for operational state.
- Markdown workpads remain the human-readable planning source; SQLite references them but does not silently rewrite them in v0.
- Capo events are append-only. Corrections are new events.
- Capo-owned IDs are authoritative. External adapter/runtime/provider IDs are stored as references.
- Raw adapter/runtime/provider records are inputs and artifacts, not the projected product state.
- Read models are rebuildable from events plus artifacts.
- Projection code must tolerate duplicate raw inputs by using idempotency keys and external references.
- ACP-specific replay and partial-streaming details live in `acp-replay-dedupe.md`.

## Storage Authority

| Fact type | Authority | Notes |
| --- | --- | --- |
| Project registration | SQLite | Points at repo/workpad paths. |
| Workpad task text/status | Markdown files | Capo stores path, heading anchor, and observed workpad status snapshots. |
| Agent configuration | SQLite | Includes adapter/runtime/provider/capability profile IDs. |
| Adapter configuration and external session refs | SQLite events and read models | Detailed protocol/provider model lives in `protocol-provider.md`. |
| Provider connector metadata | SQLite events and read models | Stores non-secret auth/provider metadata and revocation instructions, not raw credentials. |
| Runtime targets and process refs | SQLite events and read models | Detailed runtime/tunnel model lives in `runtime-tunnel.md`. |
| Connectivity endpoints | SQLite events and read models | Stores endpoint/tunnel metadata and redacted auth references, not raw secrets. |
| Session/run/turn lifecycle | SQLite events | Read models are projections. |
| Streaming messages/items/tool calls | SQLite events | Raw chunks may also be artifact records. |
| Runtime stdout/stderr/PTY logs | File artifacts plus event pointers | Avoid unbounded blobs in event rows. |
| Raw adapter events | File artifacts or raw-event table | Used for replay/debug/dedupe, not UI truth. |
| Adapter replay batches | SQLite plus raw-update artifacts | Used to reconcile ACP `session/load` and restart recovery without duplicate UI state. |
| Adapter replay candidates | SQLite staging table | Non-projecting records used during replay reconciliation before accepted Capo events are appended. |
| Capability profiles, grants, and permission decisions | SQLite events and read models | Detailed scope and policy model lives in `capability-permissions.md`. |
| Tool definitions, invocations, and observations | SQLite events and read models | Detailed wrapper/instrumentation model lives in `tool-exposure.md`. |
| Human decisions and review/evidence | SQLite events plus artifacts | Links back to workpad evidence where applicable. |
| Derived memory | SQLite events/read models plus rebuildable memory indexes | Must reference event/file provenance. Detailed model lives in `memory-architecture.md`. |
| Recovery attempts | SQLite events plus recovery metadata | Used only to make restart reconciliation idempotent. |

## Core Entities

### Project

Represents a repository/workspace under Capo control.

Fields:

- `project_id`
- `name`
- `workspace_root`
- `workpad_root?`
- `default_branch?`
- `created_at`
- `status`

Prototype status values: `active`, `archived`.

### Task

Represents a user-visible work item, often mirrored from a workpad heading.

Fields:

- `task_id`
- `project_id`
- `source_kind`: `workpad`, `manual`, `external`
- `source_ref?`: markdown path plus heading or external tracker reference
- `title`
- `capo_execution_status`
- `workpad_status_observed?`
- `active_session_id?`
- `created_at`
- `updated_at`

Prototype `capo_execution_status` values: `pending`, `active`, `blocked`, `reviewing`, `completed`, `canceled`.

Prototype `workpad_status_observed` values mirror the markdown source text when the source is a workpad. Capo does not treat an observed workpad status as permission to rewrite the markdown file.

### Agent

Represents a configured worker identity Capo can start and inspect.

Fields:

- `agent_id`
- `project_id?`
- `name`
- `adapter_config_id`
- `runtime_target_id`
- `provider_connector_id`
- `capability_profile_id`
- `metadata`
- `status`

Prototype status values: `available`, `running`, `paused`, `unhealthy`, `disabled`.

### AdapterConfig

Configured concrete agent/protocol integration.

Fields:

- `adapter_config_id`
- `project_id?`
- `name`
- `adapter_kind`: `codex_exec`, `claude_code`, `acp`, `fake`
- `command_template`
- `default_args`
- `stdin_mode`
- `stdout_format`
- `stderr_policy`
- `adapter_capabilities`
- `version_observed?`
- `status`
- `created_at`
- `updated_at`

`AdapterConfig` is a reusable integration template. `Agent` owns runtime, provider connector, and capability binding for launch decisions.

### ProviderConnector

Non-secret provider/auth/usage metadata behind an adapter.

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
- `usage_capability`
- `revocation_instructions`
- `redaction_policy`
- `status`
- `created_at`
- `updated_at`

Raw API keys, OAuth tokens, browser cookies, and vendor credential files are never stored in this table. User-local subscription connectors must use `auth_ref_kind = vendor_cli_default_login` with an empty `auth_ref`, not a filesystem or keychain reference.

### AdapterSessionRef

External session identity and adapter lifecycle metadata.

Fields:

- `adapter_session_ref_id`
- `session_id`
- `adapter_config_id`
- `external_session_ref`
- `external_turn_ref?`
- `protocol_version?`
- `adapter_state`
- `raw_event_cursor?`
- `attach_supported`
- `resume_supported`
- `load_supported`
- `created_at`
- `updated_at`

### Session

Durable conversation/execution context between Capo and one agent.

Fields:

- `session_id`
- `project_id`
- `task_id?`
- `agent_id`
- `title`
- `status`
- `current_goal`
- `current_goal_artifact_id?`
- `latest_confidence?`
- `external_session_ref?`
- `created_at`
- `updated_at`
- `last_sequence`

Prototype status values: `starting`, `active`, `waiting_for_input`, `waiting_for_permission`, `canceling`, `completed`, `failed`, `canceled`, `recovering`.

### Run

One execution attempt inside a session. A session can have multiple runs after restart, retry, provider switch, or adapter relaunch.

Fields:

- `run_id`
- `session_id`
- `runtime_process_ref?`
- `adapter_instance_ref?`
- `started_at`
- `ended_at?`
- `exit_status?`
- `status`
- `recovery_of_run_id?`

Prototype status values: `starting`, `running`, `stopping`, `exited`, `failed`, `orphaned`, `recovered`.

Restart recovery can recover the same run in place when the original process is still alive and attachable. `recovery_of_run_id` is for a new run that relaunches/retries after restart, provider switch, or explicit recovery attempt.

### RuntimeTarget

Configured execution placement for an agent process.

Fields:

- `runtime_target_id`
- `project_id?`
- `name`
- `runner_kind`: `local_process`, `remote_process`, `container`
- `workspace_root`
- `artifact_root`
- `default_cwd`
- `env_policy_json`
- `capability_profile_id`
- `connectivity_endpoint_id?`
- `status`
- `created_at`
- `updated_at`

`env_policy_json` stores environment inheritance, allowlist, redaction, and secret-handle references. It must not store raw secret values.

### RuntimeProcessRef

Opaque process reference returned by a runtime runner.

Fields:

- `runtime_process_ref_id`
- `runtime_target_id`
- `run_id`
- `external_pid?`
- `process_group_ref?`
- `remote_process_ref?`
- `started_at`
- `last_heartbeat_at?`
- `status`
- `redaction_state`

### ConnectivityEndpoint

Configured way to reach a runtime or Capo surface.

Fields:

- `connectivity_endpoint_id`
- `project_id?`
- `name`
- `tunnel_kind`: `local_loopback`, `ssh`, `tailscale`, `reverse`
- `address_ref`
- `identity_ref?`
- `auth_ref?`
- `exposure`: `loopback`, `private`, `public`
- `allowed_channels`
- `status`
- `created_at`
- `updated_at`

### ResolvedEndpoint

Resolved endpoint/channel for one runtime, Capo server, input surface, or artifact operation.

Fields:

- `resolved_endpoint_id`
- `connectivity_endpoint_id`
- `owner_kind`: `runtime_target`, `capo_server`, `input_surface`, `artifact_store`
- `owner_id`
- `channel_kind`: `control`, `stdio`, `logs`, `dashboard`, `artifact`
- `resolved_uri`
- `identity_fingerprint?`
- `expires_at?`
- `redaction_state`
- `created_at`

`owner_kind` / `owner_id` is the typed endpoint owner. Runtime targets, Capo server/API surfaces, input surfaces, and artifact stores can all own resolved endpoints.

### Turn

One controller/user instruction and the resulting agent activity.

Fields:

- `turn_id`
- `session_id`
- `run_id?`
- `origin_command_id?`
- `role`: `user`, `capo`, `agent`, `system`
- `status`
- `created_at`
- `completed_at?`

Prototype status values: `open`, `streaming`, `waiting_for_tool`, `waiting_for_permission`, `completed`, `failed`, `canceled`.

### Item

Streamable content inside a turn.

Fields:

- `item_id`
- `turn_id`
- `kind`
- `status`
- `stream_state?`
- `ordinal`
- `summary?`
- `artifact_id?`
- `external_item_ref?`
- `content_hash?`
- `chunk_count?`
- `message_boundary_confidence?`
- `adapter_timeline_key_id?`
- `import_confidence?`

Prototype kinds:

- `message`
- `reasoning`
- `plan`
- `tool_call`
- `tool_result`
- `command_output`
- `file_change`
- `diff`
- `checkpoint`
- `summary`
- `error`

Prototype status values: `started`, `streaming`, `completed`, `failed`, `redacted`, `superseded`.

### ToolCall

Tracks Capo-exposed, adapter-native, or runtime tool use.

Fields:

- `tool_call_id`
- `session_id`
- `turn_id?`
- `item_id?`
- `tool_name`
- `tool_origin`: `capo`, `adapter_native`, `runtime`, `provider_native`
- `permission_decision_id?`
- `status`
- `started_at?`
- `completed_at?`
- `latency_ms?`
- `input_artifact_id?`
- `output_artifact_id?`
- `external_tool_ref?`

Prototype status values: `requested`, `approved`, `started`, `output`, `completed`, `failed`, `denied`, `canceled`, `observed_only`.

### ToolDefinition

Registered callable surface exposed through Capo, runtime wrappers, adapter observers, provider observers, MCP bridge, or fake tests.

Fields:

- `tool_definition_id`
- `tool_id`
- `display_name`
- `origin`: `capo`, `runtime`, `adapter_native`, `provider_native`, `mcp`
- `handler_kind`
- `schema`
- `required_scopes`
- `risk`
- `redaction_policy`
- `exposure`
- `instrumentation_level`: `full`, `structured_observed`, `text_observed`, `none`
- `status`
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

### ToolObservation

Structured or partial observation of a tool event from an adapter, provider, runtime, MCP bridge, or manual review.

Fields:

- `tool_observation_id`
- `tool_call_id?`
- `tool_invocation_id?`
- `source`: `adapter_event`, `runtime_output`, `provider_usage`, `mcp_message`, `manual`
- `external_tool_ref?`
- `observed_status?`
- `confidence`
- `raw_event_id?`
- `artifact_id?`
- `observed_at`

### PermissionDecision

Durable record of a policy decision, even when the prototype allows everything.

Fields:

- `permission_decision_id`
- `request_id`
- `session_id?`
- `run_id?`
- `tool_call_id?`
- `capability_profile_id`
- `decision`: `allow`, `reject`, `cancel`
- `persistence`: `once`, `until_turn_end`, `until_session_end`, `until_revoked`, `until_time`
- `source`: `allow_trusted_local_profile`, `static_policy`, `user`, `security_agent`
- `scope`
- `expires_at?`
- `revoked_at?`

A3 owns the full scope vocabulary and ACP option mapping.

### CapabilityProfile

Named default authority envelope for an agent/session/run.

Fields:

- `capability_profile_id`
- `project_id?`
- `name`
- `description`
- `default_scopes`
- `risk_level`
- `decision_mode`
- `created_at`
- `updated_at`
- `disabled_at?`

### CapabilityGrant

Durable scoped grant or deny rule created by policy.

Fields:

- `capability_grant_id`
- `capability_profile_id`
- `scope`
- `effect`: `allow`, `deny`
- `subject`
- `decision_id`
- `source`
- `persistence`
- `expires_at?`
- `revoked_at?`
- `revocation_reason?`
- `created_at`

### CapabilityGrantUse

Audit record that a grant was consumed by a tool/runtime/adapter/input action.

Fields:

- `capability_grant_use_id`
- `capability_grant_id`
- `permission_request_id?`
- `session_id?`
- `run_id?`
- `tool_call_id?`
- `used_at`
- `result`

### PermissionRequest

Durable pending approval or policy-decision input.

Fields:

- `permission_request_id`
- `session_id?`
- `run_id?`
- `tool_call_id?`
- `capability_profile_id`
- `scope`
- `risk`
- `source`: `capo`, `adapter`, `runtime`, `tool`
- `adapter_options?`: provider/protocol-native options such as ACP approval choices
- `status`: `pending`, `decided`, `stale`, `canceled`
- `created_at`
- `decided_at?`

### Artifact

Pointer to content that should not live directly in event payloads.

Fields:

- `artifact_id`
- `project_id`
- `session_id?`
- `run_id?`
- `kind`
- `uri`
- `content_hash`
- `size_bytes`
- `redaction_state`
- `created_at`

Prototype kinds: `raw_adapter_event`, `runtime_log`, `prompt`, `tool_input`, `tool_output`, `diff`, `review`, `evidence`, `checkpoint`, `summary`.

### Evidence

Human or automated proof attached to a task, session, run, or evaluation.

Fields:

- `evidence_id`
- `project_id`
- `task_id?`
- `session_id?`
- `run_id?`
- `kind`: `test`, `smoke`, `review`, `manual_note`, `artifact`, `external_link`
- `artifact_id?`
- `source_ref?`
- `confidence`
- `created_at`

### EvaluationRecord

Assessment of whether work met criteria and how an agent performed.

Fields:

- `evaluation_id`
- `project_id`
- `task_id?`
- `session_id?`
- `run_id?`
- `status`
- `criteria`
- `result`
- `reviewer`
- `evidence_id?`
- `created_at`

### RecoveryAttempt

One startup reconciliation pass.

Fields:

- `recovery_attempt_id`
- `started_at`
- `completed_at?`
- `status`
- `emitted_sequence_start?`
- `emitted_sequence_end?`
- `notes`

### MemoryRecord

Derived reusable memory item with provenance, confidence, review state, and validity metadata.

Fields:

- `memory_record_id`
- `project_id`
- `scope`
- `scope_owner_ref`
- `subject_ref?`
- `sensitivity_classification`
- `record_kind`
- `subject`
- `predicate`
- `object`
- `body`
- `confidence`
- `review_state`
- `source_count`
- `valid_from?`
- `valid_until?`
- `supersedes_memory_record_id?`
- `revoked_by_memory_record_id?`
- `redaction_state`
- `created_at`
- `updated_at`

Permission checks and packet filtering use `scope_owner_ref`, `subject_ref`, and `sensitivity_classification`; free-text subject/predicate/object fields are not authorization inputs.

### MemorySource

Provenance edge from a memory record to a source event, artifact, markdown section, or external import.

Fields:

- `memory_source_id`
- `memory_record_id`
- `source_kind`: `event`, `artifact`, `markdown`, `external_import`
- `source_event_id?`
- `source_artifact_id?`
- `source_path?`
- `source_anchor?`
- `source_content_hash?`
- `source_sequence?`
- `quote_artifact_id?`
- `observed_at`

### MemoryIndexEntry

Rebuildable index metadata for FTS, semantic, or graph indexes.

Fields:

- `memory_index_entry_id`
- `memory_record_id`
- `index_kind`
- `index_version`
- `indexed_text_hash`
- `backend_ref?`
- `status`
- `indexed_at`

### MemoryPacket

Task-specific context packet assembled for an agent/session/turn.

Fields:

- `memory_packet_id`
- `project_id`
- `task_id?`
- `agent_id?`
- `session_id?`
- `run_id?`
- `turn_id?`
- `purpose`
- `budget_tokens`
- `selection_policy`
- `included_items`
- `excluded_items`
- `explanation_artifact_id?`
- `packet_artifact_id`
- `created_at`

`MemoryPacket` rows are created only after `packet_artifact_id` exists. Draft packet planning belongs to `MemoryJob`. The packet artifact is prompt-input evidence; source events and workpads remain factual authority.

### MemoryJob

Async or inline memory operation job.

Fields:

- `memory_job_id`
- `project_id`
- `source_query`
- `job_kind`: `extract_facts`, `index_fts`, `build_packet`, `invalidate`, `export`, `rebuild`
- `status`
- `started_at?`
- `completed_at?`
- `emitted_sequence_start?`
- `emitted_sequence_end?`
- `error?`

## Capo Event Envelope

Every event row uses the cross-cutting envelope from `boundaries.md`:

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

Additional storage fields:

- `schema_version`
- `causation_id?`: command/event that caused this event
- `correlation_id?`: command/session/recovery flow this event belongs to
- `payload_hash`
- `recorded_at`

Uniqueness rules:

- `sequence` is globally monotonic per Capo database.
- `event_id` is globally unique.
- `(project_id, idempotency_key)` is unique when `idempotency_key` is present.
- `(adapter_config_id, external_ref)` can be unique in adapter-specific raw-event indexes when the adapter provides stable refs.
- `payload_hash` is not an identity by itself; it only supports diagnostics.

## Prototype Event Types

### Project And Task

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `project.registered` | Name, workspace root, workpad root | `projects` |
| `project.archived` | Reason | `projects` |
| `task.discovered` | Source kind/ref, title, initial execution status | `tasks` |
| `task.execution_status_changed` | Old/new Capo execution status, reason | `tasks` |
| `task.workpad_status_observed` | Source ref, observed markdown status text | `tasks` |
| `task.evidence_recorded` | Evidence artifact/source refs | `tasks`, `evidence` |

### Agent And Session

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `agent.registered` | Adapter/runtime/provider connector/capability IDs | `agents` |
| `agent.status_changed` | Old/new status, health note | `agents` |
| `adapter.config_registered` | Adapter kind, command template, IO formats | `adapter_configs` |
| `adapter.config_updated` | Adapter config/status change | `adapter_configs` |
| `adapter.capabilities_observed` | Version, capability snapshot, limitations | `adapter_capability_snapshots` |
| `adapter.session_started` | External session ref and adapter state | `adapter_session_refs`, `sessions` |
| `adapter.session_attached` | Attach/resume/load metadata | `adapter_session_refs`, `sessions` |
| `adapter.session_detached` | Detached external session and reason | `adapter_session_refs`, `sessions` |
| `adapter.session_closed` | External session close result | `adapter_session_refs`, `sessions` |
| `provider.connector_registered` | Provider kind, credential scope, productization policy | `provider_connectors` |
| `provider.connector_updated` | Provider connector status/config change | `provider_connectors` |
| `provider.auth_status_observed` | Non-secret auth/account status metadata | `provider_connectors` |
| `provider.usage_observed` | Usage/cost/credit metadata when available | `provider_connectors`, `evaluations` |
| `provider.revocation_requested` | Revocation action/instructions | `provider_connectors` |
| `provider.connector_use_denied` | Connector blocked by deployment/auth/runtime policy before launch | `provider_connectors` |
| `provider.connector_disabled` | Connector disabled and reason | `provider_connectors` |
| `runtime.target_registered` | Runtime target config and runner kind | `runtime_targets` |
| `runtime.target_updated` | Runtime target change and reason | `runtime_targets` |
| `connectivity.endpoint_registered` | Endpoint/tunnel config and exposure | `connectivity_endpoints` |
| `connectivity.endpoint_updated` | Endpoint config/status change | `connectivity_endpoints` |
| `connectivity.health_changed` | Reachability status and safe diagnostic refs | `connectivity_endpoints` |
| `connectivity.endpoint_resolved` | Resolved endpoint/channel with owner and redaction state | `resolved_endpoints` |
| `connectivity.channel_opened` | Opened channel metadata and endpoint ref | `resolved_endpoints` |
| `connectivity.channel_closed` | Closed channel metadata and reason | `resolved_endpoints` |
| `connectivity.exposure_changed` | Exposure scope transition and actor | `connectivity_endpoints` |
| `session.started` | Agent, task, title, external session ref | `sessions` |
| `session.status_changed` | Old/new status, reason | `sessions` |
| `session.summary_updated` | Summary artifact or short text | `sessions` |
| `session.completed` | Outcome, evidence refs | `sessions`, `tasks` |
| `session.failed` | Error, recovery recommendation | `sessions` |

### Run And Runtime

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `run.started` | Runtime target, command metadata, process ref | `runs`, `sessions` |
| `runtime.start_requested` | Runtime request metadata, idempotency key, pending status | `runs` |
| `runtime.prepared` | Runtime target prepared for a run | `runtime_targets`, `runs` |
| `runtime.process_started` | Runtime process ref, command metadata, redaction state | `runtime_process_refs`, `runs` |
| `runtime.process_start_failed` | Launch failure, cleanup result, retryability | `runs` |
| `runtime.output_delta` | Output stream/channel, byte range, artifact ref | `runtime_process_refs`, `runs`, `items` |
| `runtime.output_artifact_recorded` | Runtime output artifact metadata | `artifacts`, `runs` |
| `runtime.stdin_written` | Stdin write metadata and byte count | `runtime_process_refs`, `runs` |
| `run.output_observed` | Stream/channel, artifact ref, byte range | `runs`, `items` |
| `run.health_changed` | Health/status details | `runs`, `agents` |
| `run.interrupt_requested` | Actor, target, reason | `runs`, `sessions` |
| `run.interrupt_sent` | Runtime process ref, result metadata | `runs` |
| `runtime.interrupt_sent` | Low-level runtime interrupt result | `runtime_process_refs`, `runs` |
| `run.interrupt_failed` | Error, retryability | `runs`, `sessions` |
| `run.stop_requested` | Actor, target, reason, force flag | `runs`, `sessions` |
| `run.terminate_sent` | Runtime process ref, escalation level | `runs` |
| `runtime.terminate_sent` | Low-level runtime terminate result | `runtime_process_refs`, `runs` |
| `runtime.kill_sent` | Low-level runtime kill result | `runtime_process_refs`, `runs` |
| `run.terminate_failed` | Error, retryability | `runs`, `sessions` |
| `run.exited` | Exit status, signal, reason | `runs`, `sessions` |
| `runtime.process_exited` | Low-level process exit status and output close state | `runtime_process_refs`, `runs` |
| `runtime.health_changed` | Runtime liveness/heartbeat status | `runtime_process_refs`, `runs` |
| `runtime.cleanup_completed` | Runtime cleanup result | `runtime_process_refs`, `runs` |
| `run.orphaned` | Restart detected process without owner | `runs` |
| `run.recovered` | Existing run reattached after restart, or recovery metadata for a relaunched run | `runs`, `sessions` |

### Adapter Replay And Attach

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `adapter.replay_started` | Replay/import source, external session, batch ID | `adapter_replay_batches`, `sessions` |
| `adapter.raw_update_observed` | Raw update artifact/hash/index | `adapter_raw_updates` |
| `adapter.raw_event_observed` | Raw non-ACP adapter event artifact/hash/index | `raw_events` |
| `adapter.event_normalized` | Raw adapter event mapped to Capo event refs | `raw_events`, `items`, `tool_calls` |
| `adapter.auth_required` | Adapter requested user/provider authentication | `adapter_session_refs`, `provider_connectors` |
| `adapter.auth_failed` | Adapter auth failure without secret material | `adapter_session_refs`, `provider_connectors` |
| `adapter.replay_duplicate_detected` | Existing item/tool matched by timeline key or content hash | `adapter_replay_batches`, `items` |
| `adapter.replay_ambiguous` | Low-confidence replay match requiring caution/review | `adapter_replay_batches`, `items` |
| `adapter.replay_completed` | Imported/duplicate/ambiguous counts and event range | `adapter_replay_batches`, `sessions` |
| `adapter.attach_started` | External session and attach method such as ACP `session/resume` | `sessions` |
| `adapter.attach_completed` | Attach result metadata | `sessions` |
| `adapter.attach_failed` | Attach error and resumability status | `sessions` |

### Command, Turn, And Items

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `command.received` | Command envelope metadata | `commands` |
| `command.accepted` | Target IDs, normalized intent | `commands` |
| `command.rejected` | Reason and safe detail | `commands` |
| `command.completed` | Result status and produced refs | `commands` |
| `session.goal_updated` | Current goal text/artifact, source command, reason | `sessions`, `tasks` |
| `session.confidence_updated` | Confidence value, source item/evaluation, explanation artifact | `sessions`, `tasks` |
| `session.redirect_requested` | Steering command, previous goal ref, new goal ref | `sessions`, `commands` |
| `session.redirect_delivered` | Adapter/runtime delivery status for steering command | `sessions`, `turns` |
| `turn.started` | Role, prompt artifact, adapter turn ref | `turns` |
| `item.started` | Kind, ordinal, external item ref | `items` |
| `item.delta` | Append/update patch, content artifact ref | `items` |
| `item.completed` | Final content/ref/status | `items` |
| `item.failed` | Error payload | `items` |
| `turn.completed` | Stop reason, summary refs | `turns`, `sessions` |
| `turn.canceled` | Actor, reason | `turns`, `sessions` |

### Tools And Permissions

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `permission.requested` | Scope/risk/source request | `permission_requests`, `permission_queue` |
| `permission.decided` | Decision, persistence, source, expiry | `permission_decisions` |
| `permission.revoked` | Grant/decision ID, reason | `permission_decisions` |
| `permission.explanation_recorded` | Human-readable decision explanation | `permission_decisions` |
| `capability.profile_created` | Profile name, scopes, decision mode | `capability_profiles` |
| `capability.profile_updated` | Scope/profile changes, reason | `capability_profiles` |
| `capability.grant_created` | Scope, effect, subject, persistence, expiry | `capability_grants` |
| `capability.grant_used` | Grant use result and action refs | `capability_grant_uses` |
| `capability.grant_expired` | Grant ID and expiry reason | `capability_grants` |
| `capability.grant_revoked` | Grant ID, actor, reason | `capability_grants` |
| `tool.definition_registered` | Tool schema, scopes, risk, exposure, instrumentation level | `tool_definitions` |
| `tool.definition_updated` | Tool schema/status/exposure change | `tool_definitions` |
| `tool.call_requested` | Tool name, origin, input artifact/ref | `tool_calls`, `items` |
| `tool.invocation_started` | Tool handler/invocation metadata | `tool_invocations`, `tool_calls` |
| `tool.call_started` | Timeline status that invocation is running | `tool_calls` |
| `tool.output_artifact_recorded` | Tool output artifact metadata | `tool_invocations`, `artifacts` |
| `tool.output_observed` | Output artifact/ref | `tool_calls`, `items` |
| `tool.observation_recorded` | Structured or partial external tool observation | `tool_observations`, `tool_calls` |
| `tool.instrumentation_downgraded` | Tool visibility downgraded to partial or observed-only | `tool_observations`, `tool_calls` |
| `tool.call_completed` | Result metadata, output artifact/ref | `tool_calls`, `items` |
| `tool.call_failed` | Error and retryability | `tool_calls`, `items` |
| `tool.call_canceled` | Actor/source and cancellation reason | `tool_calls`, `items` |
| `tool.result_delivered` | Adapter delivery status for a tool result | `tool_calls`, `items` |

### Memory, Evaluation, And Checkpoints

| Event kind | Payload summary | Projects to |
| --- | --- | --- |
| `memory.job_requested` | Source query and operation job metadata | `memory_jobs` |
| `memory.job_completed` | Operation result counts and emitted range | `memory_jobs` |
| `memory.record_ingested` | Memory fields, source refs, provenance | `memory_records`, `memory_sources`, `memory_refs` |
| `memory.record_promoted` | Review decision and reviewed state | `memory_records` |
| `memory.record_invalidated` | Record ID, reason, actor | `memory_records`, `memory_refs` |
| `memory.record_superseded` | Old/new memory record relation | `memory_records` |
| `memory.index_updated` | Index kind/version/status | `memory_index_entries` |
| `memory.packet_built` | Packet purpose, budget, included/excluded refs | `memory_packets` |
| `memory.packet_attached` | Packet attached to run/turn/session | `memory_packets`, `sessions`, `runs`, `turns` |
| `memory.export_requested` | Export format, scope, destination metadata | `memory_jobs` |
| `memory.export_completed` | Export artifact and counts | `memory_jobs`, `artifacts` |
| `evaluation.requested` | Criteria and target | `evaluations` |
| `evaluation.recorded` | Result, reviewer, evidence refs | `evaluations` |
| `evidence.recorded` | Evidence kind, artifact/source refs, confidence | `evidence` |
| `checkpoint.created` | Kind, artifact refs, recovery instructions | `checkpoints` |
| `checkpoint.restored` | Checkpoint ID, actor, result | `checkpoints`, `sessions` |
| `recovery.started` | Startup recovery attempt and scan inputs | `recovery_attempts` |
| `recovery.completed` | Recovery outcome and emitted event range | `recovery_attempts` |

## Read Models

Read models are query surfaces for CLI/dashboard/mobile/voice. They are not independent truth.

### ProjectReadModel

Includes:

- Project metadata.
- Active task counts by status.
- Active sessions and agent counts.
- Latest evidence/review status.
- Workpad path health.

### TaskReadModel

Includes:

- Title, source ref, status.
- Acceptance/evidence refs when known.
- Active session/run/agent.
- Latest summary.
- Pending permissions.
- Review state.

### AgentReadModel

Includes:

- Agent name, adapter, runtime target, provider connector.
- Health/status.
- Capability profile summary.
- Current session/run/turn.
- Recent tool calls.
- Known adapter/provider limitations.

### SessionReadModel

Includes:

- Session title/status/task/agent.
- Current goal, latest confidence, and source refs for both.
- Run status and restart/recovery markers.
- Current turn status.
- Ordered items with final or streaming state.
- Latest summary, blockers, pending permissions, recent tool calls.
- External refs visible for debugging but not used as display identity.

### PermissionQueue

Includes:

- Pending permission requests.
- Scope/risk/source.
- Affected session/run/tool.
- Staleness/restart markers.
- Decision history.

### ToolCatalogReadModel

Includes:

- Registered tools and schemas.
- Required scopes, risk, exposure, status, and instrumentation level.
- Handler kind and native-tool limitation notes.

Tool catalog rows are display and routing projections over `tool_definitions`, not independent authority.

### MemoryRecordReadModel

Includes:

- Memory record kind, scope, confidence, review state, validity window, and redaction state.
- Source refs to events, artifacts, markdown anchors, or external imports.
- Supersession/invalidation links.

### MemoryPacketReadModel

Includes:

- Packet purpose, target task/session/run/turn, budget, included item counts, and explanation artifact.
- Inclusion/exclusion reasons for debugging prompt context.

### EventStream

Includes:

- Ordered events after a sequence.
- Optional filters: project/task/session/run/agent/kind.
- Projection watermark so clients can detect whether read models have caught up.

## SQLite Prototype Layout

Database root:

```text
.capo/
  capo.sqlite
  artifacts/
    raw/
    logs/
    prompts/
    tools/
    diffs/
    reviews/
    checkpoints/
  exports/
```

The default local path is project-local `.capo/` for dogfooding. A later server mode can move the same layout under a user or server data directory.

Minimum tables:

```text
schema_migrations(version, applied_at)
events(sequence, event_id, schema_version, occurred_at, recorded_at, actor, project_id, task_id, agent_id, session_id, run_id, turn_id, item_id, kind, payload_json, payload_hash, idempotency_key, external_ref_json, redaction_state, causation_id, correlation_id)
raw_events(raw_event_id, source, adapter_config_id, provider_connector_id, runtime_target_id, external_ref_json, event_id, artifact_id, observed_at, payload_hash)
artifacts(artifact_id, project_id, session_id, run_id, kind, uri, content_hash, size_bytes, redaction_state, created_at)
projections(name, last_sequence, updated_at, status, error)
projects(project_id, name, workspace_root, workpad_root, status, updated_at)
tasks(task_id, project_id, source_kind, source_ref_json, title, capo_execution_status, workpad_status_observed, active_session_id, updated_at)
agents(agent_id, project_id, name, adapter_config_id, runtime_target_id, provider_connector_id, capability_profile_id, status, metadata_json, updated_at)
adapter_configs(adapter_config_id, project_id, name, adapter_kind, command_template_json, default_args_json, stdin_mode, stdout_format, stderr_policy, adapter_capabilities_json, version_observed, status, created_at, updated_at)
adapter_session_refs(adapter_session_ref_id, session_id, adapter_config_id, external_session_ref_json, external_turn_ref_json, protocol_version, adapter_state, raw_event_cursor, attach_supported, resume_supported, load_supported, created_at, updated_at)
provider_connectors(provider_connector_id, project_id, provider_kind, credential_scope, productization_allowed, auth_ref_kind, auth_ref, account_label, workspace_label, usage_capability, revocation_instructions, redaction_policy_json, status, created_at, updated_at)
adapter_capability_snapshots(adapter_capability_snapshot_id, adapter_config_id, provider_connector_id, observed_at, adapter_version, provider_version, auth_mode_observed, supports_streaming, supports_interrupt, supports_resume, supports_permission_prompts, supports_structured_tool_results, supports_usage_metadata, native_tools_json, limitations_json)
runtime_targets(runtime_target_id, project_id, name, runner_kind, workspace_root, artifact_root, default_cwd, env_policy_json, capability_profile_id, connectivity_endpoint_id, status, created_at, updated_at)
runtime_process_refs(runtime_process_ref_id, runtime_target_id, run_id, external_pid, process_group_ref_json, remote_process_ref_json, started_at, last_heartbeat_at, status, redaction_state)
connectivity_endpoints(connectivity_endpoint_id, project_id, name, tunnel_kind, address_ref_json, identity_ref_json, auth_ref, exposure, allowed_channels_json, status, created_at, updated_at)
resolved_endpoints(resolved_endpoint_id, connectivity_endpoint_id, owner_kind, owner_id, channel_kind, resolved_uri, identity_fingerprint, expires_at, redaction_state, created_at)
sessions(session_id, project_id, task_id, agent_id, title, status, current_goal, current_goal_artifact_id, latest_confidence, external_session_ref_json, latest_summary, last_sequence, updated_at)
runs(run_id, session_id, runtime_process_ref_json, adapter_instance_ref_json, status, started_at, ended_at, exit_status_json, recovery_of_run_id, updated_at)
turns(turn_id, session_id, run_id, origin_command_id, role, status, created_at, completed_at)
items(item_id, turn_id, kind, status, stream_state, ordinal, summary, artifact_id, external_item_ref_json, content_hash, chunk_count, message_boundary_confidence, adapter_timeline_key_id, import_confidence, updated_at)
tool_calls(tool_call_id, session_id, turn_id, item_id, tool_name, tool_origin, permission_decision_id, status, started_at, completed_at, latency_ms, input_artifact_id, output_artifact_id, external_tool_ref_json)
tool_definitions(tool_definition_id, tool_id, display_name, origin, handler_kind, schema_json, required_scopes_json, risk, redaction_policy_json, exposure, instrumentation_level, status, created_at, updated_at)
tool_invocations(tool_invocation_id, tool_call_id, tool_definition_id, session_id, run_id, turn_id, adapter_config_id, provider_connector_id, runtime_process_ref_id, external_tool_ref_json, actor_id, subject_json, permission_decision_id, capability_grant_use_id, correlation_id, instrumentation_level, input_artifact_id, output_artifact_id, status, started_at, completed_at)
tool_observations(tool_observation_id, tool_call_id, tool_invocation_id, source, external_tool_ref_json, observed_status, confidence, raw_event_id, artifact_id, observed_at)
permission_decisions(permission_decision_id, request_id, session_id, run_id, tool_call_id, capability_profile_id, decision, persistence, source, scope_json, expires_at, revoked_at, created_at)
permission_requests(permission_request_id, session_id, run_id, tool_call_id, capability_profile_id, scope_json, risk, source, adapter_options_json, status, created_at, decided_at)
capability_profiles(capability_profile_id, project_id, name, description, default_scopes_json, risk_level, decision_mode, created_at, updated_at, disabled_at)
capability_grants(capability_grant_id, capability_profile_id, scope_json, effect, subject_json, decision_id, source, persistence, expires_at, revoked_at, revocation_reason, created_at)
capability_grant_uses(capability_grant_use_id, capability_grant_id, permission_request_id, session_id, run_id, tool_call_id, used_at, result)
checkpoints(checkpoint_id, project_id, session_id, run_id, kind, artifact_id, created_at, restored_at)
commands(command_id, project_id, actor_id, origin, target_json, intent, status, idempotency_key, received_at, completed_at)
evidence(evidence_id, project_id, task_id, session_id, run_id, kind, artifact_id, source_ref_json, confidence, created_at)
evaluations(evaluation_id, project_id, task_id, session_id, run_id, status, criteria_json, result_json, reviewer, evidence_id, created_at)
memory_records(memory_record_id, project_id, scope, scope_owner_ref_json, subject_ref_json, sensitivity_classification, record_kind, subject, predicate, object, body, confidence, review_state, source_count, valid_from, valid_until, supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state, created_at, updated_at)
memory_sources(memory_source_id, memory_record_id, source_kind, source_event_id, source_artifact_id, source_path, source_anchor, source_content_hash, source_sequence, quote_artifact_id, observed_at)
memory_index_entries(memory_index_entry_id, memory_record_id, index_kind, index_version, indexed_text_hash, backend_ref, status, indexed_at)
memory_packets(memory_packet_id, project_id, task_id, agent_id, session_id, run_id, turn_id, purpose, budget_tokens, selection_policy, included_items_json, excluded_items_json, explanation_artifact_id, packet_artifact_id, created_at)
memory_jobs(memory_job_id, project_id, source_query_json, job_kind, status, started_at, completed_at, emitted_sequence_start, emitted_sequence_end, error)
memory_refs(memory_ref_id, project_id, source_event_id, source_artifact_id, memory_record_id, status, provenance_json, created_at)
recovery_attempts(recovery_attempt_id, started_at, completed_at, status, emitted_sequence_start, emitted_sequence_end, notes_json)
adapter_replay_batches(acp_replay_batch_id, session_id, external_session_ref_json, source, started_at, completed_at, load_request_id, prompt_request_id, recovery_attempt_id, raw_update_count, normalized_sequence_start, normalized_sequence_end, status, summary_json)
adapter_raw_updates(acp_raw_update_id, acp_replay_batch_id, external_session_ref_json, batch_index, jsonrpc_method, session_update_kind, external_item_ref_json, payload_hash, payload_artifact_id, observed_at, dedupe_confidence)
adapter_replay_candidates(adapter_replay_candidate_id, acp_replay_batch_id, adapter_timeline_key_id, candidate_kind, payload_hash, payload_artifact_id, status, match_event_id, created_at)
adapter_timeline_keys(adapter_timeline_key_id, session_id, external_session_ref_json, kind, stable_ref, synthetic_ref, confidence, first_sequence, last_sequence)
```

Minimum indexes:

- `events(sequence)`
- `events(project_id, sequence)`
- `events(session_id, sequence)`
- `events(kind, sequence)`
- `events(project_id, idempotency_key)` unique where idempotency key is not null
- `raw_events(source, payload_hash)`
- `raw_events(adapter_config_id, external_ref_json)` where stable external refs exist
- `items(turn_id, ordinal)`
- `items(adapter_timeline_key_id)`
- `tool_calls(session_id, status)`
- `tool_definitions(tool_id)` unique
- `tool_definitions(origin, status)`
- `tool_invocations(tool_call_id)`
- `tool_invocations(session_id, status)`
- `tool_observations(tool_call_id, observed_at)`
- `tool_observations(external_tool_ref_json)` where stable external refs exist
- `adapter_configs(project_id, adapter_kind, status)`
- `adapter_session_refs(session_id, adapter_config_id)`
- `provider_connectors(project_id, provider_kind, status)`
- `adapter_capability_snapshots(adapter_config_id, observed_at)`
- `runtime_targets(project_id, status)`
- `runtime_process_refs(run_id)`
- `runtime_process_refs(runtime_target_id, status)`
- `connectivity_endpoints(project_id, tunnel_kind, status)`
- `resolved_endpoints(owner_kind, owner_id, channel_kind)`
- `permission_requests(status, created_at)`
- `permission_decisions(session_id, revoked_at)`
- `capability_profiles(project_id, name)`
- `capability_grants(capability_profile_id, revoked_at, expires_at)`
- `capability_grant_uses(capability_grant_id, used_at)`
- `evidence(task_id, created_at)`
- `memory_records(project_id, scope, review_state)`
- `memory_records(project_id, scope_owner_ref_json, review_state)`
- `memory_records(project_id, sensitivity_classification, review_state)`
- `memory_records(project_id, record_kind, confidence)`
- `memory_sources(memory_record_id)`
- `memory_sources(source_event_id)`
- `memory_sources(source_artifact_id)`
- `memory_sources(source_path, source_anchor)`
- `memory_index_entries(memory_record_id, index_kind, status)`
- `memory_packets(session_id, created_at)`
- `memory_packets(task_id, created_at)`
- `memory_jobs(project_id, status, started_at)`
- `recovery_attempts(started_at)`
- `adapter_replay_batches(session_id, source, started_at)`
- `adapter_raw_updates(acp_replay_batch_id, batch_index)` unique
- `adapter_replay_candidates(acp_replay_batch_id, status)`
- `adapter_timeline_keys(session_id, kind, stable_ref)`
- `adapter_timeline_keys(session_id, kind, synthetic_ref)` where `stable_ref` is null

## Artifact Rules

- Event payloads should stay small and structured.
- Large text, logs, raw JSONL, diffs, binary data, and long transcripts become artifacts.
- Artifact paths are relative to `.capo/artifacts/` unless explicitly external.
- Every artifact records a content hash.
- Artifacts that may contain secrets must carry `redaction_state`: `unknown`, `redacted`, `contains_sensitive`, or `safe`.
- Raw voice transcripts are not retained by default; if retained for a reviewed feature, they must be explicit artifacts with sensitive redaction state.
- Generated memory artifacts live under `.capo/artifacts/memory/` and remain derived from source events/files.

Artifact privacy contract:

- Raw provider, adapter, runtime, tool, and ACP replay artifacts are local-only by default and must have bounded retention metadata before they are written under `.capo/artifacts/`.
- Normal persistence requires `redaction_state = safe` or `redacted`. Artifacts with `unknown` or `contains_sensitive` cannot be attached to read models, memory packets, evidence exports, or durable provider-smoke results.
- If a raw artifact cannot be classified before write, it must be written only to a quarantine path under `.capo/artifacts/quarantine/` with a short local retention window and must be excluded from projection-visible state until classified.
- Provider smoke tests must fail when persistent artifacts contain credential material, OAuth tokens, API keys, browser cookies, subscription session material, raw sensitive transcripts, or unclassified raw streams.
- Redaction failures are fail-closed: the controller records a safe error event and drops or quarantines the raw payload instead of persisting it as a normal artifact.

## Projection Rules

- Event append and projection watermark update happen in one transaction when synchronous projection is used.
- If projection fails, the event stays committed and `projections.status` records the failure.
- On startup, Capo checks every projection watermark and replays events after `last_sequence`.
- Rebuild is supported by clearing read-model tables and replaying events from sequence `1`.
- UI clients subscribe from a sequence and use read models for snapshots.
- UI state dedupe keys are Capo `sequence` and `event_id`, not adapter IDs.
- `PermissionQueue` is a read model over `permission_requests` joined to decisions and affected sessions/tools.
- Capability profile/grant read models are projections over capability and permission events.
- Runtime and connectivity read models are projections over runtime/connectivity events; they do not poll live processes directly.
- Runtime process start is request/event driven: `runtime.start_requested` is persisted before launch, then `runtime.process_started` or `runtime.process_start_failed` closes the attempt.
- Adapter/provider read models are projections over adapter/provider events and store non-secret metadata only.
- Provider connector use is gated before runtime launch; denied use emits `provider.connector_use_denied`.
- Tool catalog, invocation, and observation read models are projections over tool events. Observed-only native tools must remain labeled as partial visibility in session/agent/evaluation views.

## Workpad Status Boundary

Capo separates planning status from execution status:

- Markdown owns workpad task text and the workpad's visible `Status:` line until an explicit export/update feature is implemented.
- SQLite owns `capo_execution_status`, which answers whether Capo is actively working, blocked, reviewing, or done with its own execution attempt.
- `task.workpad_status_observed` records what Capo last read from markdown.
- `task.execution_status_changed` records Capo's operational status for scheduling, dashboard, and voice answers.
- When a future feature writes back to markdown, it must emit an explicit export event rather than silently treating the projection table as the workpad source of truth.

## Restart Recovery

Startup recovery follows this order:

1. Open SQLite and acquire the single-writer lock.
2. Run migrations.
3. Insert `recovery.started` with a new `recovery_attempt_id`.
4. Load projection watermarks and replay unprojected events.
5. Load sessions/runs with statuses that imply live work: `starting`, `active`, `waiting_for_input`, `waiting_for_permission`, `canceling`, `recovering`, `running`, `stopping`.
6. Ask `RuntimeRunner.health(...)` for known runtime process refs.
7. Ask each relevant `AgentAdapter` for attach/load capability and external session health when supported.
8. For every live-looking run:
   - If the process is alive and adapter can attach, emit `run.recovered` for the existing run and keep the session active.
   - If the process is alive but adapter cannot attach, emit `run.orphaned` and mark the session `recovering` or `failed` depending on policy.
   - If the process is gone and no terminal event exists, emit `run.exited` with unknown exit detail and mark the session `failed` or `waiting_for_input`.
   - If the session was waiting for permission, keep the permission queue pending but mark stale decisions as stale in the read model.
9. Write `checkpoint.created` for recovery snapshots when enough state exists to resume safely.
10. Emit `recovery.completed` with the emitted event sequence range.
11. Start serving input surfaces only after projections and recovery events have reached a consistent sequence.

Recovery invariants:

- No adapter raw replay mutates read models directly.
- Recovery emits new Capo events instead of editing old rows.
- A repeated restart must not create a second run recovery event for the same stable recovery observation. Recovery event idempotency keys use `(run_id, recovery_observation_kind, observed_runtime_state_hash)` and intentionally exclude `recovery_attempt_id`; the attempt ID remains payload/correlation metadata.
- Pending human approvals must survive restart.
- A session can be inspectable even when it is not resumable.

## Streaming And Dedupe Baseline

Generic adapter rules:

- Each adapter event should produce a stable `idempotency_key` when possible.
- Streaming chunks append to an `item_id` chosen by Capo when the adapter cannot provide stable item IDs.
- Finalization uses `item.completed`; repeated finalization with the same idempotency key is ignored.
- Raw stream chunks may be artifacted for debugging, but UI projections render item state from Capo events.
- Adapter replays are treated as input streams and run through the same idempotency path as live streams.
- If the adapter cannot provide stable IDs, Capo uses `(session_id, turn_id, item ordinal, chunk ordinal, payload_hash)` as a best-effort key and records confidence as low in the raw event metadata.

ACP-specific rules:

- ACP `session/resume` is preferred over `session/load` when Capo already has local event history and only needs to reconnect.
- ACP `session/load` opens an adapter replay batch, persists raw updates, stages non-projecting candidates, reconciles duplicates, appends only accepted events, then advances read-model watermarks.
- ACP tool calls use `toolCallId` as a stable timeline key.
- ACP plans are replacement projections.
- ACP message chunks in stable v1 lack stable message IDs, so Capo finalizes content hashes and records boundary confidence.
- See `acp-replay-dedupe.md` for the complete A2a design.
- See `runtime-tunnel.md` for the complete A4 runtime and connectivity design.
- See `protocol-provider.md` for the complete A5 adapter and provider design.

## Prototype E2E State Flow

The first fake-agent e2e test should prove this flow:

1. Register a project and fake agent.
2. Start a session for a task.
3. Append `command.received` and `command.accepted`.
4. Start a run and turn.
5. Emit item streaming events.
6. Emit a fake tool call, permission decision, tool result, and `tool.result_delivered`.
7. Complete the turn and session.
8. Restart the controller using the same SQLite database.
9. Rebuild read models from events.
10. Assert the session, items, tool call, permission decision, and evidence remain inspectable without duplicate UI items.

## Open Questions For Later Tasks

- A3: complete capability scope vocabulary and approval persistence semantics.
- A5/A5a: exact Codex/Claude event mapping and native tool observability limits.
- A6: exact derived-memory schema and invalidation rules.
- Prototype: whether `.capo/` is always project-local or can be configured through `CAPO_HOME` from the first scaffold.

## A2 Gate Evidence

A2 is complete when:

- Prototype entities are named and scoped.
- Prototype event kinds cover project/task/agent/session/run/turn/item/tool/permission/evaluation/checkpoint flows.
- Prototype read models are specified.
- SQLite and artifact layout are specified.
- Restart recovery behavior is specified.
- ACP-specific replay and dedupe rules are linked through `acp-replay-dedupe.md`.
