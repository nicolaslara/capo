# Capo Capability And Permission Model

## Objective

Define Capo's prototype capability and permission architecture: capability profiles, scopes, grants, permission requests, decisions, revocation, audit events, and ACP permission-option mapping.

This is the A3 architecture artifact. It keeps the first local prototype permissive while preserving a modular decision boundary for static policy, user approval, and future security-agent review.

## Design Rules

- Capo records permission decisions even when everything is allowed.
- `CapabilityProfile` is data: the declared authority envelope for a session/run.
- `PermissionPolicy` is behavior: the module that evaluates requests against a profile and context.
- Grants are durable, scoped, expirable, and revocable.
- A grant is not proof of enforcement. Runtime/tool layers must state what they actually enforce.
- Local prototype policy is `AllowTrustedLocalProfilePolicy`, but it still emits request, decision, grant, and audit events.
- ACP permission options are adapter-native choices, not Capo's policy model. Capo maps them into its own decision/grant vocabulary and returns the chosen ACP option ID.
- Voice transcript and subscription/session material are privileged data scopes, not ordinary text.

## Static Dispatch Shape

Prototype enum:

```text
enum PermissionPolicy {
  AllowTrustedLocalProfile(AllowTrustedLocalProfilePolicy),
  Static(StaticPolicy),
  UserApproval(UserApprovalPolicy),
  SecurityAgent(SecurityAgentPolicy),
  Fake(FakePermissionPolicy),
}
```

Policy contract:

```text
evaluate(PermissionRequest, CapabilityProfile, PolicyContext) -> PermissionDecision
grant(CapabilityGrantRequest) -> CapabilityGrant
revoke(CapabilityGrantId, Reason) -> RevocationResult
explain(PermissionDecisionId) -> DecisionExplanation
```

Prototype implementation:

- `AllowTrustedLocalProfilePolicy` is the default.
- It returns allow for every request whose scope is explicitly present in the selected trusted local project profile.
- Critical scopes such as public network exposure, credential-material reads/writes, and raw voice transcript reads are excluded unless the selected profile explicitly includes them.
- It still records the requested scope, decision source, selected ACP option if any, grant persistence, and audit reason.
- Tests must prove the trusted-local allow path emits the same durable records as stricter policies would.

## Core Records

### CapabilityProfile

A named bundle of default scopes for an agent/session/run.

Fields:

- `capability_profile_id`
- `project_id?`
- `name`
- `description`
- `default_scopes`
- `risk_level`: `low`, `medium`, `high`, `critical`
- `decision_mode`: `allow_trusted_local_profile`, `static`, `ask_user`, `security_agent`
- `created_at`
- `updated_at`
- `disabled_at?`

Prototype profiles:

- `trusted-local-dev`: all local scopes allowed, audit-only.
- `read-only-local`: read filesystem, git status/diff, workpad reads, no writes or shell execution.
- `reviewer`: read filesystem/workpads/events, no writes, no shell except configured test readers.
- `voice-control`: can read state summaries and submit steering commands, cannot access raw transcripts/secrets.

### CapabilityScope

Structured authority request or grant.

Fields:

- `scope_id`
- `domain`
- `action`
- `resource`
- `constraints`
- `risk`
- `redaction_policy`

Scope string shape:

```text
{domain}:{action}:{resource}
```

Scope strings are for matching and display. The enforceable resource identity must also live in typed `resource_ref` and `constraints` fields.

Examples:

- `filesystem:read:workspace`
- `filesystem:write:workspace`
- `filesystem:write:path`
- `shell:execute:workspace`
- `git:status:workspace`
- `git:diff:workspace`
- `git:commit:workspace`
- `network:connect:internet`
- `network:connect:private_tunnel`
- `browser:open:local_dashboard`
- `browser:control:remote_page`
- `tool:invoke:capo.session_summary`
- `tool:invoke:capo.workpad_read`
- `tool:invoke:external`
- `mcp:connect:server`
- `mcp:invoke:tool`
- `secret:read:provider_metadata`
- `secret:read:credential_material`
- `voice:read:transcript_summary`
- `voice:read:raw_transcript`
- `voice:approve:low_risk`

### CapabilityGrant

Durable permission to perform a scope.

Fields:

- `capability_grant_id`
- `capability_profile_id`
- `scope`
- `effect`: `allow`, `deny`
- `subject`: agent/session/run/tool/input-surface actor
- `decision_id`
- `source`: `allow_trusted_local_profile`, `static_policy`, `user`, `security_agent`, `imported`
- `persistence`: `once`, `until_turn_end`, `until_session_end`, `until_revoked`, `until_time`
- `expires_at?`
- `revoked_at?`
- `revocation_reason?`
- `created_at`

### PermissionRequest

Input to policy evaluation.

Fields:

- `permission_request_id`
- `request_kind`: `tool_call`, `runtime_action`, `input_surface_action`, `adapter_request`, `memory_access`
- `requested_scope`
- `subject`
- `resource_ref`
- `risk`
- `adapter_ref?`
- `tool_call_id?`
- `command_id?`
- `session_id?`
- `turn_id?`
- `reason`
- `created_at`

### PermissionDecision

Policy output.

Fields:

- `permission_decision_id`
- `permission_request_id`
- `decision`: `allow`, `reject`, `cancel`
- `decision_source`
- `selected_scope`
- `selected_persistence`
- `capability_grant_id?`
- `adapter_response?`
- `explanation`
- `created_at`

## Scope Domains

### Shell

Scopes:

- `shell:execute:workspace`
- `shell:execute:path`
- `shell:interrupt:process`
- `shell:terminate:process`

Prototype behavior:

- `AllowTrustedLocalProfilePolicy` allows shell execution only when the selected trusted profile includes the shell scope and logs command metadata, cwd, environment allowlist, and redaction state.
- Capo does not claim sandboxing until runtime enforcement exists.

### Filesystem

Scopes:

- `filesystem:read:workspace`
- `filesystem:write:workspace`
- `filesystem:read:path`
- `filesystem:write:path`
- `filesystem:delete:path`

Prototype behavior:

- Workspace read/write may be allowed for trusted local development.
- Path-specific scopes are stored even if v0 enforcement is coarse.
- Writes outside workspace are high risk and should not be auto-allowed outside the trusted local profile.
- Enforceable `resource_ref` is a canonical absolute path plus workspace root relationship.

### Git

Scopes:

- `git:status:workspace`
- `git:diff:workspace`
- `git:add:workspace`
- `git:commit:workspace`
- `git:branch:workspace`
- `git:push:remote`

Prototype behavior:

- Status/diff are low risk.
- Commit is medium risk and should record author/message.
- Push is high risk and not part of the first automated path.
- Enforceable `resource_ref` is repository root plus remote/branch/ref when relevant.

### Network

Scopes:

- `network:connect:internet`
- `network:connect:localhost`
- `network:connect:private_tunnel`
- `network:listen:local`
- `network:expose:public`

Prototype behavior:

- Localhost/local dashboard is low risk.
- Public exposure is high/critical and not enabled by trusted local dogfood without explicit profile selection.
- Enforceable `resource_ref` is host/origin/port/protocol plus direction.

### Browser

Scopes:

- `browser:open:local_dashboard`
- `browser:control:local_page`
- `browser:control:remote_page`
- `browser:read:page_content`

Prototype behavior:

- Browser access is separate from network access because it can expose authenticated sessions and page content.
- Enforceable `resource_ref` is browser context plus URL origin and page/session sensitivity.

### MCP And Tools

Scopes:

- `tool:list:capo`
- `tool:invoke:capo`
- `tool:invoke:capo.<tool_name>`
- `tool:invoke:adapter_native`
- `tool:invoke:provider_native`
- `mcp:connect:server`
- `mcp:invoke:tool`

Prototype behavior:

- Capo-exposed tools should route through `ToolExposure` and `PermissionPolicy`.
- Provider-native/adapter-native tools may be observed-only; mark instrumentation confidence explicitly.
- Enforceable `resource_ref` is Capo tool ID, MCP server ID/tool ID, or adapter/provider native tool reference.

### Secrets And Subscription Material

Scopes:

- `secret:read:provider_metadata`
- `secret:read:credential_material`
- `secret:write:credential_material`
- `subscription:launch:local_cli`
- `subscription:revoke:connector`

Prototype behavior:

- Provider metadata can be read when redacted.
- Credential material is critical risk and should not be read, copied, persisted, or synced by Capo.
- Launching subscription-backed local CLIs is allowed only through connector/runtime boundaries that scrub environment and logs.
- Enforceable `resource_ref` is a secret handle or connector ID, never raw secret material.

### Voice

Scopes:

- `voice:submit:command`
- `voice:read:state_summary`
- `voice:read:raw_transcript`
- `voice:approve:low_risk`
- `voice:approve:privileged`

Prototype behavior:

- Voice is a conversational Capo input surface.
- Summaries/status are allowed for authenticated local user sessions.
- Raw transcripts are sensitive artifacts and are not retained by default.
- Voice approvals are not enough for high-risk grants until explicit user-confirmation UX exists.
- Enforceable `resource_ref` is voice session ID, transcript artifact ID, or summary artifact ID.

## Permission Lifecycle

1. A boundary creates `PermissionRequest`.
2. Controller appends `permission.requested`.
3. `PermissionPolicy.evaluate(...)` produces `PermissionDecision`.
4. Controller appends `permission.decided`.
5. If allowed and persistence is not purely observational, controller creates `CapabilityGrant` and appends `capability.grant_created`.
6. Tool/runtime/adapter proceeds only after the controller records the decision.
7. Grant use emits `capability.grant_used`.
8. Expiry/revocation emits `capability.grant_expired` or `capability.grant_revoked`.

Failure rules:

- If policy evaluation fails, default to `reject` unless the profile explicitly marks the request as audit-only and local.
- If a pending approval is canceled, append `permission.decided` with `cancel`.
- If a request arrives after session cancellation, cancel it and close the queue.
- If a persisted grant is too broad for a future stricter policy, the stricter policy can revoke it by event; old events remain.

## ACP Permission Mapping

ACP `session/request_permission` contains:

- `sessionId`
- `toolCall` as `ToolCallUpdate`
- `options` as `PermissionOption[]`

ACP options have `optionId`, `name`, and `kind`.

Mapping:

| ACP option kind | Capo decision | Capo persistence | Grant behavior |
| --- | --- | --- | --- |
| `allow_once` | `allow` | `once` or `until_turn_end` | Create narrow grant for the current request/turn. |
| `allow_always` | `allow` | `until_revoked` or profile-defined expiry | Create scoped durable grant; never make it global without resource scope. |
| `reject_once` | `reject` | `once` or `until_turn_end` | No grant; record rejection for this request. |
| `reject_always` | `reject` | `until_revoked` or profile-defined expiry | Create scoped durable grant with `effect = deny`. |

Prototype `AllowTrustedLocalProfilePolicy` mapping:

- Choose the first ACP `allow_once` option when present.
- If only `allow_always` exists, choose it but downscope Capo grant persistence to `until_session_end` unless the profile explicitly allows durable remembered grants.
- If no allow option exists but a reject option exists, select the first provided `reject_once` / `reject_always` option ID and record a Capo `reject` decision.
- Use ACP outcome `cancelled` only when the prompt turn or permission request is actually canceled.
- If no selectable option exists, treat it as adapter error, record `permission.decided` with `cancel`, and fail the adapter request rather than inventing an ACP outcome.
- Still persist the ACP option list and chosen option ID in `adapter_options` / `adapter_response`.

Cancellation:

- If Capo sends ACP `session/cancel` during a pending ACP permission request, it must respond with ACP outcome `cancelled`.
- Capo records `permission.decided` with `decision = cancel` and closes any matching `PermissionQueue` row.

Important boundary:

- ACP remembered options do not become global Capo policy. They become scoped Capo grants or deny rules with subject/resource/expiry.

## State Model Additions

Add entities:

```text
capability_profiles(capability_profile_id, project_id, name, description, default_scopes_json, risk_level, decision_mode, created_at, updated_at, disabled_at)
capability_grants(capability_grant_id, capability_profile_id, scope_json, effect, subject_json, decision_id, source, persistence, expires_at, revoked_at, revocation_reason, created_at)
capability_grant_uses(capability_grant_use_id, capability_grant_id, permission_request_id, session_id, run_id, tool_call_id, used_at, result)
```

Add events:

- `capability.profile_created`
- `capability.profile_updated`
- `capability.grant_created`
- `capability.grant_used`
- `capability.grant_expired`
- `capability.grant_revoked`
- `permission.explanation_recorded`

Read models:

- `CapabilityProfileReadModel`: profile name, risk, default scopes, active grants, revoked grants.
- `PermissionQueue`: pending requests and decisions.
- `SessionReadModel`: capability profile, current grants, pending approvals.
- `AgentReadModel`: capability profile summary and recent permission decisions.

## Test Strategy

Prototype tests should prove:

1. `AllowTrustedLocalProfilePolicy` allows a shell/tool request in the trusted local profile and still emits request, decision, grant, and grant-use events.
2. A request with ACP `allow_once` maps to an allow-once/turn-scoped Capo grant and returns the matching ACP `optionId`.
3. ACP `allow_always` is downscoped unless the capability profile permits durable remembered grants.
4. ACP `reject_once` and `reject_always` produce no allow grant and close the permission queue.
5. ACP cancellation returns `cancelled` and records a canceled Capo decision.
6. Revocation prevents future grant use but does not alter old event history.
7. Voice can query summaries and submit steering commands without raw transcript retention.
8. Attempted credential-material reads are rejected outside explicit, reviewed connector flows.

## Recommendation

Implement the prototype with `AllowTrustedLocalProfilePolicy`, but code and test it through the same `PermissionPolicy` enum and durable state records used by stricter policies.

Confidence: high for the model shape and ACP option mapping. Confidence is medium for exact scope granularity; prototype traces should refine scopes before UI strings and enforcement claims are finalized.
