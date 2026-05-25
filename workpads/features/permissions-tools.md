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

Status: pending

Acceptance:

- Add wrapper/instrumentation points for shell, git, file read/write, and workpad operations where Capo executes the tool.
- Record input/output artifacts with safe/redacted classification.
- Keep policy decisions auditable.
