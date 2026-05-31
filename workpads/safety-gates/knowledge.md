# Safety Gates Knowledge

## Objective

Capture the decisions that turn Capo's built-but-inert safety machinery into
live enforcement. The scope engine, filesystem path containment, durable grant
store, and event-sourced state already exist; what is missing is that nothing in
the live loop calls them. This workpad wires `PermissionPolicy`/`ToolExposure`
into the real turn loop's decide step, makes grants authorize (not just record),
adds revoke/expiry, fixes the `TrustedLocal` critical-scope hole, adds a real
`VerificationRunner` plus a `score_run` computed from observed evidence only,
adds a single-writer workspace lock, and adds controller-owned shadow-git
checkpoint/rollback with liveness-aware restart recovery. The goal is a loop
that is safe enough to run unattended-capable: confined, reversible, audited,
verified on real exit status, and recoverable across restart.

## Scope Decision

This is its own workpad: `safety-gates` (prefix `SG`, Phase 4 - Make it safe).
It is not folded into `real-turn-loop`, `tools-aci`, or `goal-autonomy`.

It depends on `real-turn-loop`, `streaming-transport`, and `tools-aci`:

- `real-turn-loop` ships the `RealBoundaryController` observe -> decide -> emit
  loop that replaces `FakeBoundaryController`, the provider-neutral
  `AgentAdapter` trait the permission round-trip rides, and a minimal safety
  FLOOR (path confinement, hard-kill, single-snapshot pre-write checkpoint,
  resource ceiling, dry-run default, orphan reaping). Safety-gates is where the
  FLOOR becomes full ENFORCEMENT: the floor's single snapshot is upgraded to
  per-turn shadow-git, and orphan reaping is upgraded to liveness-aware
  recovery.
- `streaming-transport` carries the inline permission cards and verification
  progress over the broadcast/SSE event tail and applies redaction-on-emit, so
  the operator can see and answer a permission decision and watch a verification
  run live. Clients render those frames; they never own enforcement state.
- `tools-aci` defines and instruments the tools that get gated and emits the
  typed test/check evidence the `VerificationRunner` consumes. ACI owns evidence
  EMISSION; safety-gates owns the verification GATE and `score_run`.

This workpad converts BUILT-BUT-INERT safety machinery into enforcement. The
daily-driver review scored permissions 2.0/5 with the one-line "Real scope
engine + path containment + durable grant store with ACP mapping; but not wired
into the loop, grants write-only, no revoke/sandbox." The hard part is built; it
is simply never called by the live path, and grants are write-only (created but
never read back to authorize). This workpad makes the existing engine enforce.

It is internally sub-phased so the dependency surface stays narrow per milestone:

- enforcement: SG1, SG2, SG3, SG4, SG5 - wire decide into the loop, the
  AgentAdapter permission round-trip + ACP option mapping against fakes, grant
  read-back + revoke/expiry events + projection columns, the TrustedLocal
  critical-scope fix, and the single-writer workspace lock.
- verification: SG6, SG7 - the `VerificationRunner` on real exit status and
  `score_run` over observed evidence with wall-clock timing.
- checkpoint-recovery: SG8, SG9 - controller-owned shadow-git checkpoint/rollback
  and liveness-aware restart recovery.
- closeout: SG10 (deterministic safety suite + restart/replay) and SG11 (live
  opt-in safety smoke paired with deterministic assertions + E2E gate) span all
  three sub-phases.

This workpad IMPLEMENTS `workpads/architecture/capability-permissions.md`
(lifecycle steps 1-8, the ACP option-mapping table, and the critical-scope
exclusion rule); it does not redesign it. The designed events
`capability.grant_revoked` and `capability.grant_expired`, and the
`created_at`/`expires_at`/`revoked_at` grant projection columns, graduate from
design to code here. The designed `checkpoints` table and
`checkpoint.created`/`checkpoint.restored` events (state-model.md) also graduate
here.

The safety boundary: the server/controller owns enforcement, grant lifecycle,
the verification gate, the workspace lock, checkpoint/rollback, and recovery.
ACP `request_permission` is an adapter round-trip below the `AgentAdapter`
boundary. Clients only render permission cards and verification progress; they
never own enforcement state.

Acceptance+verification invariant (also stated in `tasks.md` SG0): no task in
this workpad completes on operator self-attestation alone; every manual smoke is
paired with a deterministic assertion (event/wire snapshot, exit status, or
replay). Live-provider work stays behind explicit opt-in env gates mirroring
`CAPO_SERVER_RUN_CODEX_LIVE` / `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`.

## Wire Enforcement Into The Live Decide Step (SG1-SG2)

The decide step is where enforcement must live. `real-turn-loop` introduces the
`RealBoundaryController`, replacing the inert path where `FakeBoundaryController`
holds `PermissionPolicy::allow_trusted_local()` and `ToolExposure::fake()`
(`crates/capo-controller/src/lib.rs:55-74`). Before any tool invocation or
workspace write proceeds, the loop calls
`PermissionPolicy::decide(PermissionRequest)` and follows the documented
lifecycle from `capability-permissions.md`: append `permission.requested`,
evaluate, append `permission.decided`, and on an allow with non-observational
persistence create the grant and append `capability.grant_created`. The
tool/runtime layer proceeds only after the decision is recorded.

A `deny` blocks the invocation: no tool runs, no workspace write occurs, and the
loop surfaces the denial as a typed decide outcome rather than silently
continuing. A `deny` for an ACI write tool maps to a structured, agent-readable
refusal the loop can reflect on, not a raw error string. `decision_source`,
`persistence`, and `explanation` are recorded on every decision event so the
audit trail is complete even when everything is allowed. The `Static` and
`TrustedLocal` policies are both reachable through the real loop; the fake
policy is an explicit test-only variant.

The `AgentAdapter` permission round-trip is fixture/option-mapping only. A
fake/scripted adapter raises a permission request, the controller decides it,
and the chosen outcome returns to the adapter using the provider-neutral adapter
types (not `Fake*`-named structs). The ACP option mapping from
`capability-permissions.md` is implemented as option-mapping logic: `allow_once`
-> allow once/turn-scoped; `allow_always` -> allow downscoped to
`until_session_end` under TrustedLocal; `reject_once`/`reject_always` -> reject
with the correct returned `optionId`; cancellation -> `cancelled` outcome plus a
`permission.decided` with `decision = cancel`. The ACP option list and chosen
option ID are persisted as adapter options/response on the decision record. When
no selectable option exists, that is an adapter error: record `permission.decided`
with `cancel` and fail the adapter request rather than inventing an ACP outcome.
The live ACP JSON-RPC wire round-trip is explicitly out of scope; it lands in
the depth workpad.

Fixture-only verification standard (SG2): SG2 is verified ENTIRELY against
scripted/fake adapters and asserted option mappings -- never the live ACP
JSON-RPC wire. The provider-neutral types live in `capo-adapters`
(`AdapterPermissionRequest` carrying the ACP `PermissionOption[]`,
`AdapterPermissionResponse` carrying the chosen `optionId` / `cancelled`
outcome, and the pure `map_acp_options_trusted_local` mapping function); the
`AgentAdapter` trait gains a `scripted_permission_request` raise seam that
`ScriptedMockAgent::with_permission_request` scripts. The controller owns the
decide + persistence (`FakeBoundaryController::decide_adapter_permission` /
`cancel_adapter_permission`, re-exported on `RealBoundaryController`): it runs
`PermissionPolicy::decide` over the requested scope (the policy is the authority
-- a policy deny over-rules an adapter allow option), applies the ACP mapping,
persists `permission.requested` -> `permission.decided` with the offered
`adapter_options` and the chosen `adapter_response` on the decision payload, and
materializes the durable grant on an allow (or a durable `reject_always` deny).
Each ACP option kind is covered by a deterministic controller test plus a replay
test proving the round-trip grant rebuilds identically from the event log. The
live wire round-trip stays in depth.

SG2 review-fix invariants (2026-05-31): the central safety semantic is that a
policy deny ALWAYS halts the adapter, even when an allow option was offered. The
ACP wire outcome returned to (and persisted for) the adapter must therefore never
be the allow option's `selected{optionId}` on a policy over-rule -- an ACP adapter
consuming `response.outcome` reads `selected` as "permitted, proceed". The
controller rewrites the over-rule outcome to a reject option's id (when offered)
else `cancelled`, and `AdapterPermissionResponse` carries an explicit
`must_not_proceed` halt flag that is the single safe signal an adapter consumes
(`may_proceed()` = allowed AND not halted). The round-trip's grant materialization
and durable-deny rule are NOT a second copy: it builds a canonical
`PermissionDecision` and funnels through the SAME `decision_creates_grant` +
`append_capability_grant_created_event` machinery the SG1 tool dispatch path owns,
so SG3 grant read-back reads ONE grant model. The round-trip is also a loop-driven
step (`run_adapter_permission_round_trip`: pull the raised request from the
`AgentAdapter` seam -> decide -> deliver the response back through
`AgentAdapter::deliver_permission_response`, capturing a `PermissionDeliveryAck`),
not a sibling API invoked beside the loop; the depth ACP adapter reuses this same
hook with a real adapter behind the seam.

## Grant Lifecycle: Read-Back, Revoke, Expiry (SG3)

Grants must authorize, not just record. Today the grant store is write-only: the
durable SQLite grant store exists, but the live path never reads a grant back to
authorize a later request. The decide step gains grant read-back: before
authorizing, it queries the durable grant store and treats an existing valid
grant as authorization, and treats a revoked or expired grant as absent.

This requires real schema work, not just a policy tweak. A
`CapabilityGrantRevoked` (and optionally `CapabilityGrantExpired`) `EventKind` is
added in `crates/capo-state/src/event.rs`; today only
`CapabilityGrantCreated`/`CapabilityGrantUsed` exist, and the only `*Revoked`
kind is `ConnectivityExposureRevoked` (event.rs:16-17,70-74). `created_at`,
`expires_at`, and `revoked_at` columns are added to
`CapabilityGrantProjection` in `crates/capo-state/src/projections.rs:96-106`
(these fields live today only on `ConnectivityExposureProjection`,
projections.rs:139), and the new events project onto them. A typed revoke
command/flow at the server/controller boundary emits `capability.grant_revoked`
with a revocation reason; future use of a revoked grant is denied while old
grant-created/used events remain unchanged. Expiry is a denial input in decide: a
grant past `expires_at` does not authorize even if never explicitly revoked. A
rebuild/replay test must reconstruct revoked/expired state identically from the
event log.

## TrustedLocal Critical-Scope Fix (SG4)

`AllowTrustedLocalProfilePolicy::decide()`
(`crates/capo-tools/src/permission.rs:87-94`) is currently a literal allow-all
that returns `effect = "allow"` with `decision_source =
"allow_trusted_local_profile"` for every request. This is the blanket-allow hole
the review flagged. The fix enumerates the critical scopes in
`capability-permissions.md` and requires an explicit grant for each even under
TrustedLocal:

- source-write outside the workspace (`filesystem:write:path` beyond the
  workspace root),
- network egress (`network:connect:internet`, `network:expose:public`),
- secret/credential read (`secret:read:credential_material`),
- arbitrary shell (`shell:execute:path` outside the workspace).

`decide()` returns `deny` for a critical-scope request unless an explicit grant
for that scope is present. Non-critical TrustedLocal audit-only allow behavior
stays intact: ordinary workspace read/write, git status/diff, and Capo tool
invocation still allow and still emit the same durable request/decision/grant
records. `PermissionPolicy::allow_trusted_local()` remains the controller default
(`crates/capo-controller/src/lib.rs:56`), but it is no longer blanket-allow on
critical scopes. The grant store's `effect = "deny"` rows (from `reject_always`)
also participate: a deny grant blocks even non-critical scopes.

## Single-Writer Workspace Lock (SG5)

A controller-owned single-writer workspace lock (a session-scoped write lease) +
its decide-style gate seam (`gate_workspace_write`). It REJECTS a
second concurrent writer rather than interleaving: while a session holds the
lease, a write request from another session/run is denied with a typed conflict
outcome. Acquire/release is event-sourced so the lock survives restart and
rebuilds from the event log; a stale lease from a dead holder is reclaimable
through the liveness-aware recovery path (SG9). Read-only tools and reads are not
blocked. This is necessary because `streaming-transport` delivers a multi-client
broadcast surface and `tools-aci` delivers `file_write`/edit/patch, so two
clients or a client plus a continuation can drive concurrent writes; without the
lock those interleave silently.

SCOPE (corrected after review): SG5 builds the lock primitive and its gate seam
and proves both with contention/replay/regression tests. SG5 does NOT itself
rewrite `dispatch_tool_call` to call `gate_workspace_write` on every write tool,
and it does NOT replace the server's process-global `WriteSerializer`
(`capo-server::transport`), which remains the ACTIVE in-process write serializer
that today defends the multi-client concurrent-writer scenario above. The
session-scoped lease is the finer-grained primitive the `WriteSerializer`
placeholder anticipated and that `goal-autonomy` `GO8` drives from the live
loop's write classification; the actual loop/transport wiring lands with that
consumer, not in SG5.

Lease key (corrected after review): the lease is keyed on a COLLISION-FREE
lower-hex encoding of the LEXICALLY-NORMALIZED workspace root (`.`/`..`/`//`/
trailing-separator resolved), not the human-readable `slug` (which dropped path
separators and collapsed distinct roots like `/srv/a/b` and `/srv/ab` to one
key). The same root spelled differently keys one lease; distinct roots never
collide. Normalization is lexical, not `fs::canonicalize` (no symlink
resolution, no on-disk existence required).

Concurrency caveat: acquire is a read-then-write across two connections (no
`BEGIN IMMEDIATE`, no DB uniqueness on `status='held'`), so the single-writer
guarantee relies on the transport serializing writers in-process; it is not a
hard cross-process mutex until SG9's liveness-aware reclaim lands.

Contract for `goal-autonomy`: this is the primitive `GO8` consumes as its "no
conflicting workspace lock" continuation precondition. `GO8` names the lock but
never builds it; `safety-gates` builds it.

## Verification On Real Exit Status And score_run (SG6-SG7)

Verification must be computed from real exit status, never operator-asserted.
Today the daily-driver gate keys off `--status passed` taken on faith, and the
only eval artifact is a descriptive markdown roll-up whose "duration" is an
event-sequence delta (`crates/capo-eval/src/lib.rs`, `duration_sequence_span`).

A `VerificationRunner` executes the project's configured check/lint/test commands
through the existing `capo-runtime` local process runner and records the real
exit status. It emits verification evidence with true pass/fail derived from
exit status; the evidence carries the command, exit status, and a redacted output
artifact ref. It consumes the typed test/check evidence `tools-aci` produces (the
`capo.test_run`/`capo.check` typed result) as an input; the runner owns the
verification gate, the ACI tool owns evidence emission. A successful run whose
output exceeds the runtime cap is NOT classified as failed: output is truncated
with truncation recorded as metadata and pass/fail still keyed off exit status.
This matters because `capped_output` (capo-runtime/src/lib.rs:1351) returns
`Err(OutputLimitExceeded)` today and the run discards artifacts on overflow, so a
long successful run is currently misclassified as an error. Verification evidence
is persisted as observed evidence (source distinct from agent-reported).

`score_run` compares acceptance criteria to verification evidence and produces
the run outcome signal. It consumes OBSERVED evidence only (verification exit
status, observed tool results, runtime events); it never reads agent-reported
claims as a score input. It adds real wall-clock timing (`started_at`/
`completed_at`) to the scored outcome, replacing the event-sequence-delta
"duration." The score is reproducible: rebuilding from the event log yields the
same score for the same observed evidence, and the score plus its inputs are
recorded as a durable event/projection so the outcome is queryable and survives
restart. Injecting only agent-reported claims (no observed evidence) must not
raise the score.

## Controller-Owned Shadow-Git Checkpoint/Rollback (SG8)

Checkpoint/rollback is controller-owned shadow-git and is the prerequisite for
unattended source-writing. It emits the designed `checkpoint.created` /
`checkpoint.restored` events and a `Restore` command (state-model.md:894-896,
1042; the `checkpoints` projection/table is designed-only today). A checkpoint is
created before a real workspace write so any write is reversible by one `Restore`
command; restore returns the workspace to the checkpointed state. This upgrades
the `real-turn-loop` single-snapshot safety floor: the RTL pre-write snapshot
(tar/copy/stash) is replaced by per-turn shadow-git checkpoints that are
restorable per-turn and survive restart. Checkpoint artifacts and restore are
recorded as observed evidence/events so a rollback is auditable.

## Liveness-Aware Restart Recovery (SG9)

Recovery is liveness-aware, replacing the blunt path that marks all live-looking
runs `exited_unknown`. Today the only recovery is
`mark_active_runs_exited_unknown` (`crates/capo-state/src/lib.rs:251-291`), which
labels every active-looking run `exited_unknown` and orphans children; the
`exited_unknown` status is still referenced in
`crates/capo-eval/src/lib.rs:177`. `real-turn-loop` already added orphan reaping
the moment it spawns real processes; this task finishes the job.

`runtime.start_requested` (plus pid/process-group) is persisted before spawn so
an in-flight run is recoverable after restart (state-model.md:786,1140). On
restart, the controller probes the liveness/health of runs that look live and
classifies into `run.recovered` (still alive, reattached), `run.orphaned`, or
`run.exited` based on the probe. It reattaches to a still-alive run in place when
attachable, distinct from relaunching a new run with `recovery_of_run_id`
(state-model.md:203-207). It reclaims a stale single-writer lease (SG5) held by a
dead run during recovery. Recovery events are idempotent: a repeated restart does
not create a second recovery event for the same stable observation (idempotency
keyed by `(run_id, recovery_observation_kind, observed_runtime_state_hash)`,
state-model.md:1179).

## Non-Goals

- Do not add autonomous continuation or completion audit; the continuation
  scheduler and evidence-gated completion auditor are `goal-autonomy`.
- Do not implement OS-level sandbox tiers (seatbelt/landlock/bwrap) or git
  worktree isolation; those land in the depth workpad.
- Do not build the live ACP JSON-RPC wire adapter; `request_permission` here is
  fixture/option-mapping only against fake/scripted adapters, and the live wire
  round-trip lands in depth.
- No web client work; clients only render permission cards and verification
  progress over the stream and never own enforcement state.
- Do not let any agent-reported field contribute to the computed `score_run`.
- Do not interleave or silently queue concurrent writers; reject explicitly.

## Open Questions

- Is shadow-git a separate `.git` worktree/index or a stash-ring? Either way the
  chosen mechanism must be restorable per-turn and survive a server restart;
  resolve in SG8.
- Does `score_run` live in `capo-eval` (currently a stub at
  `crates/capo-eval/src/lib.rs`) or in `capo-server`? RESOLVED for the
  `VerificationRunner` half (SG6): the verification GATE lives in
  `capo-controller` (`crates/capo-controller/src/verification.rs`), beside the
  other safety gates SG1-SG5, because the LOOP owner is what decides whether a
  run passed and so must own the gate that derives that verdict from real exit
  status. The tokio runtime + spawn/drain/wait bridging stay BEHIND the
  `capo-runtime` seam (`AsyncLocalProcessRunner::run_to_completion`), so the
  controller calls one synchronous method and process execution does not leak
  into the loop owner. `capo-eval` is the descriptive reporting layer and
  `capo-server` is transport; neither produces the verdict. `score_run` (SG7)
  CONSUMES the observed `evidence.recorded(kind=test/smoke)` this gate persists;
  its own placement is resolved in SG7.
- Should `CapabilityGrantExpired` be a distinct materialized event, or should
  expiry be evaluated purely from `expires_at` at decide time with no event? The
  SG3 acceptance allows it as optional; the replay test must reconstruct expired
  state identically either way.
- For reattach (SG9), what is the minimum attachable handle Capo can hold across
  restart for a live provider process given the runtime is moving to tokio in
  `streaming-transport`, and which runs are reattachable versus only
  observable-as-orphaned?
