# Daily-Driver Harness Track — Plan and Adversarial Review

Prepared 2026-05-29. Motivating review: `workpads/harness-research/daily-driver-review.md`.
This track was decomposed and then improved by an adversarial workflow over five lenses:
architecture-quality, plan/sequencing, red-team, tools-ACI, and codebase-reality.

## Workpad Sequence

real-turn-loop -> (streaming-transport || tools-aci) -> safety-gates -> goal-autonomy -> depth

## Goal-Orchestration Reconciliation

KEEP goal-orchestration as the authoritative DESIGN SOURCE; CARVE its implementation into the new sequence and RE-SEQUENCE it after the prerequisite loop/streaming/tools/safety workpads. This is unchanged from the draft and is correct: I verified goal-orchestration GO0 is done and GO1-GO14 are pending (workpads/goal-orchestration/tasks.md), and GO1-GO13 implicitly assume substrate that does not exist - a real observe->decide->emit loop (today FakeBoundaryController + an AgentAdapter enum with only Fake/ScriptedMock), a real workspace-write adapter, wired permissions, a real verification runner, and checkpoint/rollback. The workpad's own Non-Goals already defer autonomy until goal state, evidence records, stop policy, mocked replay, and checkpoint/rollback exist, so autonomy cannot land first.

Concrete reconciliation:
1. goal-orchestration REMAINS the canonical reference for the goal domain model, the GO2 agent-reporting tool contract, evidence/review/validation ledger semantics, story projections, completion-audit philosophy, parent/child contracts, and provider-native delegation. Its knowledge.md/tasks.md are cited, not duplicated, by the new workpads.
2. Re-sequence goal-orchestration in TASKS.md to AFTER real-turn-loop, streaming-transport, tools-aci, and safety-gates. It stays 'active design / blocked implementation' until those land.
3. The new goal-autonomy workpad IMPLEMENTS the runtime pieces on the now-real substrate and does NOT re-specify the design. Refinement vs the draft: the GO2 reporting-TOOL surface is implemented in tools-aci (an ACI/tool-registry concern); the goal/evidence/report event model + projections (GO1, GO3) and lifecycle/server/read commands (GO4-GO6, GO10) are implemented in goal-autonomy's first milestone depending only on real-turn-loop + tools-aci; the continuation scheduler (GO8), evidence-gated completion auditor (GO9), and reattach-after-compaction (GO13) are the second milestone, hard-gated on safety-gates checkpoint/rollback + verification.
4. After goal-autonomy closes, goal-orchestration's remaining design-only tasks are marked satisfied-by/folded as references, and goal-orchestration is closed as 'design realized in goal-autonomy + tools-aci.'

Net: one design brain (goal-orchestration), one implementation (goal-autonomy + the GO2 surface in tools-aci), no competing goal designs.

## Sequencing Rationale

The spine is the review's true critical path and is confirmed against code: the controller is fake-only (FakeBoundaryController, capo-controller/src/lib.rs), the AgentAdapter is a closed Fake/ScriptedMock enum, the runtime is synchronous (no tokio in capo-runtime), real tool wrappers are dead-routed to FakeToolExposure, permissions are inert (TrustedLocal allow-all), recovery is the blunt mark_active_runs_exited_unknown, and goal-orchestration GO1-GO14 are pending. So real-turn-loop -> streaming-transport / tools-aci (parallel) -> safety-gates -> goal-autonomy -> depth is right.

Order and why:
(1) real-turn-loop FIRST - the substrate everything attaches to. It replaces FakeBoundaryController with a genuine observe->decide->emit loop that DRIVES the existing dispatch primitives (PlanDispatch/Gate/Run) as its execution substrate (one orchestration path, not two), drives one real workspace-write adapter (Codex) end-to-end, fixes the stdout.txt-per-turn overwrite by keying artifacts on turn_id, and - critically - ships a SAFETY FLOOR (path confinement + hard-kill + pre-write checkpoint + resource ceiling + dry-run default + orphan reaping) so the first real write is confined, reversible, bounded, and never unattended. Full safety still lands in phase 4.
(2)/(3) streaming-transport and tools-aci run in PARALLEL after phase 1 (tools-aci no longer hard-depends on streaming). streaming-transport evolves the EXISTING capo-web bridge from 1500ms Dashboard-poll SSE to a broadcast-backed event tail keyed by from_sequence, adds tokio + JSON-RPC notifications + Subscribe + a multi-turn thread read model + typed interrupt + redaction-on-emit, and publishes the schema + wire-snapshot tests as the web CONTRACT (TS types optional/downstream, web/app + web/dashboard frozen). tools-aci makes the ACI real: wires the existing dead-routed registry into the real loop, adds input+output typed schemas, structured edit/patch WITH lint-on-edit, search/locator, a typed test/check tool, input+output redaction + provenance projections, and the GO2 reporting/evidence tools (observed vs agent-reported). It precedes safety because safety gates the tool calls and consumes the test tool's evidence.
(4) safety-gates FOURTH (internally sub-phased: enforcement | verification | checkpoint-recovery): wires PermissionPolicy/ToolExposure into the loop with grant read-back/revoke/expiry (new CapabilityGrantRevoked event kind), fixes the TrustedLocal critical-scope hole, adds a real VerificationRunner + score_run consuming OBSERVED evidence only, a single-writer workspace lock, controller-owned shadow-git checkpoint/rollback, and liveness-aware reattach. Autonomy is unsafe before this.
(5) goal-autonomy FIFTH (reconciled, two milestones): the goal model + projections + read/lifecycle commands depend on real-turn-loop + tools-aci; the continuation scheduler + evidence-gated auditor + reattach depend additionally on safety-gates.
(6) depth LAST (differentiated prerequisites): live ACP JSON-RPC adapter + FTS5 memory depend on real-turn-loop + tools-aci; OS sandbox + worktrees depend on safety-gates; worktree-per-goal depends on goal-autonomy; plus Claude as a second write adapter and optional OTel.

Every workpad enforces the evidence standard: deterministic fake/scripted tests before live providers; cargo fmt/clippy/test; restart+replay where state changes; manual smoke ALWAYS paired with a deterministic assertion; live providers behind explicit opt-in env gates.

## Revision Log (what the adversarial reviews changed)

### Revision 1

**Change:** BLOCKER FIX: Rewrote streaming-transport's web-bridge work from 'build a net-new axum HTTP/SSE bridge' to 'evolve the EXISTING crates/capo-web crate.' Verified crates/capo-web/src/main.rs (316 lines, untracked workspace member) is already an axum+tokio facade over the typed ServerCommand/ServerResponsePayload boundary exposing /api/dashboard, /api/commands, /api/events. Its /api/events is a 1500ms IntervalStream poll of ServerCommand::Dashboard re-serialized as SSE (main.rs ~150-165) - the exact latest_summary-poll antipattern the plan kills. New tasks: git-track capo-web, replace its poll-SSE with the broadcast-backed event tail keyed by from_sequence (delete the poll path), resolve the CapoServer !Send-across-await constraint (currently worked around by per-request spawn_blocking).

**Driven by:** adversarial architecture-quality, adversarial plan/sequencing, codebase-reality

**Rationale:** Three independent lenses verified the same factual error inherited from the prior review ('no tokio/axum in any crate'). Building a second facade would entrench debt and collide with the boundary being protected. capo-web is the server-side Rust bridge and is IN scope to evolve; the WEB CLIENT (web/app, web/dashboard) stays frozen and out of scope.

### Revision 2

**Change:** WEB BOUNDARY re-carved precisely. The server set evolves crates/capo-web (the server-side HTTP/SSE Rust bridge) so a browser CAN consume the event tail, but it does NOT build web/app or web/dashboard. Demoted generated TypeScript types from an authoritative deliverable to an OPTIONAL downstream artifact generated FROM the language-neutral schema; the authoritative contract is the JSON-RPC/SSE schema + checked-in wire-snapshot tests verifiable WITHOUT any web client. Added a documented migration handoff so the web agent can switch its front-end from Dashboard-polling to Subscribe-based tailing.

**Driven by:** adversarial plan/sequencing, red-team-skeptic, codebase-reality

**Rationale:** Conflict resolution: plan-review argued the axum re-exposure AND TS types belong to the web agent and should leave this set entirely; codebase-reality showed capo-web is a Rust crate that already partially satisfies the dashboard handoff. Decision: split ownership at the language boundary - Rust server bridge (capo-web) is ours to evolve; front-end source and TS-type adoption are the web agent's. This keeps hard constraint 1 (server delivers the CONTRACT, not the web client) while not abandoning a half-built Rust bridge.

### Revision 3

**Change:** BLOCKER FIX: Added an explicit real-turn-loop task to reconcile the turn loop with the EXISTING dispatch state machine. Verified PlanDispatch/PreflightLiveProvider/GateDispatch/RunDispatchLocal/RunLiveProviderLocal are real ServerCommands (capo-server/src/types.rs:150-186, lib.rs:528-826) that execute the live Codex run today. Decision documented: the RealBoundaryController turn loop's emit step DRIVES the dispatch primitives (plan->gate->run) as its execution substrate - one orchestration path, loop subsumes dispatch. Added a non-goal forbidding a second execution pipeline, and an acceptance that TurnFinished + per-turn artifacts reconcile with existing dispatch-run-exit events / execution-status projections (no duplicate run-completion semantics).

**Driven by:** adversarial architecture-quality

**Rationale:** Confirmed the dispatch pipeline is a genuine second multi-step orchestration path. real-turn-loop as drafted never named it - the precise definition of the parallel-orchestration-path failure the boundary constraint warns against. The loop must drive dispatch, not run beside it.

### Revision 4

**Change:** BLOCKER FIX: Pulled a SAFETY FLOOR into real-turn-loop, gating the first real workspace write rather than deferring all safety to phase 4. Added: enforced path confinement on the write adapter (the ensure_under_workspace containment engine already exists in capo-tools/runtime_wrapper_paths.rs - wire it now), a controller hard-kill switch, a pre-write single-snapshot checkpoint (git-stash/tar, not full shadow-git), a diff-preview/dry-run default, and a per-run resource ceiling (max turns, max wall-clock, hard token/cost ceiling with controller-enforced abort + run.aborted event). Added a non-goal: RTL live writes are never unattended - opt-in env gate AND diff-preview default; unattended continuation is GA-only on the SG substrate.

**Driven by:** red-team-skeptic

**Rationale:** real-turn-loop ships a live workspace-WRITE adapter in phase 1 while full PermissionPolicy/VerificationRunner/shadow-git land in phase 4 - three workpads of a live model editing a real repo with TrustedLocal allow-all in force (verified AllowTrustedLocalProfilePolicy.decide returns blanket allow, permission.rs:87-94), no rollback, unbounded provider spend. The hazard was relocated earlier, not removed. A minimal reversible/confined/bounded floor must exist the moment the first real write does. Full enforcement still lands in SG.

### Revision 5

**Change:** Split real-turn-loop's AgentAdapter-trait extraction into two tasks: (2a) define provider-neutral session/turn/result types + the AgentAdapter trait; (2b) reimplement Fake/ScriptedMock against the trait and migrate the controller + all tests off concrete Fake* signatures. Added acceptance: no concrete Fake* type at any non-fake call site.

**Driven by:** red-team-skeptic, adversarial architecture-quality

**Rationale:** Verified the enum returns concrete FakeAdapterSession/FakeAdapterTurnOutput/FakeAdapterSessionRequest in its signatures (capo-adapters/src/adapter.rs) and the controller imports FakeAdapterSessionRequest/FakeAdapterTurnRequest directly (capo-controller/src/lib.rs). A real trait forces an abstract-type redesign that ripples cross-crate. Treating it as one task hides a refactor that would silently double RTL's size.

### Revision 6

**Change:** Crash-safe in-flight run handling pulled into real-turn-loop: persist start_requested + pid/process-group before spawn; on restart reap orphaned process groups (reuse the proven descendant reaper). Full liveness-probe reattach stays in safety-gates.

**Driven by:** red-team-skeptic

**Rationale:** RTL introduces real long-running provider processes. Verified the only recovery today is the blunt mark_active_runs_exited_unknown (capo-state/src/lib.rs:251-291) which orphans children. Orphan reaping must exist the moment RTL spawns real processes, not three phases later.

### Revision 7

**Change:** Reduced tools-aci hard dependency from [real-turn-loop, streaming-transport] to [real-turn-loop] only; streaming is a soft/parallel integration (a thin follow-on task streams tool-call/result frames once both exist). tools-aci can now run in parallel with streaming-transport after phase 1.

**Driven by:** adversarial architecture-quality, adversarial plan/sequencing

**Rationale:** The tool registry, typed wrapper output, instrumentation/redaction, edit/patch/search/test quality, and GO2 reporting tools are independently valuable and testable without streaming. Over-coupling delayed the GO2 reporting contract that goal-autonomy needs and serialized parallelizable work.

### Revision 8

**Change:** Reframed tools-aci from greenfield to build-on, and fixed the load-bearing dead-routing bug. Verified CapoToolRegistry + RuntimeToolWrappers (shell_run/file_read/file_write/git_status/git_diff/git_commit) with authorize_and_invoke already EXIST (capo-tools/src/runtime_wrappers.rs), but ToolExposure::invoke hard-routes Capo and Runtime variants to FakeToolExposure (lib.rs:67-73) and the controller is built with ToolExposure::fake() - so the real path is dead code. Added an early task to wire authorize_and_invoke into the real loop and replace the fake-only routing; reframed the registry task as 'extend ToolDefinition with input+output schemas + risk/scope/redaction metadata' not 'build a registry.'

**Driven by:** codebase-reality, tools/ACI, adversarial plan/sequencing

**Rationale:** Three lenses showed the registry/wrappers are built but never reached from the loop. The real gap is wiring + narrow typed output + instrumentation + quality, not greenfield. Re-specifying existing types is wasted work and risks divergence from tool-exposure.md.

### Revision 9

**Change:** Split the conflated tools-aci task 'High-quality edit/patch and search/grep tools' into THREE: a structured edit/patch tool with syntax/lint-on-edit feedback (typed hunks, fuzzy/whitespace-tolerant match, structured retryable no-match error, post-apply lint findings as typed output); a separate search/grep + bounded locator tool; and tightened file_write to stop being a blind whole-file overwrite (require expected-precondition hash or structured replace, emit a unified-diff artifact). Verified edit/patch/search/test tools are ABSENT (zero matches) and file_write (runtime_wrappers.rs:426-455) records only before/after content_hash.

**Driven by:** tools/ACI, codebase-reality

**Rationale:** Edit/patch quality with lint-on-edit is the single highest-value daily-driver ACI lever (codex apply-patch, aider editblock), the prompt explicitly requires syntax/lint checks on edit, and the drafted single task left acceptance undefined and omitted the lint feedback loop.

### Revision 10

**Change:** Tightened tools-aci instrumentation: redaction must apply to BOTH input and output artifacts with a real policy (configurable patterns + default credential-shape/high-entropy scan), and implement the tool-exposure.md ToolInvocation/ToolObservation projections + tool.invocation_started/output_artifact_recorded/observation_recorded events as real state with provenance (correlation_id, permission_decision_id, capability_grant_use_id) and started_at/completed_at timing. Verified current redact_bytes is literal substring replace on INPUT only (runtime_wrappers.rs:515-523) and those projections are design-only.

**Driven by:** tools/ACI, red-team-skeptic

**Rationale:** Output (shell stdout/stderr) is where secrets leak and current redaction never touches it; provenance is designed in tool-exposure.md but unimplemented. Both are required for the observed-vs-reported evidence model goal-autonomy depends on and for later wall-clock eval.

### Revision 11

**Change:** Scoped tools-aci GO2 reporting/evidence tools to: register each reporting tool in the typed registry and persist agent reports as a DISTINCT event class tagged source=agent_reported (with confidence), separate from observed evidence (source=runtime_output/adapter_event); full projection/audit semantics validated in goal-autonomy. Cites goal-orchestration GO2 as design source; does not redesign.

**Driven by:** tools/ACI, adversarial plan/sequencing

**Rationale:** Keeps the GO2 tool surface (an ACI concern) here while leaving audit/projection semantics to goal-autonomy, and enforces that completion is never reachable by agent assertion alone.

### Revision 12

**Change:** SPLIT goal-autonomy dependency structure: the goal/requirement/evidence event model + projections (GO1/GO3) and read/lifecycle/server commands (GO4-GO6, GO10) depend only on real-turn-loop + tools-aci; the continuation scheduler, evidence-gated completion auditor, and reattach-after-compaction (GO8/GO9/GO13) remain hard-dependent on safety-gates. Kept as one workpad with two internal milestones rather than two workpads.

**Driven by:** adversarial architecture-quality

**Rationale:** The goal model and read surfaces don't need checkpoint/rollback; only continuation/auditing does. Splitting the dependency (not the workpad) lets the model land earlier without exploding the workpad count past the 5-7 guidance, and keeps continuation correctly gated on the safety substrate.

### Revision 13

**Change:** Scoped safety-gates 'Handle ACP session/request_permission' to the AgentAdapter-trait permission round-trip against FAKE/SCRIPTED adapters plus the existing ACP option mapping (capability-permissions.md), explicitly NOT the live ACP wire (which lands in depth). Stated the fixture-only verification standard. Kept liveness-aware recovery at the runtime boundary, independent of ACP.

**Driven by:** adversarial architecture-quality

**Rationale:** The live ACP JSON-RPC adapter is deferred to depth, so a live request_permission round-trip cannot land in safety-gates - a hidden cross-phase dependency. Scoping to fakes + option mapping removes it.

### Revision 14

**Change:** Tightened safety-gates TrustedLocal fix to enumerate critical scopes (source-write outside workspace, network egress, secret read, arbitrary shell) in capability-permissions.md and require an explicit grant for each even under TrustedLocal; added a test that AllowTrustedLocalProfilePolicy.decide() denies an un-granted critical-scope request. Made the grant-lifecycle task explicit about ADDING a CapabilityGrantRevoked (and optional CapabilityGrantExpired) EventKind and created_at/expires_at/revoked_at columns to CapabilityGrantProjection, with a replay test for the new kinds.

**Driven by:** adversarial plan/sequencing, codebase-reality

**Rationale:** Verified blanket allow-all (permission.rs:87-94) and that only CapabilityGrantCreated/Used event kinds exist (event.rs:16-17); revoked_at lives only on ConnectivityExposureProjection. The fix is concrete schema work, not just a policy tweak.

### Revision 15

**Change:** Added a single-writer workspace lock / session-scoped write lease task to safety-gates (gates tool writes; the primitive goal-autonomy GO8 'no conflicting workspace lock' consumes). Until it exists, streaming-transport and tools-aci document that concurrent writers are rejected, not interleaved.

**Driven by:** red-team-skeptic

**Rationale:** streaming-transport delivers a MULTI-CLIENT surface (broadcast, multiple subscribers) and tools-aci delivers file_write/edit/patch, so two clients or a client+continuation can drive concurrent writes from phase 2 through phase 4 with no lock. GO8 names the lock as a precondition but never builds it.

### Revision 16

**Change:** Added a runtime output-cap fix as explicit acceptance: a successful run exceeding the output cap must NOT be classified as failed; output is streamed-and-truncated with truncation recorded as metadata, proven by a >cap-successful-run test. Placed on the capo-runtime tokio task with the artifact-keying work.

**Driven by:** adversarial plan/sequencing

**Rationale:** Verified capped_output (capo-runtime/src/lib.rs:1351) returns Err(OutputLimitExceeded) and the run discards artifacts on overflow (test at line 1640) - a long successful run is misclassified as an error. The tokio rewrite must fix classification, not just streaming.

### Revision 17

**Change:** Added redaction-on-emit to streaming-transport: apply the existing RedactionState guard to the broadcast/SSE path before any frame leaves the process, with a test asserting a known secret never appears on the wire. Preserved descendant-process reaping with a ported process-group-kill regression test plus a new orphan-after-cancel reaping test on the tokio runtime task.

**Driven by:** red-team-skeptic, adversarial architecture-quality

**Rationale:** A multi-client stream is a new secret-egress surface; redaction must guard it. The tokio move must prove the existing process-group kill semantics survive.

### Revision 18

**Change:** Moved the Claude workspace-write adapter out of real-turn-loop (phase 1) to depth. One real workspace-write provider (Codex) is sufficient to make the loop real; a second provider is breadth.

**Driven by:** adversarial plan/sequencing

**Rationale:** Trims the over-scoped phase-1 workpad to one conceptual responsibility (make ONE real write loop work) and keeps the critical path lean. Conflict note: I did NOT split safety-gates into two workpads (plan-review's alternative) because that pushes past the 5-7 guidance; instead I trimmed RTL and sub-phased safety-gates internally.

### Revision 19

**Change:** Added an explicit per-task Acceptance+Verification invariant to every workpad's task 0: no task completes on operator self-attestation alone; every manual smoke is paired with a deterministic assertion (wire snapshot, exit status, or replay). Live-provider work stays behind explicit opt-in env gates mirroring CAPO_SERVER_RUN_CODEX_LIVE / CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT.

**Driven by:** red-team-skeptic

**Rationale:** Hard constraint 4 requires checkable acceptance AND a verification standard per task; 'manual smoke' alone is the operator-asserted antipattern the review flagged. The scribes expand prose, but the invariant must be stated in task 0.

### Revision 20

**Change:** Split depth's prerequisites: live ACP adapter and FTS5 memory depend on [real-turn-loop, tools-aci]; OS sandbox + worktree isolation depend on safety-gates (checkpoint/recovery); only worktree-per-goal depends on goal-autonomy. depth keeps a single workpad but its internal tasks carry differentiated prerequisites.

**Driven by:** adversarial plan/sequencing

**Rationale:** ACP and memory hardening don't need autonomy; only worktree-per-goal does. Differentiating lets breadth start once true prerequisites land without re-architecting earlier phases.

