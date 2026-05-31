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

Status: pending.

Acceptance:

- Add a controller-owned single-writer workspace lock (a session-scoped write
  lease) that gates all tool writes and workspace mutations in the real loop.
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

## SG6 - VerificationRunner: Run Check/Lint/Test And Emit Real Pass/Fail Evidence

Status: pending.

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

## SG7 - score_run Over Observed Evidence And Wall-Clock Timing

Status: pending.

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

## SG8 - Controller-Owned Shadow-Git Checkpoint/Rollback

Status: pending.

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

## SG9 - Liveness-Aware Restart Recovery Replacing exited_unknown

Status: pending.

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
