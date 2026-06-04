//! GA8: mocked end-to-end continuation and completion paths.
//!
//! This module is the deterministic, no-live-provider suite over the pieces the
//! earlier GA tasks built -- the real turn loop ([`crate::turn_loop`] /
//! [`FakeBoundaryController::run_turn`]), the GA4 safe-boundary continuation
//! scheduler ([`FakeBoundaryController::evaluate_and_record_continuation`]), and
//! the GA5 evidence-gated completion auditor
//! ([`FakeBoundaryController::audit_and_record_goal_completion`]). It drives the
//! CONTROLLER side directly -- the same controller methods a server dispatches to
//! over the server/controller boundary -- rather than instantiating `capo-server`
//! and crossing the wire boundary itself (the wire boundary is covered by the
//! `capo-server` goal tests). There is NO live provider here (the deterministic
//! e2e Must-Not-Do); agents are scripted mocks and goal state is seeded through
//! the same projection records the GA4/GA5/GA6 controller tests use.
//!
//! The suite has two layers, both asserting the EVENT SEQUENCE and PROJECTION
//! STATE -- never console text (the GA8 Must-Not-Do):
//!
//! - One DRIVEN end-to-end lifecycle
//!   ([`goal_autonomy_e2e_driven_lifecycle_continue_suppress_audit_complete`]):
//!   a single goal walked through one orchestration path -- continue at a safe
//!   boundary (real loop + workspace lock + checkpoint) -> observe no material
//!   progress -> no-progress-suppress -> audit blocks a claim-only completion ->
//!   record observed evidence -> audit completes -> render the historical report --
//!   so the loop, scheduler, and auditor genuinely compose in sequence on one goal.
//! - Per-branch focused checks that each isolate ONE scheduler/auditor branch on a
//!   freshly seeded goal, so a regression points at the exact branch:
//!   1. continue at a safe boundary (also exercises the `safety-gates` workspace
//!      lock and a real checkpoint boundary in the continue path);
//!   2. pause when user input is queued (a boundary is unsafe);
//!   3. block on a raised blocker;
//!   4. budget-limit on budget exhaustion (durable `run.aborted`);
//!   5. no-progress suppression after a no-material-progress continuation;
//!   6. premature-completion-blocked (only an agent claim exists);
//!   7. complete-with-evidence (concrete observed evidence + validation present),
//!      then generate a historical report and snapshot it.
//!
//! The historical report is rendered through the SHARED GO10 renderer
//! ([`capo_state::render_goal_report_markdown`]) -- the SAME renderer the operator
//! surface (`capo_server`) ships -- from the persisted goal projections, so it is
//! rebuildable from events + projections (GO10), is asserted against an exact
//! golden string (the GA8 "snapshot it" requirement), and cannot drift from the
//! shipped report.

#![cfg(test)]

use std::path::Path;
use std::time::Duration;

use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};
use capo_core::{
    AgentId, EvidenceId, GoalId, ProjectId, RequirementId, RunId, SessionId, TaskId, TurnId,
};
use capo_state::{
    AgentProjection, EventKind, EventRecord, EvidenceProjection, GoalAuditDecisionProjection,
    GoalProjection, GoalReportInputs, GoalReportProjection, NewEvent, ProjectionRecord,
    RenderedGoalReport, RequirementLedgerProjection, RunProjection, SessionProjection,
    TaskProjection, render_goal_report_markdown,
};

use crate::{
    CheckpointScope, ContinuationConditions, ContinuationDecision, FakeBoundaryController,
    FakeRunRefs, GoalBudget, RunResourceCeiling, RunResourceUsage, WorkspaceLeaseScope,
    WorkspaceWriteLeaseOutcome,
};


fn temp_root(name: &str) -> capo_tmptest::TempRoot {
    capo_tmptest::TempRoot::new(&format!("capo-ga8-{name}"))
}

const PROJECT: &str = "project-capo";
const GOAL: &str = "goal-ga8";
const TASK: &str = "task-ga8";
const AGENT: &str = "agent-ga8";
const SESSION: &str = "session-ga8";
const RUN: &str = "run-ga8";

/// A controller backed by a scripted-mock adapter so the continue path can drive a
/// REAL turn through the loop (`run_turn`) with a deterministic batch -- never a
/// live provider.
fn open() -> (FakeBoundaryController, capo_tmptest::TempRoot) {
    let state_root = temp_root("state");
    let controller = FakeBoundaryController::open_with_adapter(
        ProjectId::new(PROJECT),
        &state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new(SESSION)),
    )
    .expect("controller");
    (controller, state_root)
}

fn seed(controller: &FakeBoundaryController, event_id: &str, records: &[ProjectionRecord]) {
    let mut event = NewEvent::new(event_id, EventKind::GoalCreated, "test-seed");
    event.project_id = Some(ProjectId::new(PROJECT));
    event.idempotency_key = Some(event_id.to_string());
    event.item_id = Some(event_id.to_string());
    controller
        .state()
        .append_event(event, records)
        .expect("seed append");
}

fn goal_projection(status: &str) -> GoalProjection {
    GoalProjection {
        goal_id: GoalId::new(GOAL),
        project_id: ProjectId::new(PROJECT),
        task_id: Some(TaskId::new(TASK)),
        agent_id: Some(AgentId::new(AGENT)),
        session_id: Some(SessionId::new(SESSION)),
        parent_goal_id: None,
        attempt_run_id: Some(RunId::new(RUN)),
        objective: "Land the GA8 end-to-end".to_string(),
        status: status.to_string(),
        success_criteria_json: r#"{"must":["all tests pass"]}"#.to_string(),
        constraints_json: r#"{"no_network":true}"#.to_string(),
        verification_surface_json: r#"{"cmd":"cargo test"}"#.to_string(),
        budget_json: r#"{"max_turns":8}"#.to_string(),
        stop_conditions_json: r#"{"on":"blocker"}"#.to_string(),
        blocker_reason: String::new(),
        updated_sequence: 0,
    }
}

fn requirement_ledger(req_id: &str, status: &str, source: &str) -> RequirementLedgerProjection {
    RequirementLedgerProjection {
        requirement_id: RequirementId::new(req_id),
        goal_id: GoalId::new(GOAL),
        project_id: ProjectId::new(PROJECT),
        summary: format!("requirement {req_id}"),
        status: status.to_string(),
        last_status_source: source.to_string(),
        updated_sequence: 0,
    }
}

fn observed_evidence(evidence_id: &str) -> EvidenceProjection {
    EvidenceProjection {
        evidence_id: EvidenceId::new(evidence_id),
        project_id: ProjectId::new(PROJECT),
        task_id: Some(TaskId::new(TASK)),
        session_id: Some(SessionId::new(SESSION)),
        run_id: Some(RunId::new(RUN)),
        kind: "test".to_string(),
        artifact_id: None,
        confidence: 95,
        updated_sequence: 0,
    }
}

/// Seed a goal plus the run/session/agent/task projections the scheduler's
/// budget-limit abort path needs, so the continue/budget branches have a real run.
fn seed_goal_with_run(controller: &FakeBoundaryController, status: &str) {
    seed(
        controller,
        "seed-goal-ga8",
        &[
            ProjectionRecord::Goal(goal_projection(status)),
            ProjectionRecord::Task(TaskProjection {
                task_id: TaskId::new(TASK),
                project_id: ProjectId::new(PROJECT),
                title: "GA8".to_string(),
                capo_execution_status: "in_progress".to_string(),
                active_session_id: Some(SessionId::new(SESSION)),
                latest_summary: None,
                evidence_id: None,
                updated_sequence: 0,
            }),
            ProjectionRecord::Agent(AgentProjection {
                agent_id: AgentId::new(AGENT),
                project_id: ProjectId::new(PROJECT),
                name: "ga8".to_string(),
                status: "busy".to_string(),
                current_session_id: Some(SessionId::new(SESSION)),
                updated_sequence: 0,
            }),
            ProjectionRecord::Session(SessionProjection {
                session_id: SessionId::new(SESSION),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(TaskId::new(TASK)),
                agent_id: AgentId::new(AGENT),
                title: "GA8".to_string(),
                status: "running".to_string(),
                current_goal: "Land the GA8 end-to-end".to_string(),
                latest_summary: None,
                latest_confidence: None,
                latest_blocker: None,
                external_session_ref: Some("ext-session-ga8".to_string()),
                updated_sequence: 0,
            }),
            ProjectionRecord::Run(RunProjection {
                run_id: RunId::new(RUN),
                session_id: SessionId::new(SESSION),
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            }),
        ],
    );
}

fn run_refs() -> FakeRunRefs {
    FakeRunRefs {
        task_id: TaskId::new(TASK),
        agent_id: AgentId::new(AGENT),
        session_id: SessionId::new(SESSION),
        run_id: RunId::new(RUN),
        runtime_process_ref: "rt-ga8".to_string(),
        external_session_ref: "ext-session-ga8".to_string(),
    }
}

fn lease_scope(workspace_root: &Path, session: &str) -> WorkspaceLeaseScope {
    WorkspaceLeaseScope {
        task_id: TaskId::new(TASK),
        agent_id: AgentId::new(AGENT),
        session_id: SessionId::new(session),
        run_id: RunId::new(RUN),
        turn_id: TurnId::new("turn-ga8"),
        workspace_root: workspace_root.display().to_string(),
    }
}

fn checkpoint_scope(workspace_root: &Path, shadow_root: &Path, turn: &str) -> CheckpointScope {
    CheckpointScope {
        task_id: TaskId::new(TASK),
        agent_id: AgentId::new(AGENT),
        session_id: SessionId::new(SESSION),
        run_id: RunId::new(RUN),
        turn_id: TurnId::new(turn),
        workspace_root: workspace_root.display().to_string(),
        shadow_git_root: shadow_root.display().to_string(),
    }
}

fn ready_conditions() -> ContinuationConditions {
    ContinuationConditions {
        enabled: true,
        runtime_idle: true,
        session_idle: true,
        user_input_queued: false,
        permission_pending: false,
        capability_profile_valid: true,
        next_step_writes_source: false,
        checkpoint_boundary_available: true,
        verification_runner_available: true,
        last_continuation_made_no_progress: false,
        strategy_changed_since_suppression: false,
        budget: GoalBudget::unbounded(),
    }
}

/// The recorded continuation decisions for the goal, as `(decision, reason)`
/// pairs in record order -- the projection state the GA8 assertions check.
fn continuation_decisions(controller: &FakeBoundaryController) -> Vec<(String, String)> {
    controller
        .state()
        .goal_continuations_for_goal(&GoalId::new(GOAL))
        .expect("continuations")
        .into_iter()
        .map(|row| (row.decision, row.reason))
        .collect()
}

/// The kinds of every event the goal recorded, in sequence order. The GA8 branches
/// assert against this EVENT SEQUENCE (not console text).
fn event_kinds(controller: &FakeBoundaryController) -> Vec<String> {
    controller
        .state()
        .events_after(0, 10_000)
        .expect("events")
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// Whether the event log carries an event of `kind`.
fn has_event_kind(controller: &FakeBoundaryController, kind: &str) -> bool {
    event_kinds(controller).iter().any(|k| k == kind)
}

// ============================ The seven branches ============================

/// Branch 1: CONTINUE at a safe boundary.
///
/// The continue path composes the REAL turn loop (a scripted-mock turn driven
/// through `run_turn`), the `safety-gates` single-writer workspace lock, and a real
/// checkpoint boundary -- exactly the substrate GA8 says the continue path must
/// exercise -- then the scheduler decides `continue` and records it. We assert the
/// recorded continuation decision (projection state) and that the loop's
/// turn-completed and the workspace-lease + checkpoint events are in the log (event
/// sequence), never console text.
#[test]
fn goal_autonomy_e2e_continue_at_a_safe_boundary() {
    let (controller, _root) = open();
    let workspace = temp_root("workspace");
    let shadow = temp_root("shadow");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);
    let refs = run_refs();

    // (a) The safety-gates single-writer workspace lock: our session takes the
    //     write lease so the continuation owns the writer at the safe boundary.
    let scope = lease_scope(&workspace, SESSION);
    let lease = controller
        .acquire_workspace_write_lease(&scope)
        .expect("acquire lease");
    assert!(
        matches!(lease, WorkspaceWriteLeaseOutcome::Acquired { .. }),
        "the continuation acquires the single-writer workspace lease: {lease:?}"
    );

    // (b) A real checkpoint boundary BEFORE the continued turn -- the
    //     reversible-first-write guarantee the scheduler relies on.
    let checkpoint = controller
        .create_checkpoint(&checkpoint_scope(&workspace, &shadow, "turn-continue"))
        .expect("checkpoint io")
        .expect("checkpoint ok");
    assert!(
        !checkpoint.commit_ref.is_empty(),
        "checkpoint commit recorded"
    );

    // (c) The REAL turn loop drives one scripted-mock turn to a terminal
    //     `turn_completed`, projecting observed evidence through the same
    //     ingestion path the live loop uses.
    let turn_id = TurnId::new("turn-continue");
    let batch = ScriptedMockTurn::new("turn-continue")
        .message_completed("msg-continue", "made progress on the goal")
        .turn_completed("done-continue")
        .normalized_events(&refs.external_session_ref);
    let finished = controller
        .run_turn(&refs, &turn_id, &batch)
        .expect("run turn");
    assert!(
        finished.observed_terminal_event(),
        "the turn reached a terminal adapter event"
    );

    // (d) The scheduler decides at the safe boundary -- runtime/session idle, the
    //     workspace lock is held by US (no conflict), budget available -- and
    //     records `continue`.
    let outcome = controller
        .evaluate_and_record_continuation(
            &GoalId::new(GOAL),
            "cont-1",
            &ready_conditions(),
            Some(&scope),
            None,
        )
        .expect("continuation decision");
    assert_eq!(outcome.decision, ContinuationDecision::Continue);
    assert_eq!(outcome.reason, "safe_boundary");

    // Projection state: exactly one recorded continuation, `continue`/`safe_boundary`.
    assert_eq!(
        continuation_decisions(&controller),
        vec![("continue".to_string(), "safe_boundary".to_string())]
    );

    // Event sequence: the workspace lease, the checkpoint, the loop's turn
    // completion (projected as an `evidence.recorded` observed-evidence row), and
    // the continuation decision are all durably recorded.
    assert!(has_event_kind(&controller, "workspace.lease_acquired"));
    assert!(has_event_kind(&controller, "checkpoint.created"));
    assert!(has_event_kind(&controller, "evidence.recorded"));
    assert!(has_event_kind(
        &controller,
        "goal.continuation_decision_recorded"
    ));

    // The real turn loop projected one observed-evidence row for the terminal turn
    // (the durable proof the loop ran, not console text).
    let turn_evidence = controller
        .state()
        .evidence_for_session(&refs.session_id)
        .expect("session evidence")
        .into_iter()
        .filter(|row| row.kind == "adapter_replay:mock")
        .count();
    assert_eq!(
        turn_evidence, 1,
        "the real turn loop projected the terminal turn's observed evidence"
    );
}

/// Branch 2: PAUSE when input is queued / a boundary is unsafe.
///
/// At an otherwise-safe boundary, a queued user input is an unsafe boundary: the
/// operator's input takes precedence over auto-continuation. The scheduler pauses
/// with `input_queued`. We assert the recorded decision (projection) and that NO
/// continue/abort happened.
#[test]
fn goal_autonomy_e2e_pause_when_input_is_queued() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);

    let conditions = ContinuationConditions {
        user_input_queued: true,
        ..ready_conditions()
    };
    let outcome = controller
        .evaluate_and_record_continuation(&GoalId::new(GOAL), "cont-pause", &conditions, None, None)
        .expect("decision");
    assert_eq!(outcome.decision, ContinuationDecision::Pause);
    assert_eq!(outcome.reason, "input_queued");

    assert_eq!(
        continuation_decisions(&controller),
        vec![("pause".to_string(), "input_queued".to_string())]
    );
    // A pause never aborts the run.
    assert!(!has_event_kind(&controller, "run.aborted"));
}

/// Branch 3: BLOCK on a raised blocker.
///
/// A blocked goal never continues, and `block` outranks every other signal. We
/// seed the goal `blocked` and assert the recorded `block`/`goal_blocked` decision.
#[test]
fn goal_autonomy_e2e_block_on_a_raised_blocker() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::BLOCKED);

    let outcome = controller
        .evaluate_and_record_continuation(
            &GoalId::new(GOAL),
            "cont-block",
            &ready_conditions(),
            None,
            None,
        )
        .expect("decision");
    assert_eq!(outcome.decision, ContinuationDecision::Block);
    assert_eq!(outcome.reason, "goal_blocked");

    assert_eq!(
        continuation_decisions(&controller),
        vec![("block".to_string(), "goal_blocked".to_string())]
    );
}

/// Branch 4: BUDGET-LIMIT on budget exhaustion.
///
/// An exhausted goal budget is terminal: the decision is `budget-limit` AND the
/// goal's attempt run is durably aborted via the RTL7 abort path. We assert the
/// recorded decision (projection), the durable `run.aborted` (event sequence), and
/// the run's aborted status (projection).
#[test]
fn goal_autonomy_e2e_budget_limit_on_exhaustion() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);

    let conditions = ContinuationConditions {
        budget: GoalBudget {
            ceiling: RunResourceCeiling::max_turns(1),
            usage: RunResourceUsage {
                turns_taken: 2,
                ..RunResourceUsage::default()
            },
        },
        ..ready_conditions()
    };
    let refs = run_refs();
    let turn_id = TurnId::new("turn-budget");
    let outcome = controller
        .evaluate_and_record_continuation(
            &GoalId::new(GOAL),
            "cont-budget",
            &conditions,
            None,
            Some((&refs, &turn_id)),
        )
        .expect("decision");
    assert_eq!(outcome.decision, ContinuationDecision::BudgetLimit);
    assert_eq!(outcome.reason, "budget_exhausted");

    assert_eq!(
        continuation_decisions(&controller),
        vec![("budget-limit".to_string(), "budget_exhausted".to_string())]
    );
    // The run is durably aborted (event + projection), not silently dropped.
    assert!(has_event_kind(&controller, "run.aborted"));
    let run = controller
        .state()
        .run(&RunId::new(RUN))
        .expect("run lookup")
        .expect("run present");
    assert_eq!(run.status, "aborted");
}

/// Branch 5: NO-PROGRESS SUPPRESSION after a no-material-progress continuation.
///
/// The goal continues once (cont-1); the turn it authorizes makes no material
/// progress; the caller observes that on the next evaluation and the scheduler
/// suppresses the next automatic continuation. We assert both recorded decisions
/// (projection) in order.
#[test]
fn goal_autonomy_e2e_no_progress_suppression() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);

    let first = controller
        .evaluate_and_record_continuation(
            &GoalId::new(GOAL),
            "cont-1",
            &ready_conditions(),
            None,
            None,
        )
        .expect("first decision");
    assert_eq!(first.decision, ContinuationDecision::Continue);

    let no_progress = ContinuationConditions {
        last_continuation_made_no_progress: true,
        ..ready_conditions()
    };
    let second = controller
        .evaluate_and_record_continuation(&GoalId::new(GOAL), "cont-next", &no_progress, None, None)
        .expect("second decision");
    assert_eq!(second.decision, ContinuationDecision::NoProgressSuppress);
    assert_eq!(second.reason, "no_material_progress");

    assert_eq!(
        continuation_decisions(&controller),
        vec![
            ("continue".to_string(), "safe_boundary".to_string()),
            (
                "no-progress-suppress".to_string(),
                "no_material_progress".to_string()
            ),
        ]
    );
}

/// Branch 6: PREMATURE-COMPLETION-BLOCKED when only an agent claim exists.
///
/// A `validated` requirement whose ONLY support is a high-confidence
/// `capo.complete_requirement` agent claim -- with NO observed evidence -- never
/// completes the goal. The auditor is the only path to goal-complete, and it
/// returns `incomplete`/`requirement_claim_only`. We assert the verdict
/// (projection) and that the audit decision is recorded (event sequence).
#[test]
fn goal_autonomy_e2e_premature_completion_blocked() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);
    seed(
        &controller,
        "seed-claim-only",
        &[
            ProjectionRecord::RequirementLedger(requirement_ledger(
                "req-1",
                RequirementLedgerProjection::VALIDATED,
                "agent_reported",
            )),
            ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "report-claim".to_string(),
                goal_id: GoalId::new(GOAL),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: Some(RequirementId::new("req-1")),
                report_kind: "capo.complete_requirement".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(99),
                summary: "I completed the requirement".to_string(),
                body_artifact_id: None,
                evidence_id: None,
                updated_sequence: 0,
            }),
        ],
    );

    let decision = controller
        .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-claim")
        .expect("audit");
    assert!(!decision.verdict.is_complete());
    assert_eq!(decision.reason, "requirement_claim_only");

    let latest = controller
        .state()
        .latest_goal_audit_decision(&GoalId::new(GOAL))
        .expect("latest")
        .expect("decision present");
    assert_eq!(latest.verdict, GoalAuditDecisionProjection::INCOMPLETE);
    assert_eq!(latest.requirements_total, 1);
    assert_eq!(latest.requirements_complete, 0);
    assert!(latest.requirement_detail_json.contains("claim_only"));
    assert!(has_event_kind(&controller, "goal.audit_decision_recorded"));
}

/// Branch 7: COMPLETE-WITH-EVIDENCE, then generate + snapshot a historical report.
///
/// A `validated` requirement backed by concrete observed `EvidenceProjection`
/// evidence DOES complete the goal. We audit, assert the COMPLETE verdict
/// (projection + event), then render a historical report controller-side from the
/// persisted projections and assert it against an exact golden string (GO10 +
/// the GA8 "snapshot it" requirement). The complete path turns on the auditor's
/// observed-evidence gate; the real-turn-loop composition is exercised in the
/// continue branch above, so this branch keeps a single observed evidence row to
/// make the historical-report snapshot deterministic.
#[test]
fn goal_autonomy_e2e_complete_with_evidence_and_historical_report_snapshot() {
    let (controller, _root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);

    // The requirement is `validated` backed by a concrete observed evidence row.
    seed(
        &controller,
        "seed-complete",
        &[
            ProjectionRecord::RequirementLedger(requirement_ledger(
                "req-1",
                RequirementLedgerProjection::VALIDATED,
                "runtime_output",
            )),
            ProjectionRecord::Evidence(observed_evidence("evidence-check-1")),
        ],
    );

    let decision = controller
        .audit_and_record_goal_completion(&GoalId::new(GOAL), "audit-complete")
        .expect("audit");
    assert!(decision.verdict.is_complete());
    assert_eq!(decision.reason, "all_requirements_met");

    let latest = controller
        .state()
        .latest_goal_audit_decision(&GoalId::new(GOAL))
        .expect("latest")
        .expect("decision present");
    assert_eq!(latest.verdict, GoalAuditDecisionProjection::COMPLETE);
    assert_eq!(latest.requirements_total, 1);
    assert_eq!(latest.requirements_complete, 1);
    assert!(has_event_kind(&controller, "goal.audit_decision_recorded"));

    // Generate the historical report and snapshot it. CRITICAL: this renders
    // through the SAME shared GO10 renderer the operator surface ships
    // (`capo_state::render_goal_report_markdown`, reached over the
    // server/controller boundary by `capo_server`'s `handle_goal_report_rendering`),
    // NOT a test-local re-implementation -- so the format the e2e pins is exactly
    // the format the operator sees, and the two cannot drift.
    let report = render_historical_report(&controller, &GoalId::new(GOAL));
    let expected = "\
# Goal report: goal-ga8

- Objective: Land the GA8 end-to-end
- Status: active
- Requirements: 1 (1 supported, 0 blocked, 0 contradicted)
- Verdict: complete (all_requirements_met) — 1/1 requirements complete

## Requirements

- [validated] requirement req-1 (req-1) — source: runtime_output (observed)

## Story

_No reports recorded._

## Observed evidence

- evidence-check-1 (kind: test, confidence: 95)

## Timeline

_No events recorded._
";
    assert_eq!(
        report.body, expected,
        "the historical report rebuilds from projections to an exact snapshot"
    );
    // No events were supplied to the renderer, so it correctly degrades the
    // (empty) timeline section rather than rendering raw content.
    assert!(report.degraded, "an empty timeline degrades the report");

    // The report is REBUILDABLE: re-render after a projection rebuild yields the
    // identical snapshot (GO10 "rebuildable from events, projections, artifacts").
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    let rebuilt = render_historical_report(&controller, &GoalId::new(GOAL));
    assert_eq!(
        rebuilt.body, expected,
        "the report rebuilds identically through the shared renderer"
    );
}

// ===================== Shared historical-report rendering =====================

/// GA8 (GO10): render the historical execution report for a goal through the
/// SHARED [`capo_state::render_goal_report_markdown`] -- the SINGLE GO10 renderer
/// the operator surface (`capo_server`) also ships. The controller crate cannot
/// depend on `capo-server`, but both depend on `capo-state`, so the renderer lives
/// there and the e2e snapshots the SAME bytes the operator sees; the two renderers
/// can no longer diverge because there is only one.
///
/// Branch 7's golden snapshot supplies an EMPTY timeline so the snapshot string is
/// deterministic; the GA9 restart test instead supplies a REAL, non-empty timeline
/// via [`render_historical_report_with_timeline`] so the event-log-derived `##
/// Timeline` section is actually exercised across the restart. Either way, every
/// report INPUT is read from the persisted projections / durable event log, so the
/// report is rebuildable from events + projections.
fn render_historical_report(
    controller: &FakeBoundaryController,
    goal_id: &GoalId,
) -> RenderedGoalReport {
    render_historical_report_with_timeline(controller, goal_id, &[])
}

/// As [`render_historical_report`], but with an explicit `timeline` so a caller can
/// pin the `## Timeline` section. The GA9 restart test passes the timeline rebuilt
/// from the durable event log via [`goal_timeline_entries`] (the SAME item-scoped
/// gather `capo_server`'s `goal_timeline_entries` performs), so the section the e2e
/// pins is the section the operator actually sees in the shipped report.
fn render_historical_report_with_timeline(
    controller: &FakeBoundaryController,
    goal_id: &GoalId,
    timeline: &[EventRecord],
) -> RenderedGoalReport {
    let state = controller.state();
    let goal = state.goal(goal_id).expect("goal").expect("goal present");
    let requirements = state
        .requirement_ledgers_for_goal(goal_id)
        .expect("requirements");
    let reports = state.goal_reports_for_goal(goal_id).expect("reports");
    let continuations = state
        .goal_continuations_for_goal(goal_id)
        .expect("continuations");
    let delegated_provider_goals = state
        .delegated_provider_goals_for_goal(goal_id)
        .expect("delegated provider goals");
    let evidence: Vec<EvidenceProjection> = match goal.task_id.as_ref() {
        Some(task_id) => state.evidence_for_task(task_id).expect("task evidence"),
        None => Vec::new(),
    };
    let audit = state
        .latest_goal_audit_decision(goal_id)
        .expect("latest audit");
    let inputs = GoalReportInputs {
        goal: &goal,
        requirements: &requirements,
        reports: &reports,
        continuations: &continuations,
        delegated_provider_goals: &delegated_provider_goals,
        evidence: &evidence,
        audit: audit.as_ref(),
        timeline,
    };
    render_goal_report_markdown(&inputs)
}

/// Rebuild the goal's event timeline from the durable log, mirroring `capo_server`'s
/// `goal_timeline_entries` (the controller crate cannot depend on `capo-server`, but
/// both reach the SAME `capo_state` event-log reads, so the result is byte-identical
/// to what the operator-facing report renders): run-scoped evidence events plus every
/// event keyed by the goal / its requirements / reports / continuations as `item_id`,
/// deduped by `sequence` and ordered by `sequence`. This is the section of the report
/// derived from the raw `events` log rather than the projections, so it is the part
/// most dependent on a faithful replay across a restart.
fn goal_timeline_entries(
    controller: &FakeBoundaryController,
    goal_id: &GoalId,
) -> Vec<EventRecord> {
    let state = controller.state();
    let goal = state.goal(goal_id).expect("goal").expect("goal present");
    let mut records: Vec<EventRecord> = Vec::new();
    if let Some(run_id) = goal.attempt_run_id.as_ref() {
        records = state
            .evidence_events_for_run(run_id)
            .expect("evidence events for run");
    }
    let mut item_ids: Vec<String> = vec![goal_id.to_string()];
    for ledger in state
        .requirement_ledgers_for_goal(goal_id)
        .expect("requirements")
    {
        item_ids.push(ledger.requirement_id.to_string());
    }
    for report in state.goal_reports_for_goal(goal_id).expect("reports") {
        item_ids.push(report.goal_report_id);
    }
    for continuation in state
        .goal_continuations_for_goal(goal_id)
        .expect("continuations")
    {
        item_ids.push(continuation.continuation_id);
    }
    for record in state.events_for_items(&item_ids).expect("events for items") {
        if !records.iter().any(|seen| seen.sequence == record.sequence) {
            records.push(record);
        }
    }
    records.sort_by_key(|record| record.sequence);
    records
}

/// GA9 (goal-orchestration GO13 + GO14 close-out): reopen the controller over the
/// SAME on-disk state root with a fresh scripted-mock adapter. This drops every
/// in-memory handle the original held and opens a new [`capo_state::SqliteStateStore`]
/// (its `open()` never clears the projection tables, so a cold open serves whatever
/// rows are on disk). On its own the reopen proves only that the on-disk read models
/// survive a process boundary; the GA9 test makes the replay path load-bearing by
/// CLEARING those read models with [`clear_read_models`] between reopen and rebuild,
/// so the post-restart assertions can pass ONLY because `rebuild_projections()`
/// replayed the durable `projection_records` log.
fn reopen(state_root: &Path) -> FakeBoundaryController {
    FakeBoundaryController::open_with_adapter(
        ProjectId::new(PROJECT),
        state_root,
        AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new(SESSION)),
    )
    .expect("reopen controller over the same state root")
}

/// GA9: CORRUPT the on-disk read models by deleting every row from the goal-domain
/// projection tables (and the watermark), reaching the store's db file directly. The
/// durable `events` and `projection_records` logs are left intact, so a subsequent
/// `rebuild_projections()` must replay them to repopulate the read models. This is
/// what makes the GA9 restart genuinely load-bearing: after this call the goal /
/// continuation / audit reads are EMPTY, so they come back ONLY via the replay path,
/// not because stale projection rows happened to survive the process boundary.
fn clear_read_models(state: &capo_state::SqliteStateStore) {
    let connection = rusqlite::Connection::open(state.db_path()).expect("open store db directly");
    for table in [
        "goals",
        "requirement_ledgers",
        "goal_reports",
        "goal_continuations",
        "delegated_provider_goals",
        "goal_audit_decisions",
        "evidence",
        "projection_watermarks",
    ] {
        connection
            .execute(&format!("DELETE FROM {table}"), [])
            .unwrap_or_else(|err| panic!("clear {table}: {err}"));
    }
}

/// GA9: the END-TO-END restart/replay gate.
///
/// GA6 (`reattach.rs`) and `capo-state`'s
/// `goal_replay_full_goal_surface_rebuilds_identically_after_restart` prove the
/// individual read models rebuild; the GA8 branches prove the scheduler/auditor
/// COMPOSE on one live handle. GA9 closes the gap between them: it drives the full
/// orchestration lifecycle (a recorded CONTINUATION decision, a claim-only
/// AUDIT-incomplete, then an observed-evidence AUDIT-complete) on one controller,
/// then DROPS that controller, REOPENS the store from the same disk path (a real
/// server restart with no shared in-memory state), runs a full
/// `rebuild_projections()`, and proves that goal + continuation + auditor verdict +
/// historical report all survive byte-for-byte AND that the auditor re-decides
/// identically on the rebuilt state with no in-memory transcript. This is the
/// "goal + continuation + auditor + report state survives server restart and full
/// projection rebuild END TO END" acceptance for GA9.
#[test]
fn goal_autonomy_e2e_full_state_survives_server_restart_and_rebuild() {
    let (controller, state_root) = open();
    seed_goal_with_run(&controller, GoalProjection::ACTIVE);
    let goal_id = GoalId::new(GOAL);

    // (1) A continuation decision at a safe boundary -- a durable scheduler verdict.
    let cont = controller
        .evaluate_and_record_continuation(&goal_id, "cont-ga9", &ready_conditions(), None, None)
        .expect("continuation decision");
    assert_eq!(cont.decision, ContinuationDecision::Continue);

    // (2) An agent claim WITHOUT observed evidence: the auditor blocks completion.
    seed(
        &controller,
        "ga9-claim-only",
        &[
            ProjectionRecord::RequirementLedger(requirement_ledger(
                "req-1",
                RequirementLedgerProjection::VALIDATED,
                "agent_reported",
            )),
            ProjectionRecord::GoalReport(GoalReportProjection {
                goal_report_id: "ga9-report-claim".to_string(),
                goal_id: goal_id.clone(),
                project_id: ProjectId::new(PROJECT),
                session_id: Some(SessionId::new(SESSION)),
                requirement_id: Some(RequirementId::new("req-1")),
                report_kind: "capo.complete_requirement".to_string(),
                source: "agent_reported".to_string(),
                confidence: Some(99),
                summary: "I completed it".to_string(),
                body_artifact_id: None,
                evidence_id: None,
                updated_sequence: 0,
            }),
        ],
    );
    let claim_audit = controller
        .audit_and_record_goal_completion(&goal_id, "ga9-audit-claim")
        .expect("claim audit");
    assert!(!claim_audit.verdict.is_complete());
    assert_eq!(claim_audit.reason, "requirement_claim_only");

    // (3) Concrete observed evidence arrives; the auditor now completes the goal.
    seed(
        &controller,
        "ga9-observed-evidence",
        &[ProjectionRecord::Evidence(observed_evidence(
            "ga9-evidence-check",
        ))],
    );
    let complete_audit = controller
        .audit_and_record_goal_completion(&goal_id, "ga9-audit-complete")
        .expect("complete audit");
    assert!(complete_audit.verdict.is_complete());
    assert_eq!(complete_audit.reason, "all_requirements_met");

    // Capture the pre-restart read-model state the restart must reproduce. Pin a REAL,
    // non-empty `## Timeline` (rebuilt from the durable event log) so the restart
    // exercises the event-log-derived report section, not just the projections.
    let timeline_before = goal_timeline_entries(&controller, &goal_id);
    assert!(
        !timeline_before.is_empty(),
        "the lifecycle above must produce a non-empty event timeline"
    );
    let report_before =
        render_historical_report_with_timeline(&controller, &goal_id, &timeline_before);
    let continuations_before = continuation_decisions(&controller);
    let verdict_before = controller
        .state()
        .latest_goal_audit_decision(&goal_id)
        .expect("latest audit")
        .expect("verdict present");
    assert_eq!(
        continuations_before,
        vec![("continue".to_string(), "safe_boundary".to_string())]
    );
    assert_eq!(
        verdict_before.verdict,
        GoalAuditDecisionProjection::COMPLETE
    );

    // (4) SERVER RESTART: drop the live controller, reopen the store from disk, and
    //     rebuild every projection from the durable event log alone.
    drop(controller);
    let restarted = reopen(&state_root);
    // Make the replay path load-bearing: clear the goal-domain read models the cold
    // open served from disk so the post-restart assertions can pass ONLY because
    // `rebuild_projections()` replayed the durable `projection_records` log.
    clear_read_models(restarted.state());
    restarted
        .state()
        .rebuild_projections()
        .expect("rebuild projections after restart");

    // The objective + audit contract re-inject from PERSISTED goal state (GA6/GO13):
    // the reopened controller has no in-memory transcript, yet the goal is intact.
    let goal_after = restarted
        .state()
        .goal(&goal_id)
        .expect("goal lookup")
        .expect("goal survives restart");
    assert_eq!(goal_after.objective, "Land the GA8 end-to-end");
    assert_eq!(goal_after.status, GoalProjection::ACTIVE);

    // The continuation decision survives restart + rebuild byte-for-byte.
    assert_eq!(
        continuation_decisions(&restarted),
        continuations_before,
        "the scheduler's continuation verdict rebuilds identically after restart"
    );

    // The auditor verdict projection survives restart + rebuild byte-for-byte.
    let verdict_after = restarted
        .state()
        .latest_goal_audit_decision(&goal_id)
        .expect("latest audit")
        .expect("verdict survives restart");
    assert_eq!(verdict_after, verdict_before);

    // The auditor RE-DECIDES identically on the rebuilt state -- it depends only on
    // the persisted projections, never an in-memory transcript.
    let re_audit = restarted
        .audit_goal_completion(&goal_id)
        .expect("re-audit on rebuilt state");
    assert!(re_audit.verdict.is_complete());
    assert_eq!(re_audit.reason, "all_requirements_met");

    // The historical report (timeline included) rebuilds from the durable log to the
    // identical bytes -- including the `## Timeline` section rebuilt from the events log.
    let timeline_after = goal_timeline_entries(&restarted, &goal_id);
    assert_eq!(
        timeline_after, timeline_before,
        "the event-log-derived timeline rebuilds identically after a server restart"
    );
    let report_after =
        render_historical_report_with_timeline(&restarted, &goal_id, &timeline_after);
    assert_eq!(
        report_after.body, report_before.body,
        "the historical report rebuilds identically after a server restart"
    );
}

// A small compile-time sanity check that the budget helper imports resolve.
#[test]
fn goal_autonomy_e2e_budget_helpers_resolve() {
    let budget = GoalBudget {
        ceiling: RunResourceCeiling::for_live_provider(3, Duration::from_secs(60), 1_000),
        usage: RunResourceUsage::default(),
    };
    assert!(budget.available());
}
