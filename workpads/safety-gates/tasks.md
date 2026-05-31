# Safety Gates Tasks

## Objective

Make Capo's real turn loop safe to run unattended-capable. Wire the existing
`PermissionPolicy`/`ToolExposure` engine into the loop's decide step with grant
read-back, revoke, and expiry; fix the `TrustedLocal` critical-scope hole; add a
real `VerificationRunner` that runs check/lint/test and a `score_run` computed
from OBSERVED evidence only; add a single-writer workspace lock; and add
controller-owned shadow-git checkpoint/rollback plus liveness-aware restart
recovery. The hard machinery (scope engine, path containment, durable grant
store, event-sourced state) is already built; this workpad makes it enforce.

## Status

Planned. Phase 4 - Make it safe, sub-phased as enforcement | verification |
checkpoint-recovery. Depends on `real-turn-loop`, `streaming-transport`, and
`tools-aci`. `SG0` defines routing and scope. All implementation tasks pending.

## Feature Set

- Loop-level permission enforcement with grant read-back, revoke, and expiry.
- `AgentAdapter` permission round-trip and ACP option mapping against fakes.
- A `CapabilityGrantRevoked` (and optional `CapabilityGrantExpired`) event kind
  plus `created_at`/`expires_at`/`revoked_at` projection columns.
- A `TrustedLocal` fix that denies un-granted critical scopes.
- A single-writer workspace lock / session-scoped write lease.
- A real `VerificationRunner` emitting true exit-status pass/fail evidence.
- `score_run` computed from observed evidence and wall-clock timing.
- Controller-owned shadow-git checkpoint/rollback upgrading the RTL floor.
- Liveness-aware restart recovery replacing blunt `exited_unknown`.

## SG0 - Workpad, Routing, Scope, Sub-Phase Milestones, And Acceptance Invariant

Status: pending.

Acceptance:

- Confirm this is its own workpad (not folded into `real-turn-loop`,
  `tools-aci`, or `goal-autonomy`) and record the scope decision in
  `knowledge.md`.
- State the safety boundary: the server/controller owns enforcement, grant
  lifecycle, the verification gate, the workspace lock, checkpoint/rollback, and
  recovery; ACP `request_permission` is an adapter round-trip below the
  `AgentAdapter` boundary; clients only render permission cards and verification
  progress and never own enforcement state.
- Define the three sub-phase milestones (enforcement, verification,
  checkpoint-recovery) and the task-to-sub-phase mapping.
- Record the relation to `workpads/architecture/capability-permissions.md`: this
  workpad IMPLEMENTS that design (lifecycle steps 1-8, the ACP option mapping
  table, and the critical-scope exclusion rule), it does not redesign it; list
  which designed events (`capability.grant_revoked`, `capability.grant_expired`)
  and projection columns graduate from design to code here.
- Declare the seam to `tools-aci` (ACI defines/instruments tools and emits typed
  test/lint evidence; safety-gates enforces permissions and computes `score_run`
  over that evidence) and to `goal-autonomy` (safety-gates ships the
  single-writer lock and checkpoint/rollback that `GO8` consumes).
- Record the acceptance+verification invariant: no task in this workpad
  completes on operator self-attestation alone; every manual smoke is paired
  with a deterministic assertion (event/wire snapshot, exit status, or replay).

Verification:

- `workpads/safety-gates/tasks.md`, `knowledge.md`, and `references.md` exist and
  follow the conventional format.
- Scope decision and seams cross-checked against `goal-orchestration/tasks.md`
  (`GO8`) and `workpads/architecture/capability-permissions.md`.
- `git diff --check`.

## SG1 - Wire PermissionPolicy And ToolExposure Enforcement Into The Decide Step

Status: done.

Acceptance:

- In the real turn loop's decide step (the `RealBoundaryController` introduced by
  `real-turn-loop`, replacing the inert path where `FakeBoundaryController` holds
  `PermissionPolicy::allow_trusted_local()` and `ToolExposure::fake()` in
  `crates/capo-controller/src/lib.rs:55-74`), call
  `PermissionPolicy::decide(PermissionRequest)` before any tool invocation or
  workspace write proceeds.
- Follow the documented lifecycle from `capability-permissions.md`: append
  `permission.requested`, evaluate, append `permission.decided`, and on allow
  with non-observational persistence create the grant and append
  `capability.grant_created`; the tool/runtime layer proceeds only after the
  decision is recorded.
- A `deny` decision blocks the invocation: no tool runs, no workspace write
  occurs, and the loop surfaces the denial as a typed decide outcome rather than
  silently continuing.
- Map a `deny` for an ACI write tool to a structured, agent-readable refusal the
  loop can reflect on, not a raw error string.
- Record `decision_source`, `persistence`, and `explanation` from
  `PermissionDecision` on every emitted decision event so the audit trail is
  complete even when everything is allowed.
- The `Static` and `TrustedLocal` policies are both reachable through the real
  loop; the fake policy is an explicit test-only variant, not the real default.

Verification:

- Focused `cargo test -p capo-controller` proving an allowed request emits the
  requested/decided/grant-created sequence and a denied request emits no tool
  invocation event.
- `cargo fmt`.
- `git diff --check`.

Must not do:

- Do not let the decide step pass requests straight to the runtime without a
  recorded `permission.decided` event.

Evidence:

- The real loop's decide step is `RealBoundaryController::dispatch_tool_call` ->
  `FakeBoundaryController::dispatch_tool_call`
  (`crates/capo-controller/src/tool_dispatch.rs`), which runs
  `PermissionPolicy::decide` via `ToolExposure::authorize_and_invoke` BEFORE any
  tool invocation. SG1 completed the lifecycle there: it now appends
  `capability.grant_created` (with a durable `CapabilityGrantProjection`)
  immediately after `permission.decided` for any non-observational decision
  (allow OR `reject_always` deny), enriches the `permission.requested` /
  `permission.decided` event payloads with
  `decision_source`/`persistence`/`explanation`/`effect`/`capability_grant_id`,
  and returns a typed `PermissionDecideOutcome` (new `decide` field on
  `ToolDispatchOutcome`) carrying a structured `ToolRefusal` (with
  `agent_message()`) on deny. `PermissionPolicy::allow_trusted_local()` stays the
  real controller default; `Static`/`TrustedLocal` are both reachable; the fake
  policy is test-only.
- Files changed: `crates/capo-controller/src/tool_dispatch.rs` (lifecycle
  wiring + typed decide outcome), `crates/capo-controller/src/lib.rs` (export
  `PermissionDecideOutcome`/`ToolRefusal`), `crates/capo-controller/src/tests.rs`
  (two new SG1 tests
  `sg1_allowed_request_emits_requested_decided_grant_created_sequence`,
  `sg1_denied_request_blocks_invocation_with_structured_refusal`; updated the two
  existing dispatch-sequence assertions + their stale comments to include
  `capability.grant_created`).
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo fmt --check` (clean after `cargo fmt`),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test -p capo-controller` (50 passed, 0 failed, 1 ignored),
  `cargo test --workspace` (all crates ok, 0 failed). `git diff --check` clean.
- Gate fix (2026-05-31): the objective gate failed on
  `sg1_denied_request_blocks_invocation_with_structured_refusal` because the test
  asserted a `reject_once` deny (the `StaticPolicy` rejection, `effect="deny"`,
  `persistence="once"`) materialized a durable deny grant
  (`outcome.decide.grant_created` and an `effect="deny"` grant row). That
  contradicted the implementation AND the design this workpad implements
  (`capability-permissions.md:387`: `reject_once` -> "No grant; record rejection
  for this request"; only `reject_always` creates a durable deny grant). The
  implementation (`decision_creates_grant` in `tool_dispatch.rs`) was already
  correct; the test assertions were wrong. Fixed the test in
  `crates/capo-controller/src/tests.rs` to assert the documented behavior: the
  deny still emits `permission.requested`/`permission.decided` with a structured
  refusal, but creates NO `capability.grant_created` event and NO grant-store row.
  Re-ran the full gate from `/Users/nicolas/devel/capo-wt/safety-gates`:
  `cargo fmt --check` (exit 0), `cargo clippy --all-targets --all-features -- -D
  warnings` (exit 0), `cargo test --workspace` (exit 0; capo-controller 50 passed
  / 0 failed / 1 ignored; 0 failed workspace-wide). `git diff --check` clean.

## SG2 - AgentAdapter Permission Round-Trip And ACP Option Mapping Against Fakes

Status: done.

Acceptance:

- Handle the `AgentAdapter`-trait permission round-trip: a fake/scripted adapter
  raises a permission request, the controller decides it, and the chosen outcome
  is returned to the adapter using the provider-neutral adapter types introduced
  by `real-turn-loop` (not `Fake*`-named structs).
- Implement the ACP option mapping from `capability-permissions.md` as
  fixture/option-mapping logic only: `allow_once` -> allow once/turn-scoped,
  `allow_always` -> allow downscoped to `until_session_end` under TrustedLocal,
  `reject_once`/`reject_always` -> reject with the correct returned `optionId`,
  cancellation -> `cancelled` outcome plus a `permission.decided` with
  `decision = cancel`.
- Persist the ACP option list and chosen option ID as adapter options/response
  on the decision record.
- When no selectable option exists, treat it as adapter error: record
  `permission.decided` with `cancel` and fail the adapter request rather than
  inventing an ACP outcome.
- State the fixture-only verification standard in the task and in `knowledge.md`:
  the live ACP JSON-RPC wire round-trip is explicitly out of scope and lands in
  the depth workpad.

Verification:

- Focused `cargo test -p capo-controller` (and `-p capo-adapters` if the trait
  types move) covering each ACP option-kind mapping against scripted fixtures.
- `cargo fmt`.
- `git diff --check`.

Must not do:

- Do not implement or depend on a live ACP JSON-RPC adapter; this task is
  fixture and option-mapping only.

Evidence:

- Provider-neutral round-trip types added in `capo-adapters` (NOT `Fake*`-named):
  `crates/capo-adapters/src/permission_request.rs` defines
  `AdapterPermissionRequest` (carrying the ACP `PermissionOption[]` --
  `AcpPermissionOption`/`AcpPermissionOptionKind`), `AdapterPermissionResponse`
  (the ACP `AcpPermissionOutcome` `selected{option_id}`/`cancelled` plus the Capo
  decision/grant identity), and the pure `map_acp_options_trusted_local` mapping
  implementing the `capability-permissions.md` table (lines 383-397):
  `allow_once` -> allow `until_turn_end`; `allow_always` alone -> allow
  DOWNSCOPED to `until_session_end` under TrustedLocal; `reject_once` -> transient
  reject (no grant); `reject_always` -> durable `until_revoked` deny grant; no
  options -> adapter-error cancel. The `AgentAdapter` trait gains a
  `scripted_permission_request` raise seam (`crates/capo-adapters/src/adapter.rs`),
  scripted via `ScriptedMockAgent::with_permission_request`
  (`crates/capo-adapters/src/scripted_mock_agent.rs`).
- Controller owns decide + persistence:
  `crates/capo-controller/src/permission_round_trip.rs` adds
  `FakeBoundaryController::decide_adapter_permission` and
  `cancel_adapter_permission` (re-exported on `RealBoundaryController` in
  `crates/capo-controller/src/real_controller.rs`; `PermissionRoundTripScope` /
  `PermissionCancellation` exported from `lib.rs`). It runs
  `PermissionPolicy::decide` over the requested scope (the POLICY is the
  authority: a policy deny over-rules an adapter allow option), applies the ACP
  mapping, persists `permission.requested` -> `permission.decided` with the
  offered `adapter_options` and chosen `adapter_response` on the decision payload,
  and materializes the durable grant on an allow (or durable `reject_always`
  deny). No-selectable-option records `cancel` and flags `adapter_error` (fails
  the adapter request) rather than inventing an outcome.
- Tests: 6 mapping unit tests in `permission_request.rs` (one per option kind +
  no-option + operator-cancel) and 8 controller fixture tests in
  `crates/capo-controller/src/tests.rs` (`sg2_*`): allow_once turn-scoped,
  allow_always downscoped to session-end, reject_once (no grant), reject_always
  (durable deny grant), operator cancellation, no-selectable-option adapter error,
  policy-deny over-rules adapter allow, and a restart/replay test proving the
  round-trip grant rebuilds identically from the event log.
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo fmt --check` (exit 0 after `cargo fmt`),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test -p capo-controller sg2` (8 passed), `cargo test
  --workspace` (all crates ok, 0 failed; capo-controller 58 passed/0 failed/1
  ignored, capo-adapters 36 passed/0 failed/2 ignored). `git diff --check` clean.
  Acceptance met; live ACP JSON-RPC wire stays out of scope (depth).
- Review-fix pass (2026-05-31): applied four confirmed SG2 review findings.
  (1) SAFETY: a policy deny that over-rules an offered allow option no longer
  returns that allow option's ACP `selected{optionId}` (which an ACP adapter
  reads as "proceed"). `resolve_decision` now REWRITES the wire outcome to a
  reject option's id when one was offered, else `cancelled`, and
  `AdapterPermissionResponse` gained a `must_not_proceed` halt flag (true on any
  deny/cancel/adapter-error). The persisted `permission.decided` `adapter_response`
  records the RESOLVED outcome, so it can no longer contradict `decision=reject`.
  (2) ARCH: the round-trip no longer re-implements grant materialization; it now
  builds a canonical `capo_tools::PermissionDecision` and funnels through the SAME
  `decision_creates_grant` durable-deny rule + a SHARED
  `append_capability_grant_created_event` writer the SG1 tool path uses (one
  projection-construction contract, no forked grant model). (3) ARCH: added a
  loop-side step `run_adapter_permission_round_trip` (pull raised request from the
  adapter seam -> decide -> deliver back), so the round-trip is a loop-driven step,
  not a sibling API. (4) TESTS: added the closing leg --
  `AgentAdapter::deliver_permission_response` returning a `PermissionDeliveryAck`
  (proceed iff allowed and not halted) -- with end-to-end tests proving the adapter
  proceeds on allow and halts on a policy deny. Tests now: `cargo test
  -p capo-controller sg2` (12 passed), workspace `cargo fmt --check` /
  `cargo clippy --all-targets --all-features -- -D warnings` / `cargo test
  --workspace` all exit 0 (capo-controller 62 passed/0 failed/1 ignored,
  capo-adapters 37 passed/0 failed/2 ignored).

## SG3 - Grant Read-Back, Revoke/Expire Events, Projection Columns, And Revoke Command

Status: done.

Acceptance:

- Add grant read-back in decide: before authorizing, query the durable grant
  store and treat an existing valid grant as authorization (grants are not
  write-only), and treat a revoked or expired grant as absent.
- Add a `CapabilityGrantRevoked` (and optionally `CapabilityGrantExpired`)
  `EventKind` in `crates/capo-state/src/event.rs`; today only
  `CapabilityGrantCreated`/`CapabilityGrantUsed` exist and the only `*Revoked`
  kind is `ConnectivityExposureRevoked` (event.rs:16-17,70-74).
- Add `created_at`, `expires_at`, and `revoked_at` columns to
  `CapabilityGrantProjection` in `crates/capo-state/src/projections.rs:96-106`
  (these fields exist today only on `ConnectivityExposureProjection`,
  projections.rs:139), and project the new events onto them.
- Add a typed revoke command/flow at the server/controller boundary that emits
  `capability.grant_revoked` with a revocation reason; future grant use of a
  revoked grant is denied while old events remain unchanged.
- Treat expiry as a denial input in decide: a grant past `expires_at` does not
  authorize, even if never explicitly revoked.

Verification:

- Focused `cargo test -p capo-state` for the new event kind and projection
  columns, including a rebuild/replay test that reconstructs revoked/expired
  state identically from the event log.
- Focused `cargo test -p capo-controller` proving revoke then re-request is
  denied and that old grant-created/used events are preserved.
- `cargo fmt`.

Evidence:

- New event kinds: `CapabilityGrantRevoked` (`capability.grant_revoked`) and
  `CapabilityGrantExpired` (`capability.grant_expired`) added to
  `crates/capo-state/src/event.rs` (enum variant, `as_str`, and the `from_wire`
  `ALL` list so they round-trip).
- Projection columns: `created_at`/`expires_at`/`revoked_at` added to
  `CapabilityGrantProjection` (`crates/capo-state/src/projections.rs`), threaded
  through the schema (`schema.rs`: new nullable columns + `add_missing_column`
  back-compat migrations), the apply INSERT/UPDATE (`apply.rs`), the
  `capability_grants` + new `capability_grant_by_id` queries (`queries.rs`), and
  the codec round-trip (`codec.rs`/`codec_encode.rs`: the three timestamps ride
  in the projection `payload_json`, since positional slots a..g are taken, so a
  rebuild reconstructs revoked/expired state identically). Added typed helpers
  `is_active_allow`/`is_revoked`/`is_expired` (expiry compared numerically for
  epoch-millis).
- Grant read-back + revoke flow: new `crates/capo-controller/src/grant_lifecycle.rs`
  adds `FakeBoundaryController::decide_with_grant_read_back` (read-back FIRST: a
  valid allow grant for the scope authorizes; a revoked/expired grant is treated
  as ABSENT and the policy decides), `active_allow_grant_for_scope`, and the typed
  `revoke_capability_grant` (emits `capability.grant_revoked` with a reason and
  re-emits the grant projection with `revoked_at` stamped; old grant-created/used
  events stay unchanged). Re-exported on `RealBoundaryController`
  (`real_controller.rs`) and from `lib.rs` (`GrantReadBackDecision`,
  `GrantReadBackSource`, `GrantRevocation`, `GrantRevocationScope`). The SG1/SG2
  shared grant writer (`tool_dispatch.rs::append_capability_grant_created_event`)
  now stamps `created_at` and derives `expires_at` from `until_time` persistence,
  so every loop-created grant carries lifecycle timestamps.
- Tests: `crates/capo-state/src/tests.rs` adds `sg3_*` (event-kind wire
  round-trip; lifecycle columns persist + rebuild identically;
  revoked-AND-expired state rebuilds identically from the log with old
  created/used events preserved). `crates/capo-controller/src/tests.rs` adds
  `sg3_*` (valid durable grant authorizes via read-back even when the policy
  denies; revoke then re-request is denied with old events preserved + one added
  `capability.grant_revoked` event carrying the reason + replay parity; expired
  grant does not authorize without an explicit revoke). Updated the existing
  grant-projection construction sites for the three new fields
  (capo-state tests, capo-controller `fake_session.rs`, capo-cli
  `permission.rs`/`voice.rs`).
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo fmt --check` (exit 0 after `cargo fmt`),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test -p capo-state sg3` (3 passed), `cargo test
  -p capo-controller sg3` (3 passed), `cargo test --workspace` (exit 0; 0 failed
  workspace-wide; capo-state 50 passed/0 failed, capo-controller 65 passed/0
  failed/1 ignored). `git diff --check` clean. Acceptance met.

## SG4 - Fix TrustedLocal Critical-Scope Exclusion

Status: done.

Acceptance:

- Enumerate the critical scopes in
  `workpads/architecture/capability-permissions.md`: source-write outside the
  workspace (`filesystem:write:path` beyond the workspace root), network egress
  (`network:connect:internet`, `network:expose:public`), secret/credential read
  (`secret:read:credential_material`), and arbitrary shell
  (`shell:execute:path` outside the workspace).
- Change `AllowTrustedLocalProfilePolicy::decide()`
  (`crates/capo-tools/src/permission.rs:87-94`, currently a literal allow-all
  returning `effect = "allow"` with `decision_source =
  "allow_trusted_local_profile"`) so that a request whose scope is critical
  returns `deny` unless an explicit grant for that scope is present.
- Keep the non-critical TrustedLocal audit-only allow behavior intact: ordinary
  workspace read/write, git status/diff, and Capo tool invocation still allow and
  still emit the same durable request/decision/grant records.
- `PermissionPolicy::allow_trusted_local()` remains the controller default
  (`crates/capo-controller/src/lib.rs:56`), but the default is no longer
  blanket-allow on critical scopes.

Verification:

- Focused `cargo test -p capo-tools` asserting
  `AllowTrustedLocalProfilePolicy::decide()` DENIES an un-granted critical-scope
  request (one test per enumerated critical scope) and still ALLOWS a
  non-critical workspace request.
- Focused `cargo test -p capo-tools` asserting that with an explicit grant
  present, the same critical-scope request allows.
- `cargo fmt`.

Evidence:

- Critical-scope enumeration + classifier: `crates/capo-tools/src/permission.rs`
  adds a pure `critical_scope_kind(scope) -> Option<CriticalScope>` classifier
  matching the four enumerated critical scopes from `capability-permissions.md`:
  source-write outside the workspace (`filesystem:write:path`), network egress
  (`network:connect:internet`, `network:expose:public`), secret/credential read
  (`secret:read:credential_material`), and arbitrary shell (`shell:execute:path`).
  Every other scope (workspace read/write, `git:status`/`git:diff`,
  `shell:execute:workspace`, `filesystem:write:workspace`, `secret:read:provider_metadata`,
  Capo tool invocation) classifies as non-critical, so the TrustedLocal audit-only
  allow is unchanged for ordinary local work.
- TrustedLocal fix: `AllowTrustedLocalProfilePolicy` was a unit struct returning a
  literal `effect = "allow"` for every request. It now carries a
  `granted_critical_scopes: Vec<String>` set (empty by default --
  `PermissionPolicy::allow_trusted_local()` is unchanged and grants NO critical
  scope). `decide()` parses the request's scope array and DENIES
  (`effect = "deny"`, `decision_source = "allow_trusted_local_profile"`,
  `persistence = "once"`, deny-keyed `capability_grant_id`, explanation naming the
  scope) when any requested scope is critical and not explicitly granted. A request
  bundling a non-critical scope with an un-granted critical scope is denied as a
  whole (no laundering), and malformed scope json fails closed (deny). When an
  explicit grant is present (new `PermissionPolicy::allow_trusted_local_with_grants(..)`
  constructor re-admitting named critical scopes), the SAME critical-scope request
  allows with the prior `until_session_end` audit-allow shape. `allow_trusted_local()`
  stays the controller default (`crates/capo-controller/src/lib.rs:79`), now no
  longer blanket-allow on critical scopes. (The SG3 controller grant read-back path
  already authorizes a durable-store allow grant BEFORE the policy, so a durable
  explicit grant also re-admits a critical scope through the loop.)
- Tests: `crates/capo-tools/src/tests.rs` adds six SG4 tests --
  `sg4_trusted_local_denies_each_ungranted_critical_scope` (one assertion per
  enumerated critical scope), `sg4_critical_scope_classifier_covers_enumerated_scopes`,
  `sg4_trusted_local_still_allows_non_critical_workspace_request`,
  `sg4_trusted_local_denies_when_critical_mixed_with_non_critical`,
  `sg4_explicit_grant_re_admits_critical_scope` (per-scope: grant allows it, a
  different un-granted critical scope still denies), and
  `sg4_trusted_local_fails_closed_on_malformed_scope_json`.
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-tools sg4` (6 passed), `cargo fmt --check` (exit 0 after
  `cargo fmt`), `cargo clippy --all-targets --all-features -- -D warnings` (exit 0,
  no warnings), `cargo test --workspace` (exit 0; 0 failed workspace-wide;
  capo-tools 113 passed/0 failed/0 ignored). Acceptance met. No live Codex smoke
  required for this task.

## SG5 - Single-Writer Workspace Lock / Session-Scoped Write Lease

Status: done.

Acceptance:

- Add a controller-owned single-writer workspace lock (a session-scoped write
  lease) plus the decide-style gate seam (`gate_workspace_write`) the real
  loop's write path drives. SG5 builds and proves the lock + seam; `goal-autonomy`
  `GO8` is the consumer that wires the gate onto the live write classification
  (SG5 does not itself rewrite `dispatch_tool_call`, and it does not replace the
  server's `WriteSerializer`, which stays the active in-process serializer).
- The lock REJECTS a second concurrent writer rather than interleaving: while a
  session holds the lease, a write request from another session/run is denied
  with a typed conflict outcome.
- Acquire/release is event-sourced so the lock survives restart and rebuilds from
  the event log; a stale lease from a dead holder is reclaimable through the
  liveness-aware recovery path (SG9).
- Read-only tools and reads are not blocked by the write lease.
- This is the primitive `goal-autonomy` `GO8` consumes as its "no conflicting
  workspace lock" continuation precondition; record that contract in
  `knowledge.md`.

Verification:

- Focused `cargo test -p capo-controller` for lock contention: one holder, a
  second writer rejected, holder releases, second writer then succeeds.
- Restart/replay test proving lease state rebuilds from events.
- `cargo fmt`.

Must not do:

- Do not interleave concurrent writers or silently queue them; reject explicitly.

Evidence:

- New event kinds: `WorkspaceLeaseAcquired` (`workspace.lease_acquired`) and
  `WorkspaceLeaseReleased` (`workspace.lease_released`) added to
  `crates/capo-state/src/event.rs` (enum variant, `as_str`, and the `from_wire`
  `ALL` list so they round-trip).
- New event-sourced projection: `WorkspaceLeaseProjection` added to
  `crates/capo-state/src/projections.rs` (one lease row per workspace key, with
  `holder_session_id`/`holder_run_id`/`status`/`acquired_at`/`released_at`/
  `release_reason` and `is_held`/`is_held_by` helpers), threaded through the
  schema (`schema.rs`: `workspace_leases` table + clear-list), apply
  INSERT/UPDATE (`apply.rs`), the `workspace_lease_by_id`/`workspace_leases`
  queries (`queries.rs`), and the codec round-trip
  (`codec.rs`/`codec_encode.rs`), so the lease rebuilds identically from the
  event log via `rebuild_projections`.
- Controller-owned single-writer lock: new
  `crates/capo-controller/src/workspace_lock.rs` adds `WorkspaceLeaseScope`
  (keys the lease on a COLLISION-FREE lower-hex encoding of the lexically
  NORMALIZED workspace root, project-scoped -- not the lossy `slug`, which
  dropped path separators and collapsed distinct roots) and the
  typed `acquire_workspace_write_lease` (read-back FIRST: a held lease owned by
  ANOTHER session is REJECTED with a typed `WorkspaceLockConflict` carrying an
  agent-readable message; the SAME session re-acquire is idempotent with no new
  event; otherwise emit `workspace.lease_acquired`), `release_workspace_write_lease`
  (emit `workspace.lease_released` with a reason; releasing another session's
  lease conflicts), `gate_workspace_write(is_write)` (reads pass through
  un-gated; a write is allowed only for the lease holder, acquiring it if free),
  and `workspace_lease_holder` (read-back for SG9 stale-lease reclaim). Typed
  outcomes `WorkspaceWriteLeaseOutcome` / `WorkspaceWriteGate` are surfaced to
  the loop (decide-style, not errors), re-exported from `lib.rs` and on
  `RealBoundaryController` (`real_controller.rs`). The GO8 "no conflicting
  workspace lock" continuation-precondition contract is recorded in
  `knowledge.md`.
- Tests: `crates/capo-state/src/tests.rs` adds two `sg5_*` tests (event-kind
  wire round-trip; lease projection persists + rebuilds identically across a
  restart for acquire->release). `crates/capo-controller/src/tests.rs` adds eight
  `sg5_*` tests: lock contention (holder acquires, second writer rejected with a
  typed conflict via both `gate_workspace_write` and a direct acquire, holder
  releases, second writer then succeeds), reads-not-blocked (another session's
  read and the holder's own read pass while the write lease is held),
  restart/replay (acquire->release->re-acquire-by-another rebuilds identically
  from the event log and the lock still rejects a stale contender after
  rebuild), idempotent self re-acquire (no new event), and the review-fix
  regressions: same-session acquire->release->re-acquire actually re-holds and
  appends a new event (was a phantom acquire silently deduped by the event
  idempotency layer); cross-session release conflicts and leaves the original
  holder; release of a free lease is a no-op emitting no event; and two distinct
  workspace roots (`/srv/a/b` vs `/srv/ab`) get independent leases while the same
  root spelled with `.`/`..`/trailing-slash shares one lease (collision-free key).
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-state sg5` (2 passed), `cargo test -p capo-controller sg5`
  (4 passed), `cargo fmt --check` (exit 0 after `cargo fmt`),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test --workspace` (exit 0; 0 failed workspace-wide;
  capo-controller 74 passed/0 failed/1 ignored, capo-state 52 passed/0 failed).
  `git diff --check` clean. Acceptance met. No live Codex smoke required for this
  task (SG5 verification is deterministic lock-contention + replay only).

## SG6 - VerificationRunner: Run Check/Lint/Test And Emit Real Pass/Fail Evidence

Status: done.

Acceptance:

- Add a `VerificationRunner` that executes the project's configured check/lint/
  test commands through the existing `capo-runtime` local process runner and
  records the real exit status.
- Emit verification evidence with true pass/fail derived from exit status, never
  from operator assertion or agent-reported claims; the evidence carries the
  command, exit status, and a redacted output artifact ref.
- Consume the typed test/check evidence produced by `tools-aci` (the
  `capo.test_run`/`capo.check` typed result) as an input; the runner owns the
  verification GATE, the ACI tool owns evidence emission.
- A successful run whose output exceeds the runtime cap is NOT classified as
  failed: output is truncated with truncation recorded as metadata and pass/fail
  still keyed off exit status.
- Persist verification evidence as observed evidence (source distinct from
  agent-reported) so SG7 can score against it.
- Decide and record where the runner lives (`capo-eval`, currently a stub at
  `crates/capo-eval/src/lib.rs`, vs `capo-server`); resolve the open question in
  `knowledge.md`.

Verification:

- Focused `cargo test` (target crate per the SG0 decision) with a fake/scripted
  command for both pass and fail, asserting exit-status-derived classification.
- A deterministic over-cap-successful-run test proving a long successful run is
  recorded passed-and-truncated, not failed.
- `cargo fmt`.

Notes:

- Implemented in `crates/capo-controller/src/verification.rs` (the LOOP owner,
  beside SG1-SG5). `FakeBoundaryController::run_verification` executes a
  configured check/lint/test/smoke `VerificationCommand` through the existing
  `capo-runtime` async runner via a NEW synchronous seam,
  `AsyncLocalProcessRunner::run_to_completion`
  (`crates/capo-runtime/src/async_runner.rs`), so the tokio runtime + spawn ->
  drain -> `wait` bridging stay behind the runtime seam; the controller calls one
  sync method and never hand-rolls a runtime. The seam has a nested-reactor guard
  (runs the private current-thread runtime on a dedicated thread when called from
  inside an existing tokio runtime, so it can never panic on a nested
  `block_on`).
- Pass/fail is derived STRICTLY from the real exit status (`exit_code ==
  Some(0)`), never from `--status passed` or an agent claim.
  `verify_from_test_run_record` consumes the typed `capo.test_run`/`capo.check`
  record (`TestRunRecord`) and RE-DERIVES the verdict from its observed
  `exit_status`, ignoring the record's own `claimed_passed` flag (anti-spoofing).
- Evidence is persisted as OBSERVED `evidence.recorded(kind=test/smoke)`: the
  event actor is `capo-controller-verification` and the payload carries
  `source = "observed-runner"`, command, exit status, truncation flag, and the
  redacted output artifact ref, distinct from any agent-reported channel so SG7's
  observed-evidence-only `score_run` can score against it. A successful over-cap
  run is recorded passed-and-truncated, not failed.
- The dead `tokio` dependency added to `crates/capo-controller/Cargo.toml` by the
  scaffolding-only stub commit was removed (and dropped from `Cargo.lock`); the
  controller calls the `capo-runtime` sync seam instead of depending on tokio
  directly.
- Open question resolved in `knowledge.md`: the `VerificationRunner` gate lives in
  `capo-controller`; `capo-eval`/`capo-server` do not produce the verdict.
- Tests (focused): `crates/capo-controller/src/verification.rs` -- scripted
  pass+fail exit-status classification, over-cap-successful-run is
  passed-and-truncated, typed-record scored from exit status not `claimed_passed`,
  and observed-evidence survives a store reopen and re-records idempotently.
  `crates/capo-runtime/src/async_runner.rs` -- the sync seam's pass/fail,
  over-cap-success, and nested-reactor-safe paths.
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-controller verification` (4 passed),
  `cargo test -p capo-runtime run_to_completion` (3 passed), `cargo fmt --check`
  (exit 0 after `cargo fmt`), `cargo clippy --all-targets --all-features --
  -D warnings` (exit 0, no warnings), `cargo test --workspace` (553 passed,
  0 failed). No live Codex smoke required (SG6 verification is deterministic
  scripted-command + replay only).

## SG7 - score_run Over Observed Evidence And Wall-Clock Timing

Status: done.

Acceptance:

- Add `score_run` that compares acceptance criteria to verification evidence and
  produces the run outcome signal.
- `score_run` consumes OBSERVED evidence only (verification exit status, observed
  tool results, runtime events); it never reads agent-reported claims as a score
  input.
- Replace the descriptive-only roll-up: today the only eval artifact is a
  markdown report whose "duration" is an event-sequence delta
  (`crates/capo-eval/src/lib.rs`, `duration_sequence_span`); add real wall-clock
  timing (`started_at`/`completed_at`) to the scored outcome.
- The score is reproducible: rebuilding from the event log yields the same score
  for the same observed evidence.
- Record the score and its inputs as a durable event/projection so the outcome
  is queryable and survives restart.

Verification:

- Focused `cargo test` (target crate per SG0) proving a passing-evidence run and
  a failing-evidence run produce the expected scores, and that injecting only
  agent-reported claims (no observed evidence) does not raise the score.
- Wall-clock timing assertion using a controlled clock or fixture timestamps.
- `cargo fmt`.

Must not do:

- Do not let any agent-reported field contribute to the computed score.

Evidence:

- Placement (SG7 open question resolved in `knowledge.md`): `score_run` lives in
  `capo-controller` (`crates/capo-controller/src/score_run.rs`), beside the SG6
  `VerificationRunner` gate that produces the observed evidence it consumes -- the
  score is the loop's verdict over observed evidence, so the computation belongs
  with the loop owner, not `capo-eval` (descriptive reporting) or `capo-server`
  (transport).
- OBSERVED-evidence-only scoring: `FakeBoundaryController::score_run(scope,
  &[AcceptanceCriterion])` reads back the `evidence.recorded` events for the run
  and keeps ONLY those stamped by the SG6 runner -- actor
  `VERIFICATION_EVIDENCE_ACTOR` (`capo-controller-verification`) AND payload
  `source = "observed-runner"` (`VERIFICATION_EVIDENCE_SOURCE`). Everything else
  (agent-reported summaries/claims, any other actor or `source`) is filtered out
  in `parse_observed_verdict` before it can influence the score, so injecting
  only agent-reported claims never raises it. A typed `AcceptanceCriterion`
  (label + required `VerificationKind`) is MET only when an OBSERVED PASS of its
  kind exists (the gate already re-derived pass/fail from the real exit status,
  never an agent claim); the run `passed` iff every criterion is met, yielding a
  `RunScoreOutcome` of `passed`/`failed`/`inconclusive` (empty criteria set).
- Real wall-clock timing: the scored outcome carries `started_at`/`completed_at`
  (caller-supplied clock millis-since-epoch) and a derived `duration_millis`,
  replacing the `capo-eval` event-sequence-delta "duration".
- Durable + reproducible: a `run.scored` event kind (`RunScored`) and a
  `RunScoreProjection` were added to `capo-state` (enum + `as_str`/`from_wire`
  round-trip; `run_scores` table in `schema.rs` + clear-list; apply
  INSERT/UPDATE in `apply.rs`; `run_score_by_id`/`run_scores_for_session` queries
  in `queries.rs`; codec round-trip in `codec.rs`/`codec_encode.rs` with the
  scalar verdict counts riding in `payload_json` since positional slots a..h are
  taken). The score id is keyed on `(run, stable digest of the scored inputs)`,
  so re-scoring the SAME observed evidence is idempotent (same id, no duplicate
  row) and a rebuild from the event log reconstructs the score identically.
- New public types exported from `capo-controller` lib: `AcceptanceCriterion`,
  `RunScore`, `RunScoreOutcome`, `RunScoreScope`, `ScoredCriterion`.
- Tests (focused, deterministic): `crates/capo-controller/src/score_run.rs` --
  passing observed evidence scores passed; failing observed evidence scores
  failed; agent-reported claims alone do NOT raise the score (and a real observed
  pass added afterward DOES, proving the filter excludes only the claim);
  controlled-clock wall-clock timing (`duration_millis` = completed - started, not
  an event delta); durable + queryable + reproducible across a store reopen +
  `rebuild_projections` + idempotent re-score. `crates/capo-state/src/tests.rs` --
  `sg7_run_scored_event_kind_round_trips`,
  `sg7_run_score_projection_persists_and_rebuilds_identically`.
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-controller score_run` (5 passed),
  `cargo test -p capo-state sg7` (2 passed), `cargo fmt --check` (exit 0 after
  `cargo fmt`), `cargo clippy --all-targets --all-features -- -D warnings`
  (exit 0, no warnings), `cargo test --workspace` (exit 0; 0 failed
  workspace-wide; capo-controller 87 passed/0 failed/1 ignored, capo-state 54
  passed/0 failed). `git diff --check` clean. Acceptance met. No live Codex smoke
  required (SG7 verification is deterministic scripted-evidence + controlled-clock
  + replay only).
- Gate fix (2026-05-31): the objective gate failed on `cargo fmt --check` only
  (`fmt=fail clippy=ok test=ok`). The import block in
  `crates/capo-state/src/queries.rs` had `EventRecord,` and `EvidenceProjection,`
  split across two lines; `cargo fmt` wants them collapsed onto one line:
  `EventRecord, EvidenceProjection, InFlightRun, ...`. Applied that single
  formatting fix (no logic change). Re-ran the full gate from
  `/Users/nicolas/devel/capo-wt/safety-gates`: `cargo fmt --check` (exit 0),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test --workspace` (exit 0; 0 failed workspace-wide, 26
  `test result: ok` suites). Acceptance still met.

## SG8 - Controller-Owned Shadow-Git Checkpoint/Rollback

Status: done.

Acceptance:

- Implement controller-owned checkpoint/rollback as shadow-git, emitting the
  designed `checkpoint.created` / `checkpoint.restored` events and a `Restore`
  command (state-model.md:894-896; the `checkpoints` projection/table is
  designed-only today).
- A checkpoint is created before a real workspace write so any write is
  reversible by one `Restore` command; restore returns the workspace to the
  checkpointed state.
- Upgrade the `real-turn-loop` single-snapshot safety floor: the RTL pre-write
  snapshot (tar/copy/stash) is replaced by per-turn shadow-git checkpoints that
  are restorable per-turn and survive restart.
- Resolve the open question in `knowledge.md`: whether shadow-git is a separate
  `.git` worktree/index or a stash-ring; the chosen mechanism must be restorable
  per-turn and survive a server restart.
- Checkpoint artifacts and restore are recorded as observed evidence/events so a
  rollback is auditable.

Verification:

- Focused `cargo test -p capo-controller` proving create-checkpoint, write,
  restore returns the workspace to the prior state.
- Restart/replay test proving checkpoint refs survive restart and a checkpoint
  taken before restart is still restorable after.
- `cargo fmt`.

Evidence:

- Mechanism (open question RESOLVED in `knowledge.md`): shadow-git is a SEPARATE
  shadow `.git` directory, NOT a stash-ring. Each workspace gets a bare shadow
  repo whose `GIT_DIR` lives under the controller's state root
  (`<shadow_git_root>/<workspace-key>`, keyed by the SAME collision-free lower-hex
  encoding of the lexically-normalized workspace root the SG5 lock uses) and whose
  `GIT_WORK_TREE` is the workspace itself, so the user's own `.git` is NEVER
  touched (proven by `sg8_shadow_git_does_not_touch_workspace_dot_git`). A
  checkpoint is a commit in that shadow repo; the commit SHA is the restorable
  ref. Restorable per-turn (each checkpoint is its own commit, taken with a
  distinct id per turn) and survives restart (the shadow repo + commits are on
  disk and the commit SHA is in the durable `CheckpointProjection` +
  `checkpoint.created` event).
- New event kinds: `CheckpointRestored` (`checkpoint.restored`) added to
  `crates/capo-state/src/event.rs` (enum variant, `as_str`, and the `from_wire`
  `ALL` list so it round-trips); `CheckpointCreated` (`checkpoint.created`)
  already existed (the RTL floor emits it) and is reused. The designed
  `checkpoints` projection/table graduates from design to code here.
- New event-sourced projection: `CheckpointProjection` added to
  `crates/capo-state/src/projections.rs` (the designed `checkpoints` row:
  `checkpoint_id`/`project_id`/`session_id`/`run_id`/`turn_id`/`kind`/`commit_ref`/
  `workspace_root`/`shadow_git_dir`/`content_hash`/`created_at`/`restored_at`,
  with `commit_ref`/`is_restored` helpers), threaded through the schema
  (`schema.rs`: `checkpoints` table + clear-list), apply INSERT/UPDATE
  (`apply.rs`), the `checkpoint_by_id`/`checkpoints_for_run` queries
  (`queries.rs`), and the codec round-trip (`codec.rs`/`codec_encode.rs`:
  positional slots a..h carry project/session/run/turn/kind/commit_ref/
  workspace_root/content_hash; the shadow-git dir + lifecycle timestamps ride in
  `payload_json`), so the checkpoint rebuilds identically from the event log via
  `rebuild_projections`.
- Controller-owned checkpoint/rollback: new
  `crates/capo-controller/src/checkpoint.rs` adds `CheckpointScope`,
  `FakeBoundaryController::create_checkpoint` (init the shadow repo on first use
  via `git init --bare` so `GIT_DIR` resolves the metadata directly; `git add -A`;
  `git commit --allow-empty --no-verify` with `-c commit.gpgsign=false -c
  core.hooksPath=/dev/null` and a pinned `capo-checkpoint` identity so a commit
  never fails on the operator's gpg/hooks/identity config; record the commit SHA
  as the restorable ref on `checkpoint.created` + projection; idempotent
  re-checkpoint keyed on `(run, turn, content tree SHA)`), `restore_checkpoint`
  (the `Restore` command: read the durable checkpoint back by id -- works after a
  restart -- `git checkout --force <sha> -- .` then `git clean -fdx` so files
  added AFTER the checkpoint are removed, leaving the workspace byte-identical to
  the checkpointed state; emit the auditable `checkpoint.restored` event and
  re-emit the projection with `restored_at` stamped), plus `checkpoint` and
  `checkpoints_for_run` read-backs. All re-exported on `RealBoundaryController`
  (`real_controller.rs`) and from `lib.rs` (`CheckpointCreated`,
  `CheckpointError`, `CheckpointRestored`, `CheckpointScope`). This UPGRADES the
  RTL single-snapshot floor (`capo-server::safety_floor::WorkspaceCheckpoint`, a
  directory copy under the artifact root with no projection): the per-turn
  shadow-git checkpoints are restorable per-turn and survive restart, which the
  RTL directory-copy snapshot is not.
- Tests (focused, deterministic, scripted via real on-disk workspaces + system
  git -- no live providers): `crates/capo-controller/src/checkpoint.rs` -- 7 SG8
  tests: create -> write -> restore returns the workspace to the prior state
  (reverting a modified file, restoring a deleted file, removing a file added
  after the checkpoint); per-turn checkpoints independently restorable;
  checkpoint survives restart and is restorable after (reopen store ->
  `rebuild_projections` -> the commit ref/content hash/dirs reconstruct
  identically, and a NEW controller over the rebuilt state still restores the
  pre-restart checkpoint); idempotent re-checkpoint of the same tree (one row, no
  duplicate); create + restore are auditable events with `restored_at` stamped;
  restore of an unknown checkpoint is a typed error not a panic; shadow git never
  touches the workspace's own `.git`. `crates/capo-state/src/tests.rs` -- 2 SG8
  tests: `sg8_checkpoint_event_kinds_round_trip` and
  `sg8_checkpoint_projection_persists_and_rebuilds_identically` (restore updates
  the SAME row in place; the row reconstructs identically after a rebuild).
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-controller checkpoint::` (7 passed),
  `cargo test -p capo-state sg8` (2 passed), `cargo fmt --check` (exit 0 after
  `cargo fmt`), `cargo clippy --all-targets --all-features -- -D warnings`
  (exit 0, no warnings), `cargo test --workspace` (exit 0; 0 failed
  workspace-wide; capo-controller 98 passed/0 failed/1 ignored, capo-state 56
  passed/0 failed). `git diff --check` clean. Acceptance met. No live Codex smoke
  required (SG8 verification is deterministic real-workspace + system-git +
  replay only).
- Review-fix pass (2026-05-31): the original SG8 cut built the controller
  shadow-git mechanism but did NOT wire it into the RTL floor, so two parallel
  checkpoint mechanisms ran (the directory-copy floor stayed effective) and the
  shared `checkpoint.created` kind had two divergent payloads. Fixed by actually
  performing the upgrade (option (a)): `capo-server`'s
  `create_pre_write_checkpoint` now DELEGATES to
  `FakeBoundaryController::create_checkpoint`, and a new
  `CapoServer::restore_pre_write_checkpoint` delegates to `restore_checkpoint`, so
  the floor and the loop share ONE checkpoint mechanism and ONE
  `checkpoint.created` contract (`checkpoint_kind = "shadow_git"` + a
  `CheckpointProjection`). The directory-copy `WorkspaceCheckpoint` (and its
  `copy_dir_recursive`/`clear_dir_contents`/`fnv1a64` helpers) is retired;
  `WorkspaceCheckpoint` is now a shadow-git view (commit SHA + shadow `.git`
  dir + content hash). Added `SqliteStateStore::shadow_git_root()` (=
  `<state_root>/shadow-git`) and a `FakeBoundaryController::shadow_git_root()`
  accessor so the floor keys its shadow repo under the controller state root.
  Updated the RTL6 floor tests
  (`crates/capo-server/src/tests/safety_floor.rs`): the pre-write-checkpoint test
  now asserts the shadow-git mechanism (no `.git` in the workspace, the
  `checkpoint_kind="shadow_git"` payload, restore via
  `restore_pre_write_checkpoint`, and an auditable `checkpoint.restored` event);
  the confinement-rejection test asserts no checkpoint was taken via the workspace
  `.git`. Also extended `sg8_shadow_git_does_not_touch_workspace_dot_git`
  (`crates/capo-controller/src/checkpoint.rs`) to exercise the DESTRUCTIVE restore
  path (`git checkout --force` + `git clean -fdx`) over a work tree containing the
  workspace's own top-level `.git` AND a nested sub-repo `.git`, locking in git's
  top-level `.git` protection as a regression guard and asserting (and documenting)
  that a nested `.git` is removed by `clean -fdx` as untracked content. Re-ran the
  full gate from `/Users/nicolas/devel/capo-wt/safety-gates`: `cargo fmt --check`
  (exit 0), `cargo clippy --all-targets --all-features -- -D warnings` (exit 0),
  `cargo test --workspace` (exit 0).

## SG9 - Liveness-Aware Restart Recovery Replacing exited_unknown

Status: done.

Acceptance:

- Persist `runtime.start_requested` (plus pid/process-group) before spawn so an
  in-flight run is recoverable after restart (state-model.md:786,1140).
- On restart, probe the liveness/health of runs that look live and classify into
  `run.recovered` (still alive, reattached), `run.orphaned`, or `run.exited`
  based on the probe, replacing the blunt path that marks all live-looking runs
  `exited_unknown` (the `exited_unknown` status is referenced in
  `crates/capo-eval/src/lib.rs:177`).
- Reattach to a still-alive run in place when attachable, distinct from
  relaunching a new run with `recovery_of_run_id` (state-model.md:203-207).
- Reclaim a stale single-writer lease (SG5) held by a dead run during recovery.
- Recovery events are idempotent: a repeated restart does not create a second
  recovery event for the same stable observation (idempotency keyed by
  `(run_id, recovery_observation_kind, observed_runtime_state_hash)`,
  state-model.md:1179).

Verification:

- Focused `cargo test -p capo-controller` (and `-p capo-runtime` for liveness
  probing) covering recovered, orphaned, and exited classifications.
- Restart/replay test proving recovery idempotency on repeated restart.
- `cargo fmt`.

Evidence:

- `start_requested` (pid/process-group/boot-id) is ALREADY persisted before the
  run is waited on by the RTL10 in-flight marker
  (`capo-server::dispatch::append_run_started_inflight`: a `run.started` carrying
  `external_pid`/`boot_id`/`runtime_process_ref` +
  `marker = start_requested_inflight`), and `inflight_runs_for_project`
  (`capo-state/src/queries.rs`) reads it back on restart. SG9 consumes that
  durable handle; it did not need a new persist path.
- NON-destructive liveness probe (the `RuntimeRunner.health` probe SG9 needs):
  new `LocalProcessRunner::probe_run_health(external_pid, recorded_boot_id) ->
  RunHealthProbe` in `crates/capo-runtime/src/lib.rs`, with a new
  `RuntimeHealthState` (`Alive`/`Exited`). Unlike `reap_orphan_process_group`
  (which KILLS a live orphan), this only OBSERVES (`kill -0 -<pgid>`) so a live
  run can be reattached in place. It respects the same boot-id guard (a PID under
  a different boot id classifies `Exited`, never trusted as "our run"), and
  carries a stable `observed_state_hash` for idempotency. A non-Unix fallback
  classifies `Exited`.
- Liveness-aware classification + events: new
  `SqliteStateStore::recover_inflight_runs(project, attempt, &[RunRecoveryObservation])`
  (`crates/capo-state/src/lib.rs`) replacing the blunt
  `mark_active_runs_exited_unknown` path. New `RunRecoveryKind`
  (`Reattached`/`Orphaned`/`Exited`) + `RunRecoveryObservation`
  (`crates/capo-state/src/projections.rs`). It emits, per run:
  `Reattached` -> a SINGLE `run.recovered` reattaching in place (NO `run.exited`,
  the process keeps running -- distinct from relaunch with `recovery_of_run_id`,
  which stays `None`); `Orphaned` -> `run.orphaned` then terminal `run.exited`
  then `run.recovered`; `Exited` -> terminal `run.exited` then `run.recovered`.
  No path ever stamps `exited_unknown`. Idempotency key is
  `(run_id, recovery_observation_kind, observed_runtime_state_hash)` (excludes the
  attempt id) via the shared `append_run_recovery_event` writer.
- Controller sweep + stale-lease reclaim:
  `FakeBoundaryController::recover_inflight_runs(attempt)`
  (`crates/capo-controller/src/lib.rs`) probes each in-flight run via
  `LocalProcessRunner::probe_run_health`, classifies (a live run with an
  attachable `runtime_process_ref` -> `Reattached`; live without a handle ->
  `Orphaned`; gone/never-spawned -> `Exited`), records the events, then RECLAIMS
  (SG5) any workspace lease held by a run that did NOT reattach via the new
  `FakeBoundaryController::reclaim_stale_workspace_leases(dead_run_ids, reason)`
  (`crates/capo-controller/src/workspace_lock.rs`, event-sourced
  `workspace.lease_released` with a recovery reason, idempotent). A live
  (reattached) holder's lease is left untouched.
  `recover_command_liveness_aware` brackets the sweep in the same
  `begin_recovery`/`complete_recovery` recovery-attempt frame the RTL10
  `recover_command` uses. All re-exported on `RealBoundaryController`
  (`real_controller.rs`); new public types `RunHealthProbe`/`RuntimeHealthState`
  (capo-runtime) and `RunRecoveryKind`/`RunRecoveryObservation` (capo-state).
- Tests (deterministic, scripted -- no live providers): `capo-runtime` (3) --
  `probe_run_health` reports Alive WITHOUT killing the live group, Exited+stable
  for a dead pid, and Exited (no signal) for a recycled pid across a reboot
  boundary. `capo-controller` (6 `sg9_*`) -- gone run classifies Exited (not
  `exited_unknown`); a still-alive run with a handle REATTACHES in place (only
  `run.recovered`, the live descendant survives the sweep); a still-alive run
  without a handle classifies Orphaned (orphaned->exited->recovered); repeated
  recovery is idempotent (same observation -> no second event + replay parity);
  dead-holder lease reclaim frees the lock for the next writer (and is idempotent
  / leaves a live holder's lease); the full sweep reclaims a gone run's lease.
  `capo-state` (2) -- reattach emits only `run.recovered` and rebuilds
  identically; a gone run exits (never `exited_unknown`).
- Commands run (from `/Users/nicolas/devel/capo-wt/safety-gates`):
  `cargo test -p capo-runtime probe_run_health` (3 passed),
  `cargo test -p capo-controller sg9` (6 passed),
  `cargo test -p capo-state recover_inflight_runs` (2 passed),
  `cargo fmt --check` (exit 0 after `cargo fmt`),
  `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, no
  warnings), `cargo test --workspace` (exit 0; 0 failed workspace-wide;
  capo-controller 104 passed/0 failed/1 ignored, capo-runtime 44 passed/0 failed,
  capo-state 58 passed/0 failed). `git diff --check` clean. Acceptance met. No
  live Codex smoke required (SG9 verification is deterministic liveness-probe +
  classification + lease-reclaim + replay only).

## SG10 - Deterministic Safety Test Suite Plus Restart/Replay

Status: pending.

Acceptance:

- Add a deterministic suite, with no live providers, covering: denied request,
  granted request, revoked grant denied on re-request, expired grant denied,
  critical-scope denial under TrustedLocal, verification pass, verification fail,
  workspace-lock contention, and checkpoint rollback restoring prior state.
- Add restart/replay coverage proving grant lifecycle (created/revoked/expired),
  lock leases, checkpoint refs, score outcomes, and recovery classifications all
  rebuild identically from the event log.
- Every state-changing safety behavior in SG1-SG9 has at least one deterministic
  assertion (event/wire snapshot, exit status, or replay), honoring the SG0
  invariant.
- Tests use fakes/scripted adapters and fake/scripted verification commands so
  the suite is hermetic and reproducible.

Verification:

- `cargo fmt`.
- Focused `cargo test -p capo-state -p capo-tools -p capo-controller -p capo-eval`
  for the changed safety behaviors.
- `git diff --check`.

## SG11 - Live Opt-In Safety Smoke Paired With Deterministic Assertions And E2E Gate

Status: pending.

Acceptance:

- Add a live opt-in safety smoke behind an explicit env gate (mirroring the
  existing `CAPO_SERVER_RUN_CODEX_LIVE` opt-in), separate from ordinary test
  runs, that drives one real gated write: permission decided, checkpoint taken,
  write performed under the workspace lock, verification run, and `score_run`
  computed.
- The smoke strips secrets from any captured output and persists only redacted
  artifacts (`RedactionState::Safe`/`Redacted`).
- The smoke is PAIRED with deterministic assertions: the same path is also
  proven by event/wire snapshots and exit-status checks so completion is never
  solely operator-attested.
- Add an E2E gate that runs the deterministic safety suite (SG10) and reviews
  architecture fit, the safety boundary, test adequacy, and whether to proceed to
  `goal-autonomy`.
- Record review notes covering enforcement correctness, verification honesty,
  rollback reliability, and recovery safety.

Verification:

- `cargo fmt`.
- Focused `cargo test` for changed crates, widening to `cargo test` if shared
  controller/state behavior changed broadly.
- Manual live smoke transcript with secrets stripped, paired with the
  deterministic assertions above.
- `git diff --check`.
