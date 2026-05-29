# Safety Gates References

## Objective

Record the local and external sources that shape the safety-gates workpad: the
architecture designs this workpad implements, the inert implementation it makes
enforce, and the external practice it borrows from. Dated claims reflect the
observed state on 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/capability-permissions.md`
  - Key facts: the A3 capability/permission design this workpad IMPLEMENTS
    (it does not redesign it). `PermissionPolicy` is the behavior module;
    `CapabilityProfile` is data; grants are durable, scoped, expirable, and
    revocable, and a grant is not proof of enforcement. The permission lifecycle
    is the 8 steps SG1 follows (requested -> evaluate -> decided -> grant_created
    on non-observational allow -> proceed -> grant_used -> expired/revoked). The
    ACP option-mapping table (`allow_once`/`allow_always`/`reject_once`/
    `reject_always` -> Capo decision/persistence/grant) is the SG2 mapping;
    `allow_always` downscopes to `until_session_end` under TrustedLocal. The
    design already states that critical scopes (public network exposure,
    credential-material reads/writes, raw voice transcript reads) are excluded
    unless the selected profile explicitly includes them - the SG4 fix. Designs
    the `capability.grant_revoked` / `capability.grant_expired` events and the
    `capability_grants(... expires_at, revoked_at, revocation_reason ...)` table
    columns that SG3 graduates to code.
- `workpads/architecture/state-model.md`
  - Key facts: SQLite events are the source of operational truth; read models
    rebuild from events plus artifacts. Designs the `checkpoint.created`
    (state-model.md:894) and `checkpoint.restored` (895) events and the
    `checkpoints(checkpoint_id, project_id, session_id, run_id, kind, artifact_id,
    created_at, restored_at)` table (1042) that SG8 graduates to code. Designs
    `runtime.start_requested` persisted before launch (786, 1140) and the
    distinction between in-place reattach of a still-alive run and a new run with
    `recovery_of_run_id` (203-207) that SG9 implements. Recovery event idempotency
    is keyed by `(run_id, recovery_observation_kind, observed_runtime_state_hash)`
    and excludes `recovery_attempt_id` (1179).
- `workpads/harness-research/daily-driver-review.md`
  - Key facts: the lead synthesis that motivates this workpad. Permission policy
    scored 2.0/5: "Real scope engine + path containment + durable grant store
    with ACP mapping; but not wired into the loop, grants write-only, no
    revoke/sandbox" - the inert-machinery problem SG1-SG4 fix. Evaluation scored
    1.5/5: "all verification is operator-asserted; no timing/OTel" - the SG6/SG7
    problem. State/recovery scored 2.5/5 but flagged "no workspace
    checkpoint/rollback (no shadow-git, designed-only), blunt recovery (marks all
    live-looking runs exited_unknown)" - the SG8/SG9 problem. Confirms
    `AllowTrustedLocalProfilePolicy.decide` returns blanket allow
    (permission.rs:87-94) and that the live controller never consults the policy.

## Local Product And Implementation Sources

- `crates/capo-tools/src/permission.rs`
  - Key facts: the real scope/decision engine. `PermissionPolicy` enum has
    `Fake`/`TrustedLocal`/`Static` variants with a `decide(PermissionRequest)
    -> PermissionDecision` method (lines 4-51). `AllowTrustedLocalProfilePolicy::
    decide()` (87-99) is a literal allow-all returning `effect = "allow"`,
    `decision_source = "allow_trusted_local_profile"`, `persistence =
    "until_session_end"` for every request - the blanket-allow hole SG4 fixes.
    `StaticPolicy` carries an `allowed_scopes` list and is the reachable
    stricter policy. The scope engine and path containment
    (`runtime_wrapper_paths.rs`) already exist; SG4 adds critical-scope deny and
    grant-conditioned allow without rewriting the engine.
- `crates/capo-cli/src/permission.rs`
  - Key facts: the existing permission-queue CLI surface
    (`request_permission_approval`, `list_permission_approvals`). It queues a
    `PermissionApprovalProjection` and appends `PermissionApprovalQueued`, with a
    default `capability_profile_id = "trusted-local-dev"`. It is the
    write-side/queue path; it does not run the decide loop. SG3's revoke command
    is a sibling typed flow at the server/controller boundary; the live decide
    enforcement (SG1) lives in the controller, not here.
- `crates/capo-state/src/event.rs`
  - Key facts: `EventKind` enum (lines 4-54). Only `CapabilityGrantCreated`
    (16) and `CapabilityGrantUsed` (17) exist for grants; the only `*Revoked`
    kind is `ConnectivityExposureRevoked` (20). SG3 adds `CapabilityGrantRevoked`
    (and optionally `CapabilityGrantExpired`). `RedactionState` (112-133) has
    `Safe`/`Redacted`/`Unknown`/`ContainsSensitive` and `is_persistable_artifact()`
    (Safe/Redacted only) - the redaction guard SG6/SG11 use for output artifacts.
- `crates/capo-state/src/projections.rs`
  - Key facts: `CapabilityGrantProjection` (96-106) has no `created_at`,
    `expires_at`, or `revoked_at` columns today; those fields exist only on
    `ConnectivityExposureProjection` (`revoked_at` at 139). SG3 adds them to the
    grant projection and projects the new revoke/expire events onto them.
- `crates/capo-state/src/lib.rs`
  - Key facts: `mark_active_runs_exited_unknown` (251-291) is the blunt recovery
    path - it labels every active-looking run `exited_unknown` and orphans
    children, with no liveness probe. SG9 replaces it with probe-based
    `run.recovered`/`run.orphaned`/`run.exited` classification and in-place
    reattach. The event-sourced append/projection machinery here is what SG3
    (grant lifecycle), SG5 (lease), and SG8 (checkpoint refs) rebuild from.
- `crates/capo-eval/src/lib.rs`
  - Key facts: the eval stub. The only artifact is a descriptive markdown
    roll-up (`render_task_outcome_report`) whose duration is
    `duration_sequence_span` - an event-sequence delta, not wall-clock. Still
    references the `exited_unknown` status (177). SG6 (`VerificationRunner`) and
    SG7 (`score_run` with real `started_at`/`completed_at`) decide whether to
    land here or in `capo-server`; the open question is recorded in `knowledge.md`.
- `crates/capo-runtime/src/lib.rs`
  - Key facts: the real local process runner the `VerificationRunner` runs
    commands through. `RuntimeRunner` enum has `LocalProcess`/`RemoteProcess`/
    `Fake` (49-63); `LocalProcessRunner` (204) sets `process_group(0)` (341) and
    `terminate_process_group` (594) - the proven descendant reaper. `capped_output`
    (1351) returns `Err(OutputLimitExceeded)` (33, 1353) and the run discards
    artifacts on overflow (test at 1611/1665), so a long SUCCESSFUL run is
    misclassified as an error - the over-cap classification bug SG6 must avoid by
    truncating-and-recording rather than failing. `RemoteProcessRunner` (813) is a
    loopback stub.

## External Sources

- https://github.com/openai/codex (execpolicy / sandboxing)
  - Observed 2026-05-29.
  - Key facts: Codex enforces real OS-level isolation (seatbelt on macOS,
    landlock+seccomp on Linux) plus an `execpolicy` layer that gates commands.
    Confirms the daily-driver review's point that incumbents enforce isolation
    while Capo has path-prefix checks only. Capo's SG4 critical-scope deny is the
    policy-layer analogue; the OS sandbox tier itself is deferred to the depth
    workpad, not built here.
- Cline checkpoint/rollback (shadow-git)
  - Observed 2026-05-29.
  - Key facts: Cline (and Cursor) ship per-action workspace checkpoint/rollback
    backed by a shadow/auxiliary git so the user can revert agent edits. This is
    the prior art for SG8's controller-owned shadow-git, and the basis for the
    open question of separate-worktree versus stash-ring; the requirement Capo
    adds is that checkpoints are event-sourced, restorable per-turn, and survive
    a server restart.
- SWE-bench evaluation discipline
  - Observed 2026-05-29.
  - Key facts: SWE-bench scores a patch by running the repository's real test
    suite and keying success off actual pass/fail, not model self-report. This is
    the discipline SG6/SG7 adopt: verification is computed from real exit status
    and `score_run` consumes observed evidence only, never agent-reported claims.

Older external context (ACP `session/request_permission` semantics, Codex Goals,
provider safety models, and the broader peer landscape) is inherited via
`workpads/harness-research/references.md` and `workpads/harness-research/
knowledge.md` rather than re-cited here. The ACP option-mapping decisions this
workpad implements come from `workpads/architecture/capability-permissions.md`.
