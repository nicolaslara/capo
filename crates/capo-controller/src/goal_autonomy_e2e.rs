//! GA8: mocked end-to-end continuation and completion paths.
//!
//! This module is the deterministic, no-live-provider end-to-end suite that
//! COMPOSES the pieces the earlier GA tasks built -- the real turn loop
//! ([`crate::turn_loop`] / [`FakeBoundaryController::run_turn`]), the GA4 safe-
//! boundary continuation scheduler
//! ([`FakeBoundaryController::evaluate_and_record_continuation`]), and the GA5
//! evidence-gated completion auditor
//! ([`FakeBoundaryController::audit_and_record_goal_completion`]) -- through the
//! controller side of the server/controller boundary, exactly as a server would
//! drive them. There is NO live provider here (the deterministic e2e Must-Not-Do);
//! agents are scripted mocks and goal state is seeded through the same projection
//! records the GA4/GA5/GA6 controller tests use.
//!
//! It walks each scheduler/auditor branch GA8 names and, for every branch, asserts
//! the resulting EVENT SEQUENCE and PROJECTION STATE -- never console text (the GA8
//! Must-Not-Do). The seven branches:
//!
//! 1. continue at a safe boundary (also exercises the `safety-gates` workspace lock
//!    and a real checkpoint boundary in the continue path);
//! 2. pause when user input is queued (a boundary is unsafe);
//! 3. block on a raised blocker;
//! 4. budget-limit on budget exhaustion (durable `run.aborted`);
//! 5. no-progress suppression after a no-material-progress continuation;
//! 6. premature-completion-blocked (only an agent claim exists);
//! 7. complete-with-evidence (concrete observed evidence + validation present),
//!    then generate a historical report and snapshot it.
//!
//! The historical report is rendered controller-side from the persisted goal
//! projections so it is rebuildable from events + projections (GO10) and is
//! asserted against an exact golden string -- the GA8 "snapshot it" requirement.

#![cfg(test)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};
use capo_core::{
    AgentId, EvidenceId, GoalId, ProjectId, RequirementId, RunId, SessionId, TaskId, TurnId,
};
use capo_state::{
    AgentProjection, EventKind, EvidenceProjection, GoalAuditDecisionProjection, GoalProjection,
    GoalReportProjection, NewEvent, ProjectionRecord, RequirementLedgerProjection, RunProjection,
    SessionProjection, TaskProjection,
};

use crate::{
    CheckpointScope, ContinuationConditions, ContinuationDecision, FakeBoundaryController,
    FakeRunRefs, GoalBudget, RunResourceCeiling, RunResourceUsage, WorkspaceLeaseScope,
    WorkspaceWriteLeaseOutcome,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("capo-ga8-{name}-{nanos}-{n}"))
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
fn open() -> (FakeBoundaryController, PathBuf) {
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

    // Generate the historical report from persisted projections and snapshot it.
    let report = render_historical_report(&controller, &GoalId::new(GOAL));
    let expected = "\
# Goal Report: goal-ga8
Objective: Land the GA8 end-to-end
Status: active
Verdict: complete (all_requirements_met) 1/1 requirements complete

## Requirements
- req-1: validated [source=runtime_output]

## Observed Evidence
- evidence-check-1 (kind=test, confidence=95)

## Continuation Decisions
(none)
";
    assert_eq!(
        report, expected,
        "the historical report rebuilds from projections to an exact snapshot"
    );

    // The report is REBUILDABLE: re-render after a projection rebuild yields the
    // identical snapshot (GO10 "rebuildable from events, projections, artifacts").
    controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    let rebuilt = render_historical_report(&controller, &GoalId::new(GOAL));
    assert_eq!(rebuilt, expected, "the report rebuilds identically");
}

// ===================== Controller-side historical report =====================

/// GA8 (GO10): render a deterministic historical execution report for a goal from
/// the PERSISTED projections (goal, requirement ledger, observed evidence, audit
/// verdict, continuation decisions). It is rebuildable from events + projections --
/// re-render after `rebuild_projections` yields the same bytes -- and carries only
/// projected read-model facts, never inlined raw artifact bodies. This is the
/// controller-side report the deterministic e2e snapshots; the GA2 server renderer
/// is the operator-facing surface, but the controller crate does not depend on the
/// server, so the e2e renders from the same projections directly.
fn render_historical_report(controller: &FakeBoundaryController, goal_id: &GoalId) -> String {
    let state = controller.state();
    let goal = state.goal(goal_id).expect("goal").expect("goal present");

    let mut out = String::new();
    let _ = writeln!(out, "# Goal Report: {}", goal.goal_id.as_str());
    let _ = writeln!(out, "Objective: {}", goal.objective);
    let _ = writeln!(out, "Status: {}", goal.status);

    match state
        .latest_goal_audit_decision(goal_id)
        .expect("latest audit")
    {
        Some(audit) => {
            let _ = writeln!(
                out,
                "Verdict: {} ({}) {}/{} requirements complete",
                audit.verdict, audit.reason, audit.requirements_complete, audit.requirements_total
            );
        }
        None => {
            let _ = writeln!(out, "Verdict: (not yet audited)");
        }
    }

    let requirements = state
        .requirement_ledgers_for_goal(goal_id)
        .expect("requirements");
    let _ = writeln!(out, "\n## Requirements");
    if requirements.is_empty() {
        let _ = writeln!(out, "(none)");
    } else {
        for requirement in &requirements {
            let _ = writeln!(
                out,
                "- {}: {} [source={}]",
                requirement.requirement_id.as_str(),
                requirement.status,
                requirement.last_status_source
            );
        }
    }

    // Observed evidence by the goal's stable task key (cross-attempt), never inlined
    // raw bodies -- only the projected id/kind/confidence.
    let evidence: Vec<EvidenceProjection> = match goal.task_id.as_ref() {
        Some(task_id) => state.evidence_for_task(task_id).expect("task evidence"),
        None => Vec::new(),
    };
    let _ = writeln!(out, "\n## Observed Evidence");
    if evidence.is_empty() {
        let _ = writeln!(out, "(none)");
    } else {
        for row in &evidence {
            let _ = writeln!(
                out,
                "- {} (kind={}, confidence={})",
                row.evidence_id.as_str(),
                row.kind,
                row.confidence
            );
        }
    }

    let continuations = state
        .goal_continuations_for_goal(goal_id)
        .expect("continuations");
    let _ = writeln!(out, "\n## Continuation Decisions");
    if continuations.is_empty() {
        let _ = writeln!(out, "(none)");
    } else {
        for row in &continuations {
            let _ = writeln!(out, "- {} ({})", row.decision, row.reason);
        }
    }

    out
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
