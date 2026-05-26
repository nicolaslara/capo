# Permissions And Tools Feature

## Objective

Harden Capo's capability, permission, and tool-instrumentation model beyond the trusted-local allow-all prototype while preserving the audit path proven in P8/P12.

## Prototype Inputs

- P8 proved tool request, permission decision, grant use, invocation, output artifact, and result delivery events.
- P12 proved these events appear in session read-model inspection.
- Initial policy intentionally allows broadly for local prototype work.

## Dependencies

- `CapabilityProfile` remains data; `PermissionPolicy` remains the decision boundary.
- Provider-native tools stay observed-only unless Capo executes them or receives structured lifecycle evidence.

## Tasks

### PT1 - Static Policy Variant

Status: completed

Acceptance:

- Add a stricter static policy variant for common local dogfood scopes.
- Keep allow/reject decisions durable and scoped.
- Preserve trusted-local as an explicit opt-in profile.

Evidence:

- `crates/capo-tools/src/lib.rs`
- `crates/capo-state/src/lib.rs`
- `crates/capo-controller/src/lib.rs`
- `crates/capo-tools/Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-tools`
- `cargo test -p capo-state artifacts_tool_grants_memory_and_evidence_are_persisted_and_rebuilt`
- `cargo test -p capo-controller denied_static_permission_stops_tool_invocation_in_controller_path`
- `cargo test -p capo-controller`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Decision:

- Add `PermissionPolicy::Static` with `read-only-local` and `reviewer` profiles while preserving `PermissionPolicy::TrustedLocal` as the explicit broad local prototype profile.
- Parse requested scopes as structured JSON arrays using `serde_json`; malformed, non-array, or non-string scope payloads fail closed.
- Scope grant IDs by session, effect, profile, and requested scopes so multiple same-session decisions do not collide in projections.
- Persist decision source, persistence, and explanation on capability grant projections.
- In the controller path, denied permissions block tool invocation, grant use, memory packet creation, and evidence recording.

Review:

- First focused permission review found three blockers: permissive scope parsing, grant ID collisions, and non-durable decision metadata. All were fixed.
- Second focused permission review found two remaining blockers: denied controller decisions still allowed tool execution, and permission lifecycle event IDs were session-scoped. Both were fixed before completion.

### PT2 - User Approval Queue

Status: completed

Acceptance:

- Represent pending approval requests in read models.
- Map allow-once/allow-always/reject-once/reject-always into durable scoped grants or denials.
- Provide CLI approval commands before web/mobile approval surfaces.

Evidence:

- `crates/capo-state/src/lib.rs`
- `crates/capo-cli/src/main.rs`
- `crates/capo-core/src/lib.rs`
- `crates/capo-state/Cargo.toml`
- `crates/capo-cli/Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-state permission_approval`
- `cargo test -p capo-cli permission_approval_queue_maps_decisions_to_scoped_grants`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Decision:

- Add `permission_approvals` as a read model with queued and decided states, session/tool-call refs, requested scope, subject, requested-by actor, reason, decision, and linked grant ID.
- Add `capo permission request`, `capo permission list`, and `capo permission decide` as the first operator approval queue surface.
- Map `allow_once` to a narrow `allow` grant with `persistence=once`; the grant subject is bound to approval ID and any session/tool-call refs.
- Map `allow_always` to `allow` with `persistence=until_revoked`, but only for Capo-owned read/status scopes in this PT2 CLI path.
- Map `reject_once` to a decided approval without a reusable deny grant.
- Map `reject_always` to a scoped durable `deny` grant with `persistence=until_revoked`.
- Move approval decision writes into the state-store transaction with a pending-status guard so concurrent/conflicting decisions cannot both commit.
- Emit `capability.grant_created` when a decision creates a grant, preserving the audit lifecycle used by the controller path.
- Validate projection JSON in the state layer so malformed non-CLI approval/grant projections fail before commit.

Review:

- First focused review found blockers in concurrent decisions, broad durable `allow_always`, once decisions becoming reusable grants, missing grant-created audit events, and CLI-only JSON validation.
- Fixes were applied and re-reviewed. The second focused review found no blockers.

### PT3 - Tool Wrapper Expansion

Status: completed

Acceptance:

- Add wrapper/instrumentation points for shell, git, file read/write, and workpad operations where Capo executes the tool.
- Record input/output artifacts with safe/redacted classification.
- Keep policy decisions auditable.

Evidence:

- `crates/capo-tools/src/lib.rs`
- `crates/capo-tools/Cargo.toml`
- `Cargo.lock`
- `cargo test -p capo-tools`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`

Decision:

- Add `ToolExposure::Runtime(RuntimeToolWrappers)` as the first non-fake wrapper boundary.
- Register wrapper definitions for `capo.shell_run`, `capo.git_status`, `capo.git_diff`, `capo.file_read`, `capo.file_write`, and wrapper-backed `capo.workpad_read`.
- Route shell and git wrappers through `LocalProcessRunner` so process execution, output limits, redaction, and runtime output artifacts stay behind the runtime boundary.
- Keep file and workpad wrappers workspace-bound. `workpad_read` is restricted to `TASKS.md`, `project.md`, and `workpads/*.md` paths rather than arbitrary workspace files.
- Record wrapper input artifacts and output artifacts with stable hashes, sizes, redaction state, and summaries.
- Apply configured redaction rules to wrapper-owned input artifacts, not just runtime stdout/stderr.
- Sanitize tool-call and run IDs before using them as artifact path components.
- Bind split authorizations to tool, session, run, tool call, profile, scope, and input/context hash before invocation so a prior allow decision cannot be replayed against a different request.
- Denied static policy decisions cancel before invocation and do not create wrapper artifacts.

Review:

- Focused wrapper review found blockers in split authorization replay, `workpad_read` arbitrary file reads, unsanitized artifact path components, unredacted input artifacts, and misleading permission event status. All were fixed with regression tests.
- Re-review found remaining blockers in same-tool authorization replay and unsanitized runtime run IDs. Both were fixed with regression tests.
- Final re-review found one more Capo-owned registry replay issue: newline-concatenated context hashing was ambiguous. Context hashing now uses length-prefix encoding, with a regression test for ambiguous newline-bearing fields.

### PT4 - ACP Client Capability Gating

Status: completed

Acceptance:

- Add an executable policy helper that decides whether Capo may advertise ACP filesystem and terminal client capabilities.
- Gate ACP filesystem read/write and terminal capability on both registered wrapper `ToolDefinition` rows and the selected permission policy.
- Keep the helper provider-free: do not start ACP agents, provider CLIs, runtimes, tunnels, or read credential/session material.
- Cover trusted-local, read-only, and missing-wrapper cases with tests.

Evidence:

- `AcpClientCapabilityPlan` and `AcpClientCapabilityDecision` in `../../crates/capo-tools/src/lib.rs`.
- `cargo test -p capo-tools acp_client_capabilities -- --nocapture`: passed.
- `cargo test -p capo-tools static_read_only_policy_allows_read_tools_and_denies_writes -- --nocapture`: passed.

Decision:

- Gate ACP `filesystem.read_text_file`, `filesystem.write_text_file`, and `terminal` advertisement through the Capo tool catalog plus `PermissionPolicy`.
- Treat missing backing wrapper definitions as fail-closed, even for trusted-local policy.
- Update static read-only/reviewer profiles to include read-only wrapper invocation scopes for `capo.file_read`, `capo.git_status`, and `capo.git_diff`; keep `capo.file_write` and `capo.shell_run` denied.
- Keep the helper provider-free. It does not start ACP agents, provider CLIs, runtimes, tunnels, or inspect credential/session material.

### PT5 - ACP Session Setup Capability Plan

Status: completed

Acceptance:

- Add an ACP adapter setup plan that consumes the Capo tool capability gate before advertising filesystem or terminal client capabilities.
- Keep setup planning separate from launching ACP agents, provider CLIs, runtimes, or tunnels.
- Record setup safety metadata: protocol version, advertised capabilities, MCP server count, credential policy, runtime-started flag, and provider-executed flag.
- Cover read-only and missing-wrapper cases with tests.

Evidence:

- `AcpAdapter::session_setup_plan(...)` and `AcpSessionSetupPlan` in `../../crates/capo-adapters/src/lib.rs`.
- `cargo test -p capo-adapters acp_session_setup -- --nocapture`: passed.

Decision:

- Keep ACP as an adapter boundary: session setup consumes `capo-tools` capability decisions rather than duplicating permission logic in the adapter.
- Advertise only capabilities approved by the executable tool plan. Read-only policy advertises `filesystem.read_text_file` only; missing backing wrappers fail closed.
- Keep MCP server configs at `mcp_server_count=0` for this setup scaffold until a user-approved MCP config path exists.

### PT6 - ACP Client Handler Wrapper Routing

Status: completed

Acceptance:

- Route ACP filesystem and terminal client handler calls into Capo wrapper requests only when the setup plan advertised the matching client capability.
- Map `fs/read_text_file` to `capo.file_read`, `fs/write_text_file` to `capo.file_write`, and `terminal/run` to `capo.shell_run`.
- Keep routing provider-free and execution-free: do not launch ACP agents, provider CLIs, runtimes, or tunnels.
- Cover read-only denial of write/terminal calls and trusted-local terminal routing with tests.

Evidence:

- `AcpSessionSetupPlan::wrapper_request_for_client_call(...)` and `AcpClientCall` in `../../crates/capo-adapters/src/lib.rs`.
- `cargo test -p capo-adapters acp_client -- --nocapture`: passed.
- `cargo test -p capo-adapters acp_terminal -- --nocapture`: passed.

Decision:

- Treat ACP client handlers as transport adapters over Capo wrapper tools, not as direct filesystem or terminal execution.
- Refuse handler calls when the setup plan did not advertise the capability, even if the raw method name is recognized.
- Leave actual wrapper invocation to the controller/tool boundary so permission, artifacts, redaction, and audit lifecycle stay centralized.

### PT7 - Adapter Native Tool Observation Contract

Status: completed

Acceptance:

- Add an adapter-layer record for provider/ACP native tool observations that labels them as observed-only rather than governed executions.
- Derive observations from normalized adapter tool events without launching providers, runtimes, tunnels, or tools.
- Preserve external tool refs, tool names, observed status, source adapter, raw event hash, and confidence.
- Cover ACP structured tool updates and Codex/Claude fixture tool events with tests.

Evidence:

- `AdapterToolObservation`, `NormalizedAdapterEvent::tool_observation()`, and `AdapterFixtureParse::tool_observations()` in `../../crates/capo-adapters/src/lib.rs`.
- `cargo test -p capo-adapters adapter_tool_observations -- --nocapture`: passed.

Decision:

- Treat provider/ACP native tool updates as observed-only adapter facts unless Capo executed a registered wrapper tool.
- Preserve source adapter, external tool ref, tool name, observed status, raw event hash, and confidence for future read-model/evaluation ingestion.
- Use stable adapter timeline confidence to mark observations as high-confidence; heuristic and missing timeline confidence downshift to medium/low.
- Keep this contract provider-free and execution-free. It parses existing normalized fixture events only.

### PT8 - Observed-Only Tool Observation State Projection

Status: completed

Acceptance:

- Add durable state projection storage for observed-only native tool observations.
- Keep observations separate from governed `tool_calls` / `ToolInvocation` records.
- Preserve session, optional tool-call link, source, external ref, tool name, observed status, instrumentation level, confidence, raw event hash, optional artifact, and rebuild sequence.
- Cover append/read and projection rebuild behavior with tests.

Evidence:

- `ToolObservationProjection`, `tool_observations` table, and `tool_observations_for_session(...)` in `../../crates/capo-state/src/lib.rs`.
- `cargo test -p capo-state tool_observations -- --nocapture`: passed.

Decision:

- Persist observed-only native tool facts in a dedicated `tool_observations` projection instead of overloading `tool_calls`.
- Add `tool.observation_recorded` as the event kind for projection append/rebuild evidence.
- Keep the projection generic enough for ACP, Codex, Claude, provider-native, runtime-output, or manual observations while still requiring explicit instrumentation level and confidence.

### PT9 - Query And Evidence Visibility For Tool Observations

Status: completed

Acceptance:

- Surface observed-only native tool observations through shared query/session dashboard rows.
- Render observed-only tool observations in CLI dashboard and session evidence exports without treating them as governed tool calls.
- Preserve source, external ref, tool name, observed status, instrumentation level, confidence, raw event hash, and artifact ref in operator-visible output.
- Keep the slice provider-free and execution-free.
- Cover shared query and CLI rendering with regression tests.

Evidence:

- `SessionDashboardRow::tool_observations` in `../../crates/capo-query/src/lib.rs`.
- CLI dashboard and evidence rendering in `../../crates/capo-cli/src/main.rs`.
- `cargo test -p capo-query project_dashboard_aggregates_agents_sessions_runs_evidence_and_events -- --nocapture`: passed.
- `cargo test -p capo-cli prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence -- --nocapture`: passed.

Decision:

- Surface observations through the existing session dashboard row so CLI, voice, web, and mobile consumers can use the same query contract.
- Render observed-only native tool facts in a separate CLI/evidence section rather than merging them into governed `tool_call` rows.
- Preserve source, external ref, status, instrumentation level, confidence, raw event hash, and artifact ref so future evaluation work can distinguish partial provider visibility from Capo-executed wrapper tools.
