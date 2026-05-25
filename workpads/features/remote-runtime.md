# Remote Runtime And Tunnel Feature

## Objective

Add remote execution and connectivity adapters without collapsing the separation between runtime process ownership and tunnel/exposure mechanics.

## Prototype Inputs

- P5 proved local process lifecycle and redacted output artifacts.
- Architecture keeps `RuntimeRunner` and `ConnectivityTunnel` separate.
- P15 defers remote runtime until local real-agent semantics are reliable.

## Dependencies

- Real local Codex/Claude semantics should be proven before remoteizing them.
- Public exposure requires explicit permission and audit events.

## Tasks

### RR1 - Remote Runtime Contract Test

Status: pending

Acceptance:

- Add a fake or loopback remote runtime contract test that proves runtime refs, health, interrupt, terminate, and recovery events.
- Do not require Tailscale or cloud credentials.

### RR2 - Tunnel Adapter Stub

Status: pending

Acceptance:

- Add a tunnel adapter stub for endpoint resolution and health.
- Keep tunnel records separate from runtime process records.
- Record exposure scope and permission requirements.

### RR3 - Explicit Exposure Policy

Status: pending

Acceptance:

- Require durable permission events before public or remote-control exposure.
- Make revocation and health visible in read models.
