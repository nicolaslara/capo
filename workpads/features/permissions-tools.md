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

Status: pending

Acceptance:

- Add a stricter static policy variant for common local dogfood scopes.
- Keep allow/reject decisions durable and scoped.
- Preserve trusted-local as an explicit opt-in profile.

### PT2 - User Approval Queue

Status: pending

Acceptance:

- Represent pending approval requests in read models.
- Map allow-once/allow-always/reject-once/reject-always into durable scoped grants or denials.
- Provide CLI approval commands before web/mobile approval surfaces.

### PT3 - Tool Wrapper Expansion

Status: pending

Acceptance:

- Add wrapper/instrumentation points for shell, git, file read/write, and workpad operations where Capo executes the tool.
- Record input/output artifacts with safe/redacted classification.
- Keep policy decisions auditable.
