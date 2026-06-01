use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn prototype_state_backend_is_sqlite() {
    assert_eq!(PROTOTYPE_STATE_BACKEND, "sqlite");
}

#[test]
fn fake_store_reports_state_boundary() {
    assert_eq!(StateStore::fake().binding().kind, BoundaryKind::StateStore);
}

#[test]
fn sqlite_store_persists_events_and_core_projections() {
    let store = temp_store("core-projections");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-p2");
    let agent_id = AgentId::new("agent-fake");
    let session_id = SessionId::new("session-fake");
    let run_id = RunId::new("run-fake");

    let sequence = store
        .append_event(
            NewEvent {
                event_id: "event-1".to_string(),
                kind: EventKind::SessionStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{\"kind\":\"session.started\"}".to_string(),
                idempotency_key: Some("session-started:test".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Project(ProjectProjection {
                    project_id: project_id.clone(),
                    name: "Capo".to_string(),
                    status: "active".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Task(TaskProjection {
                    task_id: task_id.clone(),
                    project_id: project_id.clone(),
                    title: "P2".to_string(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(session_id.clone()),
                    latest_summary: Some("state scaffold".to_string()),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: agent_id.clone(),
                    project_id: project_id.clone(),
                    name: "fake".to_string(),
                    status: "active".to_string(),
                    current_session_id: Some(session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id,
                    title: "Fake session".to_string(),
                    status: "starting".to_string(),
                    current_goal: "prove state".to_string(),
                    latest_summary: Some("booting".to_string()),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    external_session_ref: Some("adapter-session-fake".to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id,
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append event");

    assert_eq!(sequence, 1);
    assert_eq!(store.event_count().unwrap(), 1);
    assert_eq!(store.watermark("default").unwrap(), Some(1));
    let session = store.session(&session_id).unwrap().expect("session");
    assert_eq!(session.current_goal, "prove state");
    assert_eq!(session.latest_confidence, Some(70));
    assert_eq!(
        session.external_session_ref.as_deref(),
        Some("adapter-session-fake")
    );
    let task = store.task(&task_id).unwrap().expect("task");
    assert_eq!(task.latest_summary.as_deref(), Some("state scaffold"));

    // external_session_ref rides in payload_json, so confirm it survives a
    // full projection rebuild from the persisted projection records.
    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .session(&session_id)
        .unwrap()
        .expect("rebuilt session");
    assert_eq!(
        rebuilt.external_session_ref.as_deref(),
        Some("adapter-session-fake")
    );
}

#[test]
fn source_binding_projection_is_persisted_and_rebuilt() {
    let store = temp_store("source-binding-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-source-binding");

    store
        .append_event(
            NewEvent {
                event_id: "event-source-binding".to_string(),
                kind: EventKind::WorkpadTaskImported,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("source-binding-task-source-binding".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::SourceBinding(SourceBindingProjection {
                source_binding_id: "source-binding-task-source-binding".to_string(),
                project_id: project_id.clone(),
                task_id: task_id.clone(),
                source_kind: "markdown".to_string(),
                source_task_id: "workpads:scaffold:tasks.md#s5".to_string(),
                source_path: "workpads/scaffold/tasks.md".to_string(),
                source_anchor: "S5 - Explicit Source Binding Projection".to_string(),
                source_hash: "hash-source-binding".to_string(),
                binding_status: "active".to_string(),
                updated_sequence: 0,
            })],
        )
        .expect("append source binding");

    let binding = store
        .source_binding_for_task(&task_id)
        .expect("query source binding")
        .expect("source binding");
    assert_eq!(binding.source_kind, "markdown");
    assert_eq!(binding.source_task_id, "workpads:scaffold:tasks.md#s5");
    assert_eq!(binding.source_hash, "hash-source-binding");
    assert_eq!(binding.binding_status, "active");
    assert_eq!(
        store.source_bindings(&project_id).unwrap(),
        vec![binding.clone()]
    );

    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        store
            .source_binding_for_task(&task_id)
            .expect("query rebuilt source binding"),
        Some(binding)
    );
}

// GA1 (goal-orchestration GO1/GO3): the goal-domain projections must project
// in-transaction like every other projection and rebuild byte-identically from
// the persisted projection records. These tests prove the full encode -> row ->
// decode -> apply round-trip for the goal lifecycle, requirement ledger, agent
// report ledger, continuation decision, and observed delegated-provider state,
// plus that a duplicate report submission is deduped by its idempotency key.
#[test]
fn goal_projections_are_persisted_and_rebuild_identically() {
    let store = temp_store("goal-projections-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-goal");
    let agent_id = AgentId::new("agent-goal");
    let session_id = SessionId::new("session-goal");
    let run_id = RunId::new("run-goal-attempt");
    let goal_id = GoalId::new("goal-ship-feature");
    let requirement_id = RequirementId::new("req-tests-pass");
    let evidence_id = EvidenceId::new("evidence-check-output");

    let goal = GoalProjection {
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        task_id: Some(task_id.clone()),
        agent_id: Some(agent_id.clone()),
        session_id: Some(session_id.clone()),
        parent_goal_id: None,
        attempt_run_id: Some(run_id.clone()),
        objective: "Ship the goal-domain projections".to_string(),
        status: GoalProjection::ACTIVE.to_string(),
        success_criteria_json: "{\"criteria\":[\"tests pass\"]}".to_string(),
        constraints_json: "{\"no_network\":true}".to_string(),
        verification_surface_json: "{\"surface\":\"cargo test\"}".to_string(),
        budget_json: "{\"max_runs\":3}".to_string(),
        stop_conditions_json: "{\"on\":\"budget\"}".to_string(),
        blocker_reason: String::new(),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-goal-created".to_string(),
                kind: EventKind::GoalCreated,
                actor: "controller".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some(goal_id.to_string()),
                payload_json: "{\"kind\":\"goal.created\"}".to_string(),
                idempotency_key: Some("goal.created:goal-ship-feature".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Goal(goal.clone())],
        )
        .expect("append goal created");

    let requirement = RequirementLedgerProjection {
        requirement_id: requirement_id.clone(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        summary: "All tests pass".to_string(),
        status: RequirementLedgerProjection::SUPPORTED.to_string(),
        last_status_source: "runtime_output".to_string(),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-requirement-status".to_string(),
                kind: EventKind::RequirementStatusChanged,
                actor: "auditor".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some(requirement_id.to_string()),
                payload_json: "{\"kind\":\"goal.requirement_status_changed\"}".to_string(),
                idempotency_key: Some(
                    "goal.requirement_status_changed:req-tests-pass:supported".to_string(),
                ),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::RequirementLedger(requirement.clone())],
        )
        .expect("append requirement status");

    // An agent-reported completion CLAIM: stored as source=agent_reported with
    // confidence, never as observed evidence (GA1 acceptance / knowledge.md).
    let claim_report = GoalReportProjection {
        goal_report_id: "report-claim-complete".to_string(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        session_id: Some(session_id.clone()),
        requirement_id: Some(requirement_id.clone()),
        report_kind: "capo.complete_requirement".to_string(),
        source: "agent_reported".to_string(),
        confidence: Some(80),
        summary: "Agent claims the requirement is done".to_string(),
        body_artifact_id: Some("artifact-claim-body".to_string()),
        evidence_id: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-report-claim".to_string(),
                kind: EventKind::GoalReportRecorded,
                actor: "agent-goal".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some("report-claim-complete".to_string()),
                payload_json: "{\"kind\":\"goal.report_recorded\"}".to_string(),
                idempotency_key: Some("goal.report_recorded:report-claim-complete".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::GoalReport(claim_report.clone())],
        )
        .expect("append claim report");

    // An OBSERVED evidence report: source=runtime_output, no confidence, cites the
    // reused EvidenceRecorded row rather than restating it.
    let observed_report = GoalReportProjection {
        goal_report_id: "report-observed-check".to_string(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        session_id: Some(session_id.clone()),
        requirement_id: Some(requirement_id.clone()),
        report_kind: "runtime_output".to_string(),
        source: "runtime_output".to_string(),
        confidence: None,
        summary: "Observed check output".to_string(),
        body_artifact_id: None,
        evidence_id: Some(evidence_id.clone()),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-report-observed".to_string(),
                kind: EventKind::GoalReportRecorded,
                actor: "runtime".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some("report-observed-check".to_string()),
                payload_json: "{\"kind\":\"goal.report_recorded\"}".to_string(),
                idempotency_key: Some("goal.report_recorded:report-observed-check".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::GoalReport(observed_report.clone())],
        )
        .expect("append observed report");

    let continuation = GoalContinuationProjection {
        continuation_id: "continuation-1".to_string(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        attempt_run_id: Some(run_id.clone()),
        decision: "pause".to_string(),
        reason: "input_queued".to_string(),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-continuation".to_string(),
                kind: EventKind::ContinuationDecisionRecorded,
                actor: "scheduler".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some("continuation-1".to_string()),
                payload_json: "{\"kind\":\"goal.continuation_decision_recorded\"}".to_string(),
                idempotency_key: Some(
                    "goal.continuation_decision_recorded:continuation-1".to_string(),
                ),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::GoalContinuation(continuation.clone())],
        )
        .expect("append continuation decision");

    let delegated = DelegatedProviderGoalProjection {
        delegated_goal_id: "delegated-codex-1".to_string(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        session_id: Some(session_id.clone()),
        provider_kind: "codex".to_string(),
        provider_goal_ref: Some("codex-goal-abc".to_string()),
        provider_state: "in_progress".to_string(),
        source: "agent_reported".to_string(),
        body_artifact_id: Some("artifact-codex-goal".to_string()),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent {
                event_id: "event-delegated".to_string(),
                kind: EventKind::DelegatedProviderGoalObserved,
                actor: "adapter".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some("delegated-codex-1".to_string()),
                payload_json: "{\"kind\":\"goal.delegated_provider_observed\"}".to_string(),
                idempotency_key: Some(
                    "goal.delegated_provider_observed:delegated-codex-1".to_string(),
                ),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::DelegatedProviderGoal(delegated.clone())],
        )
        .expect("append delegated provider goal");

    // Read the live projections back (the sequence-stamped read model).
    let read_goal = store.goal(&goal_id).unwrap().expect("goal");
    assert!(read_goal.is_active());
    assert_eq!(read_goal.objective, goal.objective);
    assert_eq!(read_goal.attempt_run_id.as_ref(), Some(&run_id));
    assert_eq!(read_goal.budget_json, goal.budget_json);

    let read_requirements = store.requirement_ledgers_for_goal(&goal_id).unwrap();
    assert_eq!(read_requirements.len(), 1);
    assert_eq!(read_requirements[0].status, "supported");
    assert_eq!(read_requirements[0].last_status_source, "runtime_output");

    let read_reports = store.goal_reports_for_goal(&goal_id).unwrap();
    assert_eq!(read_reports.len(), 2);
    let claim = read_reports
        .iter()
        .find(|report| report.goal_report_id == "report-claim-complete")
        .expect("claim report");
    assert!(claim.is_agent_reported());
    assert!(!claim.is_observed_evidence());
    assert_eq!(claim.confidence, Some(80));
    let observed = read_reports
        .iter()
        .find(|report| report.goal_report_id == "report-observed-check")
        .expect("observed report");
    assert!(observed.is_observed_evidence());
    assert!(!observed.is_agent_reported());
    assert_eq!(observed.confidence, None);
    assert_eq!(observed.evidence_id.as_ref(), Some(&evidence_id));

    let read_continuations = store.goal_continuations_for_goal(&goal_id).unwrap();
    assert_eq!(read_continuations.len(), 1);
    assert_eq!(read_continuations[0].decision, "pause");
    assert_eq!(read_continuations[0].reason, "input_queued");

    let read_delegated = store.delegated_provider_goals_for_goal(&goal_id).unwrap();
    assert_eq!(read_delegated.len(), 1);
    assert_eq!(read_delegated[0].provider_kind, "codex");
    assert_eq!(read_delegated[0].provider_state, "in_progress");

    // The load-bearing property: every goal projection rebuilds IDENTICALLY from
    // the persisted projection records (full encode/decode/apply round-trip).
    let goal_before = read_goal.clone();
    let requirements_before = read_requirements.clone();
    let reports_before = read_reports.clone();
    let continuations_before = read_continuations.clone();
    let delegated_before = read_delegated.clone();

    store.rebuild_projections().expect("rebuild projections");

    assert_eq!(store.goal(&goal_id).unwrap(), Some(goal_before));
    assert_eq!(
        store.requirement_ledgers_for_goal(&goal_id).unwrap(),
        requirements_before
    );
    assert_eq!(
        store.goal_reports_for_goal(&goal_id).unwrap(),
        reports_before
    );
    assert_eq!(
        store.goal_continuations_for_goal(&goal_id).unwrap(),
        continuations_before
    );
    assert_eq!(
        store.delegated_provider_goals_for_goal(&goal_id).unwrap(),
        delegated_before
    );
}

#[test]
fn duplicate_goal_report_submission_is_idempotent() {
    let store = temp_store("goal-report-idempotent");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-goal-idem");
    let goal_id = GoalId::new("goal-idem");

    // Seed the goal so the report references a real goal.
    store
        .append_event(
            NewEvent {
                event_id: "event-goal-idem".to_string(),
                kind: EventKind::GoalCreated,
                actor: "controller".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some(goal_id.to_string()),
                payload_json: "{\"kind\":\"goal.created\"}".to_string(),
                idempotency_key: Some("goal.created:goal-idem".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Goal(GoalProjection {
                goal_id: goal_id.clone(),
                project_id: project_id.clone(),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                parent_goal_id: None,
                attempt_run_id: None,
                objective: "Idempotency".to_string(),
                status: GoalProjection::ACTIVE.to_string(),
                success_criteria_json: "{}".to_string(),
                constraints_json: "{}".to_string(),
                verification_surface_json: "{}".to_string(),
                budget_json: "{}".to_string(),
                stop_conditions_json: "{}".to_string(),
                blocker_reason: String::new(),
                updated_sequence: 0,
            })],
        )
        .expect("append goal");

    let report = GoalReportProjection {
        goal_report_id: "report-dup".to_string(),
        goal_id: goal_id.clone(),
        project_id: project_id.clone(),
        session_id: Some(session_id.clone()),
        requirement_id: None,
        report_kind: "capo.report_progress".to_string(),
        source: "agent_reported".to_string(),
        confidence: Some(55),
        summary: "Progress report".to_string(),
        body_artifact_id: None,
        evidence_id: None,
        updated_sequence: 0,
    };
    let report_event = |event_id: &str| NewEvent {
        event_id: event_id.to_string(),
        kind: EventKind::GoalReportRecorded,
        actor: "agent".to_string(),
        project_id: Some(project_id.clone()),
        task_id: None,
        agent_id: None,
        session_id: Some(session_id.clone()),
        run_id: None,
        turn_id: None,
        item_id: Some("report-dup".to_string()),
        payload_json: "{\"kind\":\"goal.report_recorded\"}".to_string(),
        // Same idempotency key for both submissions: a replayed/duplicated report
        // must dedupe rather than double-project.
        idempotency_key: Some("goal.report_recorded:report-dup".to_string()),
        redaction_state: RedactionState::Safe,
    };

    let first = store
        .append_event(
            report_event("event-report-first"),
            &[ProjectionRecord::GoalReport(report.clone())],
        )
        .expect("append first report");
    // The duplicate carries a DIFFERENT summary/confidence under the SAME
    // idempotency key. Genuine event-layer dedup must drop the second event
    // outright (first write wins); a benign upsert that re-projected would
    // instead overwrite the row with these new values (last-write-wins). The
    // differing payload is what distinguishes the two.
    let duplicate_report = GoalReportProjection {
        confidence: Some(99),
        summary: "Overwritten progress report".to_string(),
        ..report.clone()
    };
    let second = store
        .append_event(
            report_event("event-report-duplicate"),
            &[ProjectionRecord::GoalReport(duplicate_report)],
        )
        .expect("append duplicate report");

    // The duplicate returns the original sequence and does not append a new
    // event: goal-created + the first report = exactly 2 events, the duplicate
    // must not increment.
    assert_eq!(first, second);
    assert_eq!(
        store.event_count().unwrap(),
        2,
        "duplicate report must not append a new event"
    );
    let reports = store.goal_reports_for_goal(&goal_id).unwrap();
    assert_eq!(reports.len(), 1, "duplicate report must not double-project");
    // The FIRST write wins (dedup), so the original summary/confidence persist
    // rather than the duplicate's overwriting values.
    assert_eq!(reports[0].summary, "Progress report");
    assert_eq!(reports[0].confidence, Some(55));

    // A rebuild from the deduped log keeps exactly one report row with the
    // first write's values.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.goal_reports_for_goal(&goal_id).unwrap(), reports);
}

// GA1: the goal lifecycle transitions (blocked/resumed/cleared) and the
// `blocker_reason` current-blocker field must project last-write-wins and
// rebuild byte-identically. The happy-path test above only ever projects an
// ACTIVE goal with an empty blocker_reason, so this exercises the
// non-`active` statuses, the `is_active() == false` path, and the
// blocker_reason column that a lifecycle write carries.
#[test]
fn goal_lifecycle_transitions_project_and_rebuild_identically() {
    let store = temp_store("goal-lifecycle-transitions");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-goal-lifecycle");
    let goal_id = GoalId::new("goal-lifecycle");

    // A small helper that projects the goal at a given lifecycle status with a
    // given blocker_reason, under a status-scoped idempotency key.
    let append_status = |event_id: &str, kind: EventKind, status: &str, blocker_reason: &str| {
        store
            .append_event(
                NewEvent {
                    event_id: event_id.to_string(),
                    kind,
                    actor: "controller".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: None,
                    item_id: Some(goal_id.to_string()),
                    payload_json: format!(
                        "{{\"kind\":\"{}\",\"blocker_reason\":{}}}",
                        kind.as_str(),
                        serde_json::to_string(blocker_reason).unwrap()
                    ),
                    idempotency_key: Some(format!("{}:goal-lifecycle", kind.as_str())),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Goal(GoalProjection {
                    goal_id: goal_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    parent_goal_id: None,
                    attempt_run_id: None,
                    objective: "Lifecycle goal".to_string(),
                    status: status.to_string(),
                    success_criteria_json: "{}".to_string(),
                    constraints_json: "{}".to_string(),
                    verification_surface_json: "{}".to_string(),
                    budget_json: "{}".to_string(),
                    stop_conditions_json: "{}".to_string(),
                    blocker_reason: blocker_reason.to_string(),
                    updated_sequence: 0,
                })],
            )
            .expect("append lifecycle transition")
    };

    append_status(
        "event-goal-created",
        EventKind::GoalCreated,
        GoalProjection::ACTIVE,
        "",
    );
    // Blocked carries a non-empty blocker_reason (the GO3 current-blocker state).
    append_status(
        "event-goal-blocked",
        EventKind::GoalBlocked,
        GoalProjection::BLOCKED,
        "waiting on operator approval",
    );

    // After the block, the live read model reflects last-write-wins status and
    // the persisted blocker_reason, and the goal is no longer active.
    let blocked = store.goal(&goal_id).unwrap().expect("blocked goal");
    assert_eq!(blocked.status, GoalProjection::BLOCKED);
    assert!(!blocked.is_active());
    assert_eq!(blocked.blocker_reason, "waiting on operator approval");

    // Resuming clears the blocker_reason and restores active.
    append_status(
        "event-goal-resumed",
        EventKind::GoalResumed,
        GoalProjection::ACTIVE,
        "",
    );
    let resumed = store.goal(&goal_id).unwrap().expect("resumed goal");
    assert_eq!(resumed.status, GoalProjection::ACTIVE);
    assert!(resumed.is_active());
    assert_eq!(resumed.blocker_reason, "");

    // Clearing is terminal and is not active.
    append_status(
        "event-goal-cleared",
        EventKind::GoalCleared,
        GoalProjection::CLEARED,
        "",
    );
    let cleared = store.goal(&goal_id).unwrap().expect("cleared goal");
    assert_eq!(cleared.status, GoalProjection::CLEARED);
    assert!(!cleared.is_active());

    // The load-bearing property: the goal rebuilds IDENTICALLY from the durable
    // projection records, including the (now-empty) blocker_reason that rode the
    // lifecycle payloads.
    let cleared_before = cleared.clone();
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.goal(&goal_id).unwrap(), Some(cleared_before));
}

// GA1: pin the new goal-lifecycle EventKind wire strings so a typo in any of
// them is caught, mirroring the SG3 grant-lifecycle round-trip test. The
// persisted `EventRecord.kind` string is the durable contract, so `as_str` and
// `from_wire` must round-trip for every goal-lifecycle kind.
#[test]
fn ga1_goal_lifecycle_event_kinds_round_trip() {
    for (kind, wire) in [
        (EventKind::GoalCreated, "goal.created"),
        (EventKind::GoalUpdated, "goal.updated"),
        (EventKind::GoalPaused, "goal.paused"),
        (EventKind::GoalResumed, "goal.resumed"),
        (EventKind::GoalBlocked, "goal.blocked"),
        (EventKind::GoalCleared, "goal.cleared"),
        (
            EventKind::RequirementStatusChanged,
            "goal.requirement_status_changed",
        ),
        (EventKind::GoalReportRecorded, "goal.report_recorded"),
        (
            EventKind::ContinuationDecisionRecorded,
            "goal.continuation_decision_recorded",
        ),
        (
            EventKind::DelegatedProviderGoalObserved,
            "goal.delegated_provider_observed",
        ),
    ] {
        assert_eq!(kind.as_str(), wire);
        assert_eq!(EventKind::from_wire(wire), Some(kind));
    }
}

#[test]
fn tool_observations_are_persisted_and_rebuilt() {
    let store = temp_store("tool-observations");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-tools-observed");
    let tool_call_id = ToolCallId::new("tool-adapter-native");

    store
        .append_event(
            NewEvent {
                event_id: "event-tool-observation".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "adapter-replay".to_string(),
                project_id: Some(project_id),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some("tool-1".to_string()),
                payload_json: "{\"kind\":\"tool.observation_recorded\"}".to_string(),
                idempotency_key: Some("tool-observation:tool-1:completed".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: "observation-tool-1-completed".to_string(),
                    session_id: session_id.clone(),
                    tool_call_id: Some(tool_call_id.clone()),
                    source: "adapter_event".to_string(),
                    external_tool_ref: Some("tool-1".to_string()),
                    tool_name: "exec_command".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "observed_only".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "fnv1a64:testhash".to_string(),
                    artifact_id: Some("artifact-adapter-output".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append observation");

    let observations = store
        .tool_observations_for_session(&session_id)
        .expect("read observations");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].tool_call_id, Some(tool_call_id));
    assert_eq!(observations[0].instrumentation_level, "observed_only");
    assert_eq!(observations[0].confidence, "high");
    assert_eq!(
        observations[0].artifact_id.as_deref(),
        Some("artifact-adapter-output")
    );

    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .tool_observations_for_session(&session_id)
        .expect("read rebuilt observations");
    assert_eq!(rebuilt, observations);
}

/// ACI8: an agent-reported observation (`source=agent_reported`, carrying
/// confidence) is persisted as a DISTINCT class from observed evidence
/// (`source=runtime_output` / `adapter_event`); the classification survives
/// replay and a duplicate report submission (same idempotency key) dedupes.
#[test]
fn agent_reported_observations_are_distinct_from_observed_and_dedupe_on_replay() {
    let store = temp_store("agent-reported-observations");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-aci8-state");
    let report_call = ToolCallId::new("tool-agent-report");
    let observed_call = ToolCallId::new("tool-runtime-observed");

    let append_report = |event_suffix: &str| {
        store
            .append_event(
                NewEvent {
                    event_id: format!("event-agent-report-{event_suffix}"),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "agent-report".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: None,
                    item_id: Some(report_call.to_string()),
                    payload_json: "{\"source\":\"agent_reported\"}".to_string(),
                    // The idempotency key duplicate submissions dedupe on.
                    idempotency_key: Some("agent-report:sub-1".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "agent-report-obs-1".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(report_call.clone()),
                        source: "agent_reported".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.complete_requirement".to_string(),
                        observed_status: "reported".to_string(),
                        instrumentation_level: "structured_observed".to_string(),
                        confidence: "80".to_string(),
                        raw_event_hash: "agent-report:tool-agent-report".to_string(),
                        artifact_id: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append agent report")
    };

    // The agent report (a CLAIM) ...
    append_report("first");
    // ... and an OBSERVED runtime-evidence observation, distinct class.
    store
        .append_event(
            NewEvent {
                event_id: "event-runtime-observed".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "runtime".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some(observed_call.to_string()),
                payload_json: "{\"source\":\"runtime_output\"}".to_string(),
                idempotency_key: Some("runtime-observed:tool-runtime-observed".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: "runtime-observed-obs-1".to_string(),
                    session_id: session_id.clone(),
                    tool_call_id: Some(observed_call.clone()),
                    source: "runtime_output".to_string(),
                    external_tool_ref: None,
                    tool_name: "capo.test_run".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "full".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "fnv1a64:runtimehash".to_string(),
                    artifact_id: Some("artifact-test-output".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append runtime observation");

    // A DUPLICATE report submission (same idempotency key) dedupes: no second row.
    append_report("duplicate");

    let observations = store
        .tool_observations_for_session(&session_id)
        .expect("read observations");
    assert_eq!(
        observations.len(),
        2,
        "the duplicate agent report must dedupe; only the report + the observed row remain"
    );

    let report = observations
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&report_call))
        .expect("agent report observation");
    let observed = observations
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&observed_call))
        .expect("runtime observed observation");

    // The two are a DISTINCT class: the agent report is `agent_reported`, the
    // runtime evidence is `runtime_output`. Completion is never reachable by the
    // agent claim alone because the two never share a source classification.
    assert_eq!(report.source, "agent_reported");
    assert_eq!(report.confidence, "80");
    assert_eq!(observed.source, "runtime_output");
    assert_ne!(report.source, observed.source);

    // The classification survives a restart/replay.
    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .tool_observations_for_session(&session_id)
        .expect("read rebuilt observations");
    assert_eq!(
        rebuilt, observations,
        "classification must replay identically"
    );
}

#[test]
fn append_event_is_idempotent_for_project_scoped_keys() {
    let store = temp_store("idempotency");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-idempotent");

    let first = store
        .append_event(
            NewEvent {
                event_id: "event-idempotent-1".to_string(),
                kind: EventKind::TaskDiscovered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("task:discover:one".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Task(TaskProjection {
                task_id: task_id.clone(),
                project_id: project_id.clone(),
                title: "first".to_string(),
                capo_execution_status: "pending".to_string(),
                active_session_id: None,
                latest_summary: Some("first".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("append first");

    let second = store
        .append_event(
            NewEvent {
                event_id: "event-idempotent-2".to_string(),
                kind: EventKind::TaskDiscovered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("task:discover:one".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Task(TaskProjection {
                task_id: task_id.clone(),
                project_id,
                title: "second".to_string(),
                capo_execution_status: "active".to_string(),
                active_session_id: None,
                latest_summary: Some("second".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("append duplicate");

    assert_eq!(first, second);
    assert_eq!(store.event_count().unwrap(), 1);
    assert_eq!(
        store
            .task(&task_id)
            .unwrap()
            .expect("task")
            .latest_summary
            .as_deref(),
        Some("first")
    );
}

#[test]
fn recovery_marks_active_looking_runs_exited_unknown_once() {
    let store = temp_store("active-run-recovery");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-running");
    let run_id = RunId::new("run-running");

    store
        .append_event(
            NewEvent {
                event_id: "event-run-started".to_string(),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("run:start".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-running"),
                    title: "Running session".to_string(),
                    status: "active".to_string(),
                    current_goal: "recover active run".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");

    assert_eq!(store.active_looking_runs().unwrap().len(), 1);
    let recovered = store
        .mark_active_runs_exited_unknown(&project_id, "recovery-1")
        .expect("recover active runs");
    let recovered_again = store
        .mark_active_runs_exited_unknown(&project_id, "recovery-1")
        .expect("recover active runs idempotently");

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered_again.len(), 0);
    assert_eq!(
        store.run(&run_id).unwrap().expect("run").status,
        "exited_unknown"
    );
    assert_eq!(store.active_looking_runs().unwrap().len(), 0);
    assert_eq!(store.event_count().unwrap(), 2);
}

#[test]
fn run_aborted_event_projects_aborted_status_and_rebuilds_identically() {
    // RTL7: the `run.aborted` event (emitted when a per-run resource ceiling is
    // exceeded) carries a `Run` projection of status `aborted`, has an
    // idempotency key, and rebuilds identically from the event log.
    let store = temp_store("run-aborted-projection");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-ceiling");
    let run_id = RunId::new("run-ceiling");

    store
        .append_event(
            NewEvent {
                event_id: "event-run-started".to_string(),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("run:start".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-ceiling"),
                    title: "Ceiling session".to_string(),
                    status: "active".to_string(),
                    current_goal: "run under a ceiling".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");

    let aborted_event = NewEvent {
        event_id: "event-run-aborted".to_string(),
        kind: EventKind::RunAborted,
        actor: "capo-controller".to_string(),
        project_id: Some(project_id.clone()),
        task_id: None,
        agent_id: None,
        session_id: Some(session_id.clone()),
        run_id: Some(run_id.clone()),
        turn_id: Some("turn-2".to_string()),
        item_id: Some(run_id.to_string()),
        payload_json: "{\"reason_code\":\"max_turns_exceeded\",\"status\":\"aborted\"}".to_string(),
        idempotency_key: Some(
            "run-aborted:project-capo:run-ceiling:max_turns_exceeded".to_string(),
        ),
        redaction_state: RedactionState::Safe,
    };
    let aborted_projection = RunProjection {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        status: "aborted".to_string(),
        recovery_of_run_id: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            aborted_event.clone(),
            &[ProjectionRecord::Run(aborted_projection.clone())],
        )
        .expect("abort run");

    assert_eq!(EventKind::RunAborted.as_str(), "run.aborted");
    assert_eq!(store.run(&run_id).unwrap().expect("run").status, "aborted");
    // An aborted run is not active-looking, so recovery never resurrects it.
    assert!(store.active_looking_runs().unwrap().is_empty());
    assert_eq!(store.event_count().unwrap(), 2);

    // Idempotent: re-appending the same abort appends nothing and the run stays
    // aborted.
    store
        .append_event(aborted_event, &[ProjectionRecord::Run(aborted_projection)])
        .expect("re-abort run idempotently");
    assert_eq!(store.event_count().unwrap(), 2);

    // Rebuild from the event log: the run is still aborted.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.run(&run_id).unwrap().expect("run").status, "aborted");
}

#[test]
fn inflight_runs_carry_the_persisted_pid_marker() {
    // RTL10: the in-flight marker (a `run.started` event carrying `external_pid`
    // + the process-group reference, persisted before the spawn returned) is
    // what the orphan reaper reads to recover a crashed run.
    let store = temp_store("inflight-marker");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-inflight");
    let run_id = RunId::new("run-inflight");

    start_running_run(&store, &project_id, &session_id, &run_id);
    // Persist the in-flight pid marker as the live spawn path does.
    store
        .append_event(
            NewEvent {
                event_id: "event-run-started-inflight".to_string(),
                kind: EventKind::RunStarted,
                actor: "capo-server".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("local-process-run-inflight".to_string()),
                payload_json: "{\"status\":\"running\",\"runtime_process_ref\":\"local-process-run-inflight\",\"external_pid\":4242,\"boot_id\":\"linux-btime-1700000000\",\"marker\":\"start_requested_inflight\"}".to_string(),
                idempotency_key: Some("server-run-started-inflight:run-inflight:4242".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("persist in-flight marker");

    let inflight = store.inflight_runs_for_project(&project_id).unwrap();
    assert_eq!(inflight.len(), 1);
    assert_eq!(inflight[0].run_id, run_id);
    assert_eq!(inflight[0].external_pid, Some(4242));
    // The persisted boot id is read back so restart recovery can refuse to reap
    // a recycled PID across a reboot.
    assert_eq!(
        inflight[0].boot_id.as_deref(),
        Some("linux-btime-1700000000")
    );
    assert_eq!(
        inflight[0].runtime_process_ref.as_deref(),
        Some("local-process-run-inflight")
    );
}

#[test]
fn inflight_runs_treat_a_zero_pid_marker_as_no_process() {
    // RTL10 safety: a zero `external_pid` in a marker (e.g. from a defaulted
    // payload) is not a real process group target -- `kill -<0>` would hit the
    // caller's own group -- so it reads back as "no process to reap".
    let store = temp_store("inflight-zero-pid");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-zero");
    let run_id = RunId::new("run-zero");

    start_running_run(&store, &project_id, &session_id, &run_id);
    store
        .append_event(
            NewEvent {
                event_id: "event-run-started-inflight-zero".to_string(),
                kind: EventKind::RunStarted,
                actor: "capo-server".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("local-process-run-zero".to_string()),
                payload_json: "{\"status\":\"running\",\"runtime_process_ref\":\"local-process-run-zero\",\"external_pid\":0,\"marker\":\"start_requested_inflight\"}".to_string(),
                idempotency_key: Some("server-run-started-inflight:run-zero:0".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("persist zero-pid marker");

    let inflight = store.inflight_runs_for_project(&project_id).unwrap();
    assert_eq!(inflight.len(), 1);
    assert_eq!(
        inflight[0].external_pid, None,
        "a zero pid must not be a reapable process group target"
    );
}

#[test]
fn reap_orphaned_runs_records_orphan_and_exit_and_is_idempotent_across_restarts() {
    // RTL10: a restart mid-run reaps the orphaned process group and records the
    // outcome. A still-alive (now reaped) orphan records `run.orphaned`, a
    // terminal `run.exited`, and `run.recovered`; the run is no longer
    // active-looking; repeated restarts that observe the same runtime state
    // append nothing; and the recovered run rebuilds identically from the log.
    let store = temp_store("reap-orphaned");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-orphan");
    let run_id = RunId::new("run-orphan");

    start_running_run(&store, &project_id, &session_id, &run_id);
    assert_eq!(store.active_looking_runs().unwrap().len(), 1);

    let observation = RunReapObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunReapKind::AliveReaped,
        external_pid: Some(4242),
        observed_runtime_state_hash: "fnv1a64:deadbeefdeadbeef".to_string(),
    };

    let recovered = store
        .reap_orphaned_runs(
            &project_id,
            "recovery-1",
            std::slice::from_ref(&observation),
        )
        .expect("reap orphaned runs");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].status, "recovered");

    // orphaned -> exited -> recovered were all recorded for the reaped orphan.
    let events = store.recent_events_for_session(&session_id, 16).unwrap();
    let kinds: Vec<&str> = events.iter().map(|event| event.kind.as_str()).collect();
    assert!(kinds.contains(&"run.orphaned"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"run.exited"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"run.recovered"), "kinds: {kinds:?}");

    // The recovered run is terminal: recovery never resurrects it.
    assert!(store.active_looking_runs().unwrap().is_empty());
    let event_count_after_first = store.event_count().unwrap();

    // A repeated restart that observes the SAME runtime state appends nothing.
    let recovered_again = store
        .reap_orphaned_runs(
            &project_id,
            "recovery-2",
            std::slice::from_ref(&observation),
        )
        .expect("reap orphaned runs again");
    assert_eq!(recovered_again.len(), 1);
    assert_eq!(store.event_count().unwrap(), event_count_after_first);

    // Rebuild from the event log: the run is still recovered/terminal.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        store.run(&run_id).unwrap().expect("run").status,
        "recovered"
    );
    assert!(store.active_looking_runs().unwrap().is_empty());
}

#[test]
fn reap_orphaned_runs_records_exit_for_an_already_gone_run_without_orphan_event() {
    // A run whose process was already gone (no terminal event) reaches a
    // terminal `run.exited` directly -- it is never recorded as orphaned,
    // because no live process was found on restart.
    let store = temp_store("reap-already-gone");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-gone");
    let run_id = RunId::new("run-gone");

    start_running_run(&store, &project_id, &session_id, &run_id);

    let observation = RunReapObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunReapKind::AlreadyGone,
        external_pid: Some(9999),
        observed_runtime_state_hash: "fnv1a64:0000000000000001".to_string(),
    };
    store
        .reap_orphaned_runs(&project_id, "recovery-1", &[observation])
        .expect("reap already-gone run");

    let kinds: Vec<String> = store
        .recent_events_for_session(&session_id, 16)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert!(!kinds.iter().any(|kind| kind == "run.orphaned"));
    assert!(kinds.iter().any(|kind| kind == "run.exited"));
    assert!(kinds.iter().any(|kind| kind == "run.recovered"));
    assert!(store.active_looking_runs().unwrap().is_empty());
}

/// SG9: the LIVENESS-AWARE recovery classifies a still-alive run as REATTACHED
/// (a single `run.recovered`, NO `run.exited` -- the run keeps running), unlike
/// the reaper which always terminates and exits a live orphan. Reattach is
/// idempotent across repeated restarts and rebuilds identically from the log.
#[test]
fn recover_inflight_runs_reattaches_a_live_run_without_exiting_it() {
    let store = temp_store("recover-reattach");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-reattach");
    let run_id = RunId::new("run-reattach");

    start_running_run(&store, &project_id, &session_id, &run_id);
    assert_eq!(store.active_looking_runs().unwrap().len(), 1);

    let observation = RunRecoveryObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunRecoveryKind::Reattached,
        external_pid: Some(4242),
        runtime_process_ref: Some("fake-runtime-process-codex".to_string()),
        observed_runtime_state_hash: "fnv1a64:abc123abc123abc1".to_string(),
    };

    let recovered = store
        .recover_inflight_runs(
            &project_id,
            "recovery-1",
            std::slice::from_ref(&observation),
        )
        .expect("recover inflight runs");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].status, "recovered");

    // A reattach emits ONLY run.recovered -- the live process is NOT exited.
    let kinds: Vec<String> = store
        .recent_events_for_session(&session_id, 16)
        .unwrap()
        .into_iter()
        .filter(|event| event.actor == "capo-recovery")
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds, vec!["run.recovered".to_string()]);

    let event_count_after_first = store.event_count().unwrap();

    // Repeated restart observing the same runtime state appends nothing.
    store
        .recover_inflight_runs(
            &project_id,
            "recovery-2",
            std::slice::from_ref(&observation),
        )
        .expect("recover again");
    assert_eq!(store.event_count().unwrap(), event_count_after_first);

    // Rebuild from the event log: the reattached run reconstructs identically.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        store.run(&run_id).unwrap().expect("run").status,
        "recovered"
    );
}

/// SG9: a gone run is classified `Exited` (terminal `run.exited` then
/// `run.recovered`), NEVER the blunt `exited_unknown` the old
/// `mark_active_runs_exited_unknown` path stamped on every live-looking run.
#[test]
fn recover_inflight_runs_exits_a_gone_run_not_exited_unknown() {
    let store = temp_store("recover-exited");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-exited");
    let run_id = RunId::new("run-exited");

    start_running_run(&store, &project_id, &session_id, &run_id);

    let observation = RunRecoveryObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunRecoveryKind::Exited,
        external_pid: Some(9999),
        runtime_process_ref: None,
        observed_runtime_state_hash: "fnv1a64:0000000000000009".to_string(),
    };
    store
        .recover_inflight_runs(&project_id, "recovery-1", &[observation])
        .expect("recover gone run");

    let kinds: Vec<String> = store
        .recent_events_for_session(&session_id, 16)
        .unwrap()
        .into_iter()
        .filter(|event| event.actor == "capo-recovery")
        .map(|event| event.kind)
        .collect();
    assert_eq!(
        kinds,
        vec!["run.exited".to_string(), "run.recovered".to_string()]
    );
    // No event ever carries the blunt exited_unknown status payload.
    let any_exited_unknown = store
        .recent_events_for_session(&session_id, 16)
        .unwrap()
        .into_iter()
        .any(|event| event.payload_json.contains("exited_unknown"));
    assert!(
        !any_exited_unknown,
        "SG9 recovery never records the blunt exited_unknown status"
    );
    assert!(store.active_looking_runs().unwrap().is_empty());
}

fn start_running_run(
    store: &SqliteStateStore,
    project_id: &ProjectId,
    session_id: &SessionId,
    run_id: &RunId,
) {
    store
        .append_event(
            NewEvent {
                event_id: format!("event-run-started-{run_id}"),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some(format!("run:start:{run_id}")),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-orphan"),
                    title: "Orphan session".to_string(),
                    status: "active".to_string(),
                    current_goal: "recover an orphaned run".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");
}

#[test]
fn artifacts_tool_grants_memory_and_evidence_are_persisted_and_rebuilt() {
    let store = temp_store("artifact-rebuild");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-fake");
    let run_id = RunId::new("run-fake");
    let task_id = TaskId::new("task-p2");
    let artifact_id = "artifact-summary";

    store
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.to_string(),
            project_id: Some(project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "summary".to_string(),
            uri: "artifacts/raw/summary.md".to_string(),
            content_hash: "hash-summary".to_string(),
            size_bytes: 42,
            redaction_state: RedactionState::Redacted,
        })
        .expect("record artifact");

    store
        .append_event(
            NewEvent::new("event-2", EventKind::EvidenceRecorded, "test"),
            &[
                ProjectionRecord::CapabilityGrant(CapabilityGrantProjection {
                    capability_grant_id: "grant-local".to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"state:read:project\"]".to_string(),
                    effect: "allow".to_string(),
                    subject_json: "{\"agent\":\"fake\"}".to_string(),
                    decision_source: "allow_trusted_local_profile".to_string(),
                    persistence: "until_session_end".to_string(),
                    explanation: "test grant".to_string(),
                    created_at: None,
                    expires_at: None,
                    revoked_at: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: ToolCallId::new("tool-status"),
                    session_id: session_id.clone(),
                    turn_id: Some("turn-1".to_string()),
                    tool_name: "capo.session_summary".to_string(),
                    tool_origin: "capo".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: None,
                    output_artifact_id: Some(artifact_id.to_string()),
                    provenance: Default::default(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                    memory_packet_id: MemoryPacketId::new("packet-1"),
                    project_id: project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: Some("turn-1".to_string()),
                    packet_artifact_id: Some(artifact_id.to_string()),
                    purpose: "turn_context".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new("evidence-1"),
                    project_id,
                    task_id: Some(task_id),
                    session_id: Some(session_id),
                    run_id: Some(run_id),
                    kind: "summary".to_string(),
                    artifact_id: Some(artifact_id.to_string()),
                    confidence: 80,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append evidence event");

    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.watermark("default").unwrap(), Some(1));

    let connection = Connection::open(store.db_path()).unwrap();
    for (table, expected) in [
        ("artifacts", 1),
        ("capability_grants", 1),
        ("tool_calls", 1),
        ("memory_packet_refs", 1),
        ("evidence", 1),
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, expected, "{table}");
    }

    let grants = store.capability_grants().expect("read grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].decision_source, "allow_trusted_local_profile");
    assert_eq!(grants[0].persistence, "until_session_end");
    assert_eq!(grants[0].explanation, "test grant");
}

#[test]
fn tool_call_provenance_and_timing_persist_and_rebuild_identically() {
    // ACI7: the per-call provenance (correlation_id, permission_decision_id,
    // capability_grant_use_id) and wall-clock timing (started_at/completed_at)
    // are persisted on the `ToolCall` projection AND rebuild byte-identically on
    // replay, so provenance is queryable and survives a restart.
    let store = temp_store("tool-call-provenance-rebuild");
    let session_id = SessionId::new("session-prov");

    let provenance = ToolCallProvenance {
        correlation_id: Some("corr-session-prov-run-1-turn-1-tool-prov".to_string()),
        permission_decision_id: Some("decision-grant-allow-abc".to_string()),
        capability_grant_use_id: Some("grant-use-tool-prov-grant-allow-abc".to_string()),
        started_at: Some(1_700_000_000_123),
        completed_at: Some(1_700_000_000_456),
    };

    store
        .append_event(
            NewEvent::new("event-tool-prov", EventKind::ToolCallCompleted, "test"),
            &[ProjectionRecord::ToolCall(ToolCallProjection {
                tool_call_id: ToolCallId::new("tool-prov"),
                session_id: session_id.clone(),
                turn_id: Some("turn-1".to_string()),
                tool_name: "capo.file_read".to_string(),
                tool_origin: "runtime".to_string(),
                status: "completed".to_string(),
                input_artifact_id: Some("artifact-input".to_string()),
                output_artifact_id: Some("artifact-output".to_string()),
                provenance: provenance.clone(),
                updated_sequence: 0,
            })],
        )
        .expect("append tool call with provenance");

    // Provenance is queryable from the live projection.
    let before = store
        .tool_calls_for_session(&session_id)
        .expect("read tool calls");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].provenance, provenance);
    assert_eq!(before[0].provenance.started_at, Some(1_700_000_000_123));
    assert_eq!(before[0].provenance.completed_at, Some(1_700_000_000_456));

    // A restart/replay (rebuild from the event-sourced projection records)
    // reconstructs the exact same provenance and timing.
    store.rebuild_projections().expect("rebuild projections");
    let after = store
        .tool_calls_for_session(&session_id)
        .expect("read tool calls after rebuild");
    assert_eq!(after, before, "tool call must rebuild identically");
    assert_eq!(after[0].provenance, provenance);
}

/// ACI11: REOPEN the state store from disk (a true restart, not just an
/// in-process rebuild), rebuild projections from the event log, and assert the
/// tool-call, observation, AND agent-report projections rebuild IDENTICALLY,
/// and that an adapter-native tool update with a stable external id deduped on
/// append (`tool-exposure.md:352`).
///
/// This is the load-bearing replay-identity gate for ACI11: a fresh
/// `SqliteStateStore::open` over the same root sees only the persisted event
/// log, derives the read models from scratch, and yields byte-identical
/// projections across all three tool classes -- so a Capo restart loses
/// nothing and an adapter that re-sends the same `toolCallId` never doubles a
/// row.
#[test]
fn aci11_reopened_store_rebuilds_tool_call_observation_and_report_projections_identically() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("capo-state-aci11-reopen-{nanos}"));

    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-aci11-reopen");
    let tool_call = ToolCallId::new("tool-aci11-observed");
    let report_call = ToolCallId::new("tool-aci11-report");
    let adapter_call = ToolCallId::new("tool-aci11-adapter");

    // -- First "boot": write the three tool-projection classes + an
    //    adapter-native observation with a stable external id. --
    let (tools_before, observations_before, event_count_before) = {
        let store = SqliteStateStore::open(&root).expect("open state store");

        // 1) A tool-call (ToolInvocation) projection with provenance.
        store
            .append_event(
                NewEvent::new("event-aci11-call", EventKind::ToolCallCompleted, "runtime"),
                &[ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: tool_call.clone(),
                    session_id: session_id.clone(),
                    turn_id: Some("turn-aci11".to_string()),
                    tool_name: "capo.file_read".to_string(),
                    tool_origin: "runtime".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: Some("artifact-input".to_string()),
                    output_artifact_id: Some("artifact-output".to_string()),
                    provenance: ToolCallProvenance {
                        correlation_id: Some("corr-aci11".to_string()),
                        permission_decision_id: Some("decision-aci11".to_string()),
                        capability_grant_use_id: Some("grant-use-aci11".to_string()),
                        started_at: Some(1_700_000_000_001),
                        completed_at: Some(1_700_000_000_002),
                    },
                    updated_sequence: 0,
                })],
            )
            .expect("append tool call");

        // 2) An OBSERVED runtime-evidence observation for that call.
        store
            .append_event(
                NewEvent {
                    event_id: "event-aci11-observed".to_string(),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "runtime".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: Some("turn-aci11".to_string()),
                    item_id: Some(tool_call.to_string()),
                    payload_json: "{\"source\":\"runtime_output\"}".to_string(),
                    idempotency_key: Some("runtime-observed:tool-aci11-observed".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "obs-aci11-observed".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(tool_call.clone()),
                        source: "runtime_output".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.file_read".to_string(),
                        observed_status: "completed".to_string(),
                        instrumentation_level: "full".to_string(),
                        confidence: "observed".to_string(),
                        raw_event_hash: "fnv1a64:observedhash".to_string(),
                        artifact_id: Some("artifact-output".to_string()),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append observed observation");

        // 3) An `agent_reported` claim (distinct class), carrying confidence.
        store
            .append_event(
                NewEvent {
                    event_id: "event-aci11-report".to_string(),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "agent-report".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: Some("turn-aci11".to_string()),
                    item_id: Some(report_call.to_string()),
                    payload_json: "{\"source\":\"agent_reported\"}".to_string(),
                    idempotency_key: Some("agent-report:sub-aci11".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "obs-aci11-report".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(report_call.clone()),
                        source: "agent_reported".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.complete_subtask".to_string(),
                        observed_status: "reported".to_string(),
                        instrumentation_level: "structured_observed".to_string(),
                        confidence: "90".to_string(),
                        raw_event_hash: "agent-report:tool-aci11-report".to_string(),
                        artifact_id: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append agent report");

        // 4) An adapter-native tool update with a STABLE external id, sent TWICE
        //    with the same idempotency key. The second append must dedupe
        //    (tool-exposure.md:352: a `toolCallId` is stable within a session).
        let append_adapter = |event_suffix: &str| {
            store
                .append_event(
                    NewEvent {
                        event_id: format!("event-aci11-adapter-{event_suffix}"),
                        kind: EventKind::ToolObservationRecorded,
                        actor: "adapter-replay".to_string(),
                        project_id: Some(project_id.clone()),
                        task_id: None,
                        agent_id: None,
                        session_id: Some(session_id.clone()),
                        run_id: None,
                        turn_id: Some("turn-aci11".to_string()),
                        item_id: Some(adapter_call.to_string()),
                        payload_json: "{\"source\":\"adapter_event\"}".to_string(),
                        idempotency_key: Some(
                            "tool-observation:tool-aci11-adapter:completed".to_string(),
                        ),
                        redaction_state: RedactionState::Safe,
                    },
                    &[ProjectionRecord::ToolObservation(
                        ToolObservationProjection {
                            tool_observation_id: "obs-aci11-adapter".to_string(),
                            session_id: session_id.clone(),
                            tool_call_id: Some(adapter_call.clone()),
                            source: "adapter_event".to_string(),
                            external_tool_ref: Some(adapter_call.to_string()),
                            tool_name: "exec_command".to_string(),
                            observed_status: "completed".to_string(),
                            instrumentation_level: "observed_only".to_string(),
                            confidence: "high".to_string(),
                            raw_event_hash: "fnv1a64:adapterhash".to_string(),
                            artifact_id: None,
                            updated_sequence: 0,
                        },
                    )],
                )
                .expect("append adapter observation")
        };
        append_adapter("first");
        // Same stable external id / idempotency key -> deduped (no second row).
        append_adapter("duplicate");

        let tools = store
            .tool_calls_for_session(&session_id)
            .expect("read tool calls");
        let observations = store
            .tool_observations_for_session(&session_id)
            .expect("read observations");
        // 3 observations (observed + report + adapter), NOT 4: the adapter
        // duplicate deduped on its stable external id.
        assert_eq!(
            observations.len(),
            3,
            "the adapter-native duplicate must dedupe on its stable external id"
        );
        let event_count = store.event_count().expect("event count");
        (tools, observations, event_count)
    };

    // -- Restart: a FRESH store reopened from the same root on disk derives the
    //    read models from the persisted event log alone. --
    let reopened = SqliteStateStore::open(&root).expect("reopen state store");
    reopened.rebuild_projections().expect("rebuild projections");

    assert_eq!(
        reopened
            .tool_calls_for_session(&session_id)
            .expect("reopened tool calls"),
        tools_before,
        "tool-call projection must rebuild identically after reopen",
    );
    assert_eq!(
        reopened
            .tool_observations_for_session(&session_id)
            .expect("reopened observations"),
        observations_before,
        "observation + report projections must rebuild identically after reopen",
    );
    assert_eq!(
        reopened.event_count().expect("reopened event count"),
        event_count_before,
        "reopen + rebuild introduces no new events",
    );
}

#[test]
fn memory_records_and_sources_are_persisted_rebuilt_and_packet_filterable() {
    let store = temp_store("memory-record-rebuild");
    let project_id = ProjectId::new("project-capo");
    let record_id = "memory-record-architecture-static-dispatch";

    store
            .append_event(
                NewEvent {
                    event_id: "event-memory-record-ingested".to_string(),
                    kind: EventKind::MemoryRecordIngested,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(record_id.to_string()),
                    payload_json: "{\"kind\":\"memory.record_ingested\"}".to_string(),
                    idempotency_key: Some("memory:record:static-dispatch".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[
                    ProjectionRecord::MemoryRecord(Box::new(MemoryRecordProjection {
                        memory_record_id: record_id.to_string(),
                        project_id: project_id.clone(),
                        scope: "project".to_string(),
                        scope_owner_ref: "project-capo".to_string(),
                        subject_ref: Some("workpads/architecture/boundaries.md".to_string()),
                        sensitivity_classification: "internal".to_string(),
                        record_kind: "repo_convention".to_string(),
                        subject: "architecture boundaries".to_string(),
                        predicate: "prefer".to_string(),
                        object: "static dispatch for known prototype boundaries".to_string(),
                        body: "Use static dispatch for known Capo boundaries while keeping adapter swaps explicit.".to_string(),
                        confidence: "high".to_string(),
                        review_state: "reviewed".to_string(),
                        source_count: 1,
                        valid_from: Some("2026-05-25T00:00:00Z".to_string()),
                        valid_until: None,
                        supersedes_memory_record_id: None,
                        revoked_by_memory_record_id: None,
                        redaction_state: RedactionState::Safe.as_str().to_string(),
                        invalidated_at: None,
                        invalidation_reason: None,
                        packet_item_ref: Some("memory-record:architecture-static-dispatch".to_string()),
                        updated_sequence: 0,
                    })),
                    ProjectionRecord::MemorySource(MemorySourceProjection {
                        memory_source_id: "memory-source-boundaries-static-dispatch".to_string(),
                        memory_record_id: record_id.to_string(),
                        source_kind: "markdown".to_string(),
                        source_event_id: None,
                        source_artifact_id: None,
                        source_path: Some("workpads/architecture/boundaries.md".to_string()),
                        source_anchor: Some("Static Dispatch Shape".to_string()),
                        source_content_hash: Some("sha256:boundaries".to_string()),
                        source_sequence: Some(1),
                        quote_artifact_id: Some("artifact-quote-static-dispatch".to_string()),
                        observed_at: Some("2026-05-25T00:00:00Z".to_string()),
                        updated_sequence: 0,
                    }),
                ],
            )
            .expect("append memory record");

    let records = store
        .memory_records_for_project(&project_id)
        .expect("memory records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].review_state, "reviewed");
    assert_eq!(records[0].sensitivity_classification, "internal");
    assert_eq!(
        records[0].packet_item_ref.as_deref(),
        Some("memory-record:architecture-static-dispatch")
    );
    assert!(records[0].is_packet_eligible());

    let sources = store
        .memory_sources_for_record(record_id)
        .expect("memory sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].source_path.as_deref(),
        Some("workpads/architecture/boundaries.md")
    );
    assert_eq!(
        sources[0].source_anchor.as_deref(),
        Some("Static Dispatch Shape")
    );
    assert_eq!(
        sources[0].source_content_hash.as_deref(),
        Some("sha256:boundaries")
    );

    store
            .append_event(
                NewEvent {
                    event_id: "event-memory-record-invalidated".to_string(),
                    kind: EventKind::MemoryRecordInvalidated,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(record_id.to_string()),
                    payload_json: "{\"kind\":\"memory.record_invalidated\"}".to_string(),
                    idempotency_key: Some("memory:record:static-dispatch:invalidated".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::MemoryRecord(Box::new(MemoryRecordProjection {
                    memory_record_id: record_id.to_string(),
                    project_id: project_id.clone(),
                    scope: "project".to_string(),
                    scope_owner_ref: "project-capo".to_string(),
                    subject_ref: Some("workpads/architecture/boundaries.md".to_string()),
                    sensitivity_classification: "internal".to_string(),
                    record_kind: "repo_convention".to_string(),
                    subject: "architecture boundaries".to_string(),
                    predicate: "prefer".to_string(),
                    object: "static dispatch for known prototype boundaries".to_string(),
                    body: "Use static dispatch for known Capo boundaries while keeping adapter swaps explicit.".to_string(),
                    confidence: "high".to_string(),
                    review_state: "superseded".to_string(),
                    source_count: 1,
                    valid_from: Some("2026-05-25T00:00:00Z".to_string()),
                    valid_until: Some("2026-05-25T01:00:00Z".to_string()),
                    supersedes_memory_record_id: None,
                    revoked_by_memory_record_id: Some("memory-record-new-convention".to_string()),
                    redaction_state: RedactionState::Safe.as_str().to_string(),
                    invalidated_at: Some("2026-05-25T01:00:00Z".to_string()),
                    invalidation_reason: Some("superseded by clearer boundary note".to_string()),
                    packet_item_ref: Some("memory-record:architecture-static-dispatch".to_string()),
                    updated_sequence: 0,
                }))],
            )
            .expect("append invalidation");

    assert!(
        store
            .packet_eligible_memory_records(&project_id)
            .expect("packet eligible records")
            .is_empty()
    );

    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .memory_records_for_project(&project_id)
        .expect("rebuilt memory records");
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].review_state, "superseded");
    assert_eq!(
        rebuilt[0].invalidation_reason.as_deref(),
        Some("superseded by clearer boundary note")
    );
    assert_eq!(
        store
            .memory_sources_for_record(record_id)
            .expect("rebuilt memory sources")[0]
            .source_content_hash
            .as_deref(),
        Some("sha256:boundaries")
    );
}

#[test]
fn packet_eligible_memory_records_require_replayable_sources() {
    let store = temp_store("memory-record-packet-eligibility");
    let project_id = ProjectId::new("project-capo");
    let complete_record = reviewed_memory_record(&project_id, "memory-record-complete", 1);
    let no_source_count_record =
        reviewed_memory_record(&project_id, "memory-record-no-source-count", 0);
    let missing_hash_record = reviewed_memory_record(&project_id, "memory-record-no-hash", 1);

    store
        .append_event(
            NewEvent::new(
                "event-memory-packet-eligibility",
                EventKind::MemoryRecordIngested,
                "test",
            ),
            &[
                ProjectionRecord::MemoryRecord(Box::new(complete_record)),
                ProjectionRecord::MemorySource(MemorySourceProjection {
                    memory_source_id: "memory-source-complete".to_string(),
                    memory_record_id: "memory-record-complete".to_string(),
                    source_kind: "markdown".to_string(),
                    source_event_id: None,
                    source_artifact_id: None,
                    source_path: Some("workpads/prototype/knowledge.md".to_string()),
                    source_anchor: Some("Prototype Gate".to_string()),
                    source_content_hash: Some("sha256:complete".to_string()),
                    source_sequence: Some(1),
                    quote_artifact_id: None,
                    observed_at: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::MemoryRecord(Box::new(no_source_count_record)),
                ProjectionRecord::MemoryRecord(Box::new(missing_hash_record)),
                ProjectionRecord::MemorySource(MemorySourceProjection {
                    memory_source_id: "memory-source-missing-hash".to_string(),
                    memory_record_id: "memory-record-no-hash".to_string(),
                    source_kind: "markdown".to_string(),
                    source_event_id: None,
                    source_artifact_id: None,
                    source_path: Some("workpads/prototype/knowledge.md".to_string()),
                    source_anchor: Some("Prototype Gate".to_string()),
                    source_content_hash: None,
                    source_sequence: Some(2),
                    quote_artifact_id: None,
                    observed_at: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append memory eligibility records");

    let eligible = store
        .packet_eligible_memory_records(&project_id)
        .expect("eligible records");
    assert_eq!(eligible.len(), 1);
    assert_eq!(eligible[0].memory_record_id, "memory-record-complete");
}

#[test]
fn rebuild_fails_closed_on_incomplete_memory_record_payloads() {
    let store = temp_store("memory-record-malformed-projection");
    store
        .append_event(
            NewEvent::new(
                "event-malformed-memory-source",
                EventKind::MemoryRecordIngested,
                "test",
            ),
            &[],
        )
        .unwrap();

    let connection = Connection::open(store.db_path()).unwrap();
    connection
        .execute(
            "INSERT INTO projection_records (
                    sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
                 ) VALUES (1, 'memory_record', 'memory-record-bad', 'project-capo',
                    'project', 'project-capo', NULL, 'internal', 'fact', 'reviewed', '1', '{}')",
            [],
        )
        .unwrap();

    assert!(store.rebuild_projections().is_err());
}

#[test]
fn task_outcome_reports_are_persisted_and_rebuilt() {
    let store = temp_store("task-outcome-report-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-me2");
    let session_id = SessionId::new("session-me2");
    let run_id = RunId::new("run-me2");
    let report_id = "task-outcome-task-me2";

    store
        .append_event(
            NewEvent {
                event_id: "event-task-outcome-report".to_string(),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some(report_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::TaskOutcomeReport(
                TaskOutcomeReportProjection {
                    task_outcome_report_id: report_id.to_string(),
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    session_id,
                    run_id,
                    outcome_status: "completed".to_string(),
                    started_sequence: 2,
                    completed_sequence: 8,
                    duration_sequence_span: 6,
                    action_count: 7,
                    tool_call_count: 2,
                    evidence_count: 3,
                    memory_packet_count: 1,
                    confidence: Some(84),
                    blocker: None,
                    review_outcome: "reviewed_no_blockers".to_string(),
                    report_artifact_id: Some("artifact-task-outcome".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append task outcome report");

    store.rebuild_projections().expect("rebuild projections");
    let reports = store
        .task_outcome_reports_for_task(&task_id)
        .expect("task outcome reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].project_id, project_id);
    assert_eq!(reports[0].outcome_status, "completed");
    assert_eq!(reports[0].duration_sequence_span, 6);
    assert_eq!(reports[0].tool_call_count, 2);
    assert_eq!(reports[0].review_outcome, "reviewed_no_blockers");
    assert_eq!(
        reports[0].report_artifact_id.as_deref(),
        Some("artifact-task-outcome")
    );
}

#[test]
fn review_findings_are_persisted_and_rebuilt() {
    let store = temp_store("review-finding-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-me3");
    let session_id = SessionId::new("session-me3");
    let run_id = RunId::new("run-me3");
    let tool_call_id = ToolCallId::new("tool-me3");
    let finding_id = "review-finding-me3";

    store
        .append_event(
            NewEvent {
                event_id: "event-review-finding".to_string(),
                kind: EventKind::ReviewFindingRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some(finding_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: finding_id.to_string(),
                project_id: project_id.clone(),
                task_id: task_id.clone(),
                session_id: session_id.clone(),
                run_id: Some(run_id.clone()),
                tool_call_id: Some(tool_call_id.clone()),
                workpad_task_id: Some("ME3".to_string()),
                reviewer: "focused-review".to_string(),
                finding_kind: "blocker".to_string(),
                severity: "high".to_string(),
                summary: "Link findings to follow-up workpad tasks.".to_string(),
                status: "open".to_string(),
                evidence_artifact_id: Some("artifact-review".to_string()),
                follow_up: Some("ME3".to_string()),
                updated_sequence: 0,
            })],
        )
        .expect("append review finding");

    store.rebuild_projections().expect("rebuild projections");
    let findings = store
        .review_findings_for_session(&session_id)
        .expect("review findings");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].project_id, project_id);
    assert_eq!(findings[0].task_id, task_id);
    assert_eq!(findings[0].run_id.as_ref(), Some(&run_id));
    assert_eq!(findings[0].tool_call_id.as_ref(), Some(&tool_call_id));
    assert_eq!(findings[0].workpad_task_id.as_deref(), Some("ME3"));
    assert_eq!(findings[0].finding_kind, "blocker");
    assert_eq!(findings[0].status, "open");
    assert_eq!(
        findings[0].evidence_artifact_id.as_deref(),
        Some("artifact-review")
    );
}

#[test]
fn permission_approval_projection_is_persisted_and_rebuilt() {
    let store = temp_store("permission-approval-rebuild");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-fake");
    let approval_id = "approval-shell";
    let grant_id = "grant-approval-shell";

    store
        .append_event(
            NewEvent {
                event_id: "event-approval-queued".to_string(),
                kind: EventKind::PermissionApprovalQueued,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some("tool-call-1".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: approval_id.to_string(),
                    project_id: project_id.clone(),
                    session_id: Some(session_id.clone()),
                    tool_call_id: Some(ToolCallId::new("tool-call-1")),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    status: "pending".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "run shell".to_string(),
                    decision: None,
                    capability_grant_id: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append queued approval");

    store
        .append_event(
            NewEvent::new(
                "event-approval-decided",
                EventKind::PermissionDecided,
                "test",
            ),
            &[
                ProjectionRecord::PermissionApproval(PermissionApprovalProjection {
                    approval_id: approval_id.to_string(),
                    project_id: project_id.clone(),
                    session_id: Some(session_id),
                    tool_call_id: Some(ToolCallId::new("tool-call-1")),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    status: "decided".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "run shell".to_string(),
                    decision: Some("reject_always".to_string()),
                    capability_grant_id: Some(grant_id.to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::CapabilityGrant(CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    effect: "deny".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    decision_source: "user".to_string(),
                    persistence: "until_revoked".to_string(),
                    explanation: "user approval decision reject_always for approval-shell"
                        .to_string(),
                    created_at: None,
                    expires_at: None,
                    revoked_at: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append decided approval");

    store.rebuild_projections().expect("rebuild projections");
    let approval = store
        .permission_approval(&project_id, approval_id)
        .expect("approval query")
        .expect("approval");
    assert_eq!(approval.status, "decided");
    assert_eq!(approval.decision.as_deref(), Some("reject_always"));
    assert_eq!(approval.capability_grant_id.as_deref(), Some(grant_id));
    assert_eq!(approval.reason, "run shell");
    let grants = store.capability_grants().expect("grant query");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].effect, "deny");
    assert_eq!(grants[0].persistence, "until_revoked");
}

#[test]
fn runtime_targets_are_persisted_and_rebuilt() {
    let store = temp_store("runtime-target-rebuild");
    let project_id = ProjectId::new("project-capo");
    store
        .append_event(
            NewEvent {
                event_id: "event-runtime-target-registered".to_string(),
                kind: EventKind::RuntimeTargetRegistered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("runtime-target-local-1".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::RuntimeTarget(RuntimeTargetProjection {
                runtime_target_id: "runtime-target-local-1".to_string(),
                project_id: project_id.clone(),
                name: "local dev box".to_string(),
                runner_kind: "local-process".to_string(),
                workspace_root: "/tmp/capo-workspace".to_string(),
                artifact_root: "/tmp/capo-artifacts".to_string(),
                default_cwd: "/tmp/capo-workspace".to_string(),
                capability_profile_id: "read-only-local".to_string(),
                connectivity_endpoint_id: Some("endpoint-loopback-1".to_string()),
                status: "available".to_string(),
                updated_sequence: 0,
            })],
        )
        .expect("append runtime target");

    store.rebuild_projections().expect("rebuild projections");
    let targets = store.runtime_targets(&project_id).expect("runtime targets");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].runtime_target_id, "runtime-target-local-1");
    assert_eq!(targets[0].runner_kind, "local-process");
    assert_eq!(targets[0].workspace_root, "/tmp/capo-workspace");
    assert_eq!(
        targets[0].connectivity_endpoint_id.as_deref(),
        Some("endpoint-loopback-1")
    );
}

#[test]
fn connectivity_exposure_requires_grant_and_projects_revocation_and_health() {
    let store = temp_store("connectivity-exposure-policy");
    let project_id = ProjectId::new("project-capo");
    let exposure_id = "exposure-private-control";
    let grant_id = "grant-private-tunnel";

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-requested".to_string(),
                kind: EventKind::ConnectivityExposureRequested,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    status: "blocked_pending_permission".to_string(),
                    capability_grant_id: None,
                    health_status: "unknown".to_string(),
                    reachable: false,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append requested exposure");

    assert_eq!(
        store
            .connectivity_exposures(&project_id)
            .expect("exposures")[0]
            .status,
        "blocked_pending_permission"
    );

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-grant".to_string(),
                kind: EventKind::CapabilityGrantCreated,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::CapabilityGrant(
                CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: "remote-control-reviewed".to_string(),
                    scope_json: "[\"network:connect:private_tunnel\"]".to_string(),
                    effect: "allow".to_string(),
                    subject_json:
                        "{\"endpoint_id\":\"endpoint-private-1\",\"owner_id\":\"remote-target-1\"}"
                            .to_string(),
                    decision_source: "user".to_string(),
                    persistence: "until_revoked".to_string(),
                    explanation: "operator allowed private remote-control exposure".to_string(),
                    created_at: None,
                    expires_at: None,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append exposure grant");

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-changed".to_string(),
                kind: EventKind::ConnectivityExposureChanged,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    status: "active".to_string(),
                    capability_grant_id: Some(grant_id.to_string()),
                    health_status: "available".to_string(),
                    reachable: true,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append granted exposure");

    let active = store
        .connectivity_exposures(&project_id)
        .expect("active exposure")
        .pop()
        .expect("exposure row");
    assert_eq!(active.status, "active");
    assert_eq!(active.capability_grant_id.as_deref(), Some(grant_id));
    assert_eq!(active.health_status, "available");
    assert!(active.reachable);

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-revoked".to_string(),
                kind: EventKind::ConnectivityExposureRevoked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    status: "revoked".to_string(),
                    reachable: false,
                    health_status: "disabled".to_string(),
                    revoked_at: Some("2026-05-25T00:00:00Z".to_string()),
                    capability_grant_id: Some(grant_id.to_string()),
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append revoked exposure");

    store.rebuild_projections().expect("rebuild projections");
    let revoked = store
        .connectivity_exposures(&project_id)
        .expect("rebuilt exposure")
        .pop()
        .expect("exposure row");
    assert_eq!(revoked.status, "revoked");
    assert_eq!(revoked.health_status, "disabled");
    assert!(!revoked.reachable);
    assert_eq!(revoked.revoked_at.as_deref(), Some("2026-05-25T00:00:00Z"));
}

#[test]
fn adapter_readiness_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-readiness-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-readiness".to_string(),
                kind: EventKind::AdapterReadinessChecked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("codex_exec".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterReadiness(
                AdapterReadinessProjection {
                    adapter_kind: "codex_exec".to_string(),
                    project_id: project_id.clone(),
                    program: "codex".to_string(),
                    opt_in_env: "CAPO_RUN_CODEX_LOCAL_SMOKE".to_string(),
                    opted_in: false,
                    smoke_status: "waiting_on_opt_in".to_string(),
                    credential_policy: "not_inspected".to_string(),
                    expected_marker: "CAPO_CODEX_SMOKE_OK".to_string(),
                    env_allowlist_count: 7,
                    redaction_rule_count: 6,
                    output_limit_bytes: 131072,
                    dogfood_blocker: Some("real_subscription_smoke_not_recorded".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter readiness");

    store.rebuild_projections().expect("rebuild projections");
    let readiness = store
        .adapter_readiness(&project_id)
        .expect("adapter readiness");
    assert_eq!(readiness.len(), 1);
    assert_eq!(readiness[0].adapter_kind, "codex_exec");
    assert_eq!(readiness[0].credential_policy, "not_inspected");
    assert_eq!(
        readiness[0].dogfood_blocker.as_deref(),
        Some("real_subscription_smoke_not_recorded")
    );
}

#[test]
fn adapter_smoke_report_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-smoke-report-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-smoke-report".to_string(),
                kind: EventKind::AdapterSmokeRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("adapter-smoke-codex-skipped".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterSmokeReport(
                AdapterSmokeReportProjection {
                    smoke_report_id: "adapter-smoke-codex-skipped".to_string(),
                    project_id: project_id.clone(),
                    adapter_kind: "codex_exec".to_string(),
                    smoke_status: "skipped".to_string(),
                    credential_scan_status: "not_run".to_string(),
                    marker_found: false,
                    artifact_root: None,
                    reason: "waiting for opt-in".to_string(),
                    dogfood_readiness_effect: "real_subscription_smoke_not_recorded".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter smoke report");

    store.rebuild_projections().expect("rebuild projections");
    let reports = store
        .adapter_smoke_reports(&project_id)
        .expect("adapter smoke reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].adapter_kind, "codex_exec");
    assert_eq!(reports[0].credential_scan_status, "not_run");
    assert_eq!(
        reports[0].dogfood_readiness_effect,
        "real_subscription_smoke_not_recorded"
    );
}

#[test]
fn adapter_dispatch_plan_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-plan-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-plan".to_string(),
                kind: EventKind::AdapterDispatchPlanned,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-plan-codex".to_string()),
                payload_json:
                    "{\"runtime_prompt_policy\":\"not_rendered\",\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchPlan(
                AdapterDispatchPlanProjection {
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    project_id: project_id.clone(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_kind: "codex_subscription".to_string(),
                    credential_scope: "user_local_subscription".to_string(),
                    agent_id: AgentId::new("agent-codex"),
                    agent_name: "codex".to_string(),
                    session_id: SessionId::new("session-codex"),
                    run_id: RunId::new("run-codex"),
                    runtime_program: "codex".to_string(),
                    runtime_arg_count: 9,
                    runtime_prompt_policy: "not_rendered".to_string(),
                    runtime_cwd: "/tmp/capo-workspace".to_string(),
                    artifact_root: "/tmp/capo-artifacts".to_string(),
                    request_env_count: 0,
                    env_allowlist_count: 7,
                    redaction_rule_count: 6,
                    stdout_format: "jsonl".to_string(),
                    stderr_policy: "logs_redacted".to_string(),
                    provider_cli_executed: false,
                    status: "planned".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch plan");

    store.rebuild_projections().expect("rebuild projections");
    let plans = store
        .adapter_dispatch_plans(&project_id)
        .expect("adapter dispatch plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].adapter_kind, "codex_exec");
    assert_eq!(plans[0].credential_scope, "user_local_subscription");
    assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
    assert!(!plans[0].provider_cli_executed);
    assert_eq!(plans[0].status, "planned");
}

#[test]
fn adapter_dispatch_gate_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-gate-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-gate".to_string(),
                kind: EventKind::AdapterDispatchGateChecked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-gate-codex".to_string()),
                payload_json:
                    "{\"provider_cli_execution_allowed\":false,\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchGate(
                AdapterDispatchGateProjection {
                    dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_cli_execution_allowed: false,
                    status: "blocked".to_string(),
                    required_dogfood_gate: "blocked_pending_real_smoke".to_string(),
                    reason_codes: "codex_exec:real_subscription_smoke_not_recorded".to_string(),
                    provider_cli_executed: false,
                    runtime_prompt_policy: "not_rendered".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch gate");

    store.rebuild_projections().expect("rebuild projections");
    let gates = store
        .adapter_dispatch_gates(&project_id)
        .expect("adapter dispatch gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(gates[0].adapter_kind, "codex_exec");
    assert_eq!(gates[0].status, "blocked");
    assert!(!gates[0].provider_cli_execution_allowed);
    assert!(!gates[0].provider_cli_executed);
    assert_eq!(gates[0].runtime_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_replay_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-replay-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-replay".to_string(),
                    kind: EventKind::AdapterDispatchReplayed,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(TaskId::new("task-codex")),
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-replay-codex".to_string()),
                    payload_json:
                        "{\"provider_cli_executed\":false,\"raw_content_policy\":\"content_hashed_not_rendered\"}"
                            .to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchReplay(
                    AdapterDispatchReplayProjection {
                        dispatch_replay_id: "adapter-dispatch-replay-codex".to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        session_id: SessionId::new("session-codex"),
                        run_id: RunId::new("run-codex"),
                        fixture_path: "fixtures/codex-exec.jsonl".to_string(),
                        fixture_hash: "fixture-hash".to_string(),
                        input_event_count: 4,
                        appended_event_count: 4,
                        tool_event_count: 2,
                        summary_event_count: 1,
                        completed_turn_count: 1,
                        provider_cli_executed: false,
                        raw_content_policy: "content_hashed_not_rendered".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch replay");

    store.rebuild_projections().expect("rebuild projections");
    let replays = store
        .adapter_dispatch_replays(&project_id)
        .expect("adapter dispatch replays");
    assert_eq!(replays.len(), 1);
    assert_eq!(replays[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(replays[0].dispatch_gate_id, "adapter-dispatch-gate-codex");
    assert_eq!(replays[0].adapter_kind, "codex_exec");
    assert_eq!(replays[0].fixture_hash, "fixture-hash");
    assert_eq!(replays[0].tool_event_count, 2);
    assert!(!replays[0].provider_cli_executed);
    assert_eq!(replays[0].raw_content_policy, "content_hashed_not_rendered");
}

#[test]
fn adapter_dispatch_execution_request_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-execution-request-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-execution-request".to_string(),
                kind: EventKind::AdapterDispatchExecutionRequested,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-execution-request-codex".to_string()),
                payload_json:
                    "{\"provider_cli_execution_allowed\":true,\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchExecutionRequest(
                AdapterDispatchExecutionRequestProjection {
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_cli_execution_allowed: true,
                    provider_cli_executed: false,
                    status: "waiting_on_explicit_provider_opt_in".to_string(),
                    opt_in_env: "CAPO_RUN_CODEX_LOCAL_DISPATCH".to_string(),
                    runtime_prompt_policy: "not_rendered".to_string(),
                    reason_codes: "explicit_provider_execution_opt_in_required".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch execution request");

    store.rebuild_projections().expect("rebuild projections");
    let requests = store
        .adapter_dispatch_execution_requests(&project_id)
        .expect("adapter dispatch execution requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(requests[0].dispatch_gate_id, "adapter-dispatch-gate-codex");
    assert_eq!(requests[0].status, "waiting_on_explicit_provider_opt_in");
    assert_eq!(requests[0].opt_in_env, "CAPO_RUN_CODEX_LOCAL_DISPATCH");
    assert!(requests[0].provider_cli_execution_allowed);
    assert!(!requests[0].provider_cli_executed);
    assert_eq!(requests[0].runtime_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_execution_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-execution-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-execution".to_string(),
                kind: EventKind::AdapterDispatchExecuted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-execution-codex".to_string()),
                payload_json:
                    "{\"provider_cli_executed\":true,\"raw_prompt_policy\":\"not_rendered\"}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchExecution(
                AdapterDispatchExecutionProjection {
                    dispatch_execution_id: "adapter-dispatch-execution-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    session_id: SessionId::new("session-codex"),
                    run_id: RunId::new("run-codex"),
                    provider_cli_execution_allowed: true,
                    provider_cli_executed: true,
                    status: "exited".to_string(),
                    exit_code: Some(0),
                    runtime_process_ref: Some("local-process-run-codex".to_string()),
                    stdout_artifact_id: Some("artifact-stdout".to_string()),
                    stderr_artifact_id: Some("artifact-stderr".to_string()),
                    artifact_root: "/tmp/capo-artifacts".to_string(),
                    credential_scan_status: "clean".to_string(),
                    raw_prompt_policy: "not_rendered".to_string(),
                    raw_output_policy: "bounded_redacted_artifacts".to_string(),
                    reason_codes: "provider_cli_executed_and_artifacts_scanned".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch execution");

    store.rebuild_projections().expect("rebuild projections");
    let executions = store
        .adapter_dispatch_executions(&project_id)
        .expect("adapter dispatch executions");
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].dispatch_plan_id,
        "adapter-dispatch-plan-codex"
    );
    assert_eq!(
        executions[0].execution_request_id,
        "adapter-dispatch-execution-request-codex"
    );
    assert_eq!(executions[0].status, "exited");
    assert_eq!(executions[0].exit_code, Some(0));
    assert!(executions[0].provider_cli_execution_allowed);
    assert!(executions[0].provider_cli_executed);
    assert_eq!(executions[0].credential_scan_status, "clean");
    assert_eq!(executions[0].raw_prompt_policy, "not_rendered");
    assert_eq!(
        executions[0].raw_output_policy,
        "bounded_redacted_artifacts"
    );
}

#[test]
fn adapter_dispatch_prompt_source_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-prompt-source-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-prompt-source".to_string(),
                kind: EventKind::AdapterDispatchPromptSourceRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-prompt-source-codex".to_string()),
                payload_json:
                    "{\"raw_prompt_policy\":\"not_rendered\",\"source_kind\":\"workpad_task\"}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchPromptSource(
                AdapterDispatchPromptSourceProjection {
                    prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    prompt_hash: "prompt-hash".to_string(),
                    source_kind: "workpad_task".to_string(),
                    source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                    source_hash: Some("source-hash".to_string()),
                    materialization_status: "replayable_if_source_hash_matches".to_string(),
                    raw_prompt_policy: "not_rendered".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch prompt source");

    store.rebuild_projections().expect("rebuild projections");
    let sources = store
        .adapter_dispatch_prompt_sources(&project_id)
        .expect("adapter dispatch prompt sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(sources[0].source_kind, "workpad_task");
    assert_eq!(
        sources[0].source_ref.as_deref(),
        Some("workpads/features/tasks.md#f1")
    );
    assert_eq!(
        sources[0].materialization_status,
        "replayable_if_source_hash_matches"
    );
    assert_eq!(sources[0].raw_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_prompt_materialization_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-prompt-materialization-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-prompt-materialization".to_string(),
                    kind: EventKind::AdapterDispatchPromptMaterialized,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("adapter-dispatch-prompt-materialization-codex".to_string()),
                    payload_json:
                        "{\"raw_prompt_policy\":\"not_rendered\",\"status\":\"ready_without_rendering_prompt\"}"
                            .to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                    AdapterDispatchPromptMaterializationProjection {
                        materialization_id: "adapter-dispatch-prompt-materialization-codex"
                            .to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                        source_kind: "workpad_task".to_string(),
                        source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                        expected_source_hash: Some("source-hash".to_string()),
                        observed_source_hash: Some("source-hash".to_string()),
                        expected_prompt_hash: "prompt-hash".to_string(),
                        materialized_prompt_hash: Some("prompt-hash".to_string()),
                        status: "ready_without_rendering_prompt".to_string(),
                        raw_prompt_policy: "not_rendered".to_string(),
                        reason_codes: "prompt_hash_matches_source".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch prompt materialization");

    store.rebuild_projections().expect("rebuild projections");
    let rows = store
        .adapter_dispatch_prompt_materializations(&project_id)
        .expect("adapter dispatch prompt materializations");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "ready_without_rendering_prompt");
    assert_eq!(rows[0].raw_prompt_policy, "not_rendered");
    assert_eq!(
        rows[0].materialized_prompt_hash.as_deref(),
        Some("prompt-hash")
    );
}

#[test]
fn permission_approval_projection_rejects_invalid_json_payloads() {
    let store = temp_store("permission-approval-invalid-json");
    let project_id = ProjectId::new("project-capo");

    let error = store
        .append_event(
            NewEvent::new(
                "event-invalid-approval-json",
                EventKind::PermissionApprovalQueued,
                "test",
            ),
            &[ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: "approval-invalid".to_string(),
                    project_id,
                    session_id: None,
                    tool_call_id: None,
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{not-json".to_string(),
                    status: "pending".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "invalid".to_string(),
                    decision: None,
                    capability_grant_id: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect_err("invalid projection JSON should fail before commit");
    assert!(matches!(
        error,
        StateError::InvalidProjectionJson {
            kind: "permission_approval",
            field: "subject_json",
            ..
        }
    ));
    assert_eq!(store.event_count().expect("event count"), 0);
}

#[test]
fn artifact_persistence_rejects_unclassified_or_sensitive_rows() {
    let store = temp_store("artifact-redaction");
    let artifact = |artifact_id: &str, redaction_state| ArtifactRecord {
        artifact_id: artifact_id.to_string(),
        project_id: None,
        session_id: None,
        run_id: None,
        kind: "raw-output".to_string(),
        uri: "artifacts/raw/output.txt".to_string(),
        content_hash: "hash-output".to_string(),
        size_bytes: 99,
        redaction_state,
    };

    assert!(matches!(
        store.record_artifact(artifact("artifact-unknown", RedactionState::Unknown)),
        Err(StateError::UnsafeArtifactRedactionState(
            RedactionState::Unknown
        ))
    ));
    assert!(matches!(
        store.record_artifact(artifact(
            "artifact-sensitive",
            RedactionState::ContainsSensitive
        )),
        Err(StateError::UnsafeArtifactRedactionState(
            RedactionState::ContainsSensitive
        ))
    ));
}

#[test]
fn rebuild_watermark_tracks_events_without_projection_records() {
    let store = temp_store("empty-projection-watermark");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-p2");

    store
        .append_event(
            NewEvent::new("event-with-projection", EventKind::TaskDiscovered, "test"),
            &[ProjectionRecord::Task(TaskProjection {
                task_id,
                project_id,
                title: "P2".to_string(),
                capo_execution_status: "active".to_string(),
                active_session_id: None,
                latest_summary: None,
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .unwrap();
    store
        .append_event(
            NewEvent::new(
                "event-without-projection",
                EventKind::RecoveryStarted,
                "test",
            ),
            &[],
        )
        .unwrap();

    assert_eq!(store.watermark("default").unwrap(), Some(2));
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.watermark("default").unwrap(), Some(2));
}

#[test]
fn rebuild_fails_closed_on_malformed_projection_numbers() {
    let store = temp_store("malformed-projection");
    store
        .append_event(
            NewEvent::new("event-malformed-source", EventKind::SessionStarted, "test"),
            &[],
        )
        .unwrap();

    let connection = Connection::open(store.db_path()).unwrap();
    connection
        .execute(
            "INSERT INTO projection_records (
                    sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
                 ) VALUES (1, 'session', 'session-bad', 'project-capo', NULL,
                    'agent-fake', 'Bad session', 'running', 'prove decode', NULL,
                    'not-a-number', '{}')",
            [],
        )
        .unwrap();

    assert!(store.rebuild_projections().is_err());
}

#[test]
fn recovery_attempts_record_restart_shape_without_mutating_events() {
    let store = temp_store("recovery");
    store
        .append_event(
            NewEvent::new("event-recovery-source", EventKind::RecoveryStarted, "test"),
            &[],
        )
        .unwrap();

    let started = store.begin_recovery("recovery-1").unwrap();
    assert_eq!(started.status, "started");
    assert_eq!(started.started_sequence, 1);
    assert_eq!(store.event_count().unwrap(), 1);

    let completed = store.complete_recovery("recovery-1").unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.started_sequence, 1);
    assert_eq!(completed.completed_sequence, Some(1));
    assert_eq!(store.event_count().unwrap(), 1);
}

#[test]
fn recovery_completion_requires_started_attempt() {
    let store = temp_store("missing-recovery");
    assert!(matches!(
        store.complete_recovery("missing"),
        Err(StateError::MissingRecoveryAttempt(id)) if id == "missing"
    ));
}

#[test]
fn sg3_capability_grant_revoked_and_expired_event_kinds_round_trip() {
    // SG3: the new grant-lifecycle event kinds have stable wire strings and
    // round-trip through `as_str`/`from_wire`.
    assert_eq!(
        EventKind::CapabilityGrantRevoked.as_str(),
        "capability.grant_revoked"
    );
    assert_eq!(
        EventKind::CapabilityGrantExpired.as_str(),
        "capability.grant_expired"
    );
    assert_eq!(
        EventKind::from_wire("capability.grant_revoked"),
        Some(EventKind::CapabilityGrantRevoked)
    );
    assert_eq!(
        EventKind::from_wire("capability.grant_expired"),
        Some(EventKind::CapabilityGrantExpired)
    );
}

#[test]
fn sg3_grant_lifecycle_columns_persist_and_rebuild_identically() {
    // SG3: the created_at/expires_at/revoked_at columns on a CapabilityGrant
    // projection persist to the `capability_grants` table AND rebuild
    // byte-identically from the durable projection_records log on restart.
    let store = temp_store("sg3-grant-columns");
    let grant = CapabilityGrantProjection {
        capability_grant_id: "grant-sg3-columns".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: "[\"filesystem:write:workspace\"]".to_string(),
        effect: "allow".to_string(),
        subject_json: "{\"session_id\":\"session-sg3\"}".to_string(),
        decision_source: "allow_trusted_local_profile".to_string(),
        persistence: "until_time".to_string(),
        explanation: "bounded write grant".to_string(),
        created_at: Some("1700000000000".to_string()),
        expires_at: Some("1700003600000".to_string()),
        revoked_at: None,
        updated_sequence: 0,
    };

    store
        .append_event(
            NewEvent::new("event-sg3-grant", EventKind::CapabilityGrantCreated, "test"),
            &[ProjectionRecord::CapabilityGrant(grant.clone())],
        )
        .expect("append grant");

    let live = store.capability_grants().expect("read grants");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].created_at.as_deref(), Some("1700000000000"));
    assert_eq!(live[0].expires_at.as_deref(), Some("1700003600000"));
    assert_eq!(live[0].revoked_at, None);
    // Single-grant read-back accessor also returns the columns.
    let by_id = store
        .capability_grant_by_id("grant-sg3-columns")
        .expect("grant by id")
        .expect("grant present");
    assert_eq!(by_id.expires_at.as_deref(), Some("1700003600000"));

    // Restart/replay: a fresh store over the same root rebuilds the timestamp
    // columns identically purely from the durable event/projection log.
    store.rebuild_projections().expect("rebuild");
    let rebuilt = store
        .capability_grants()
        .expect("read grants after rebuild");
    assert_eq!(rebuilt, live);
}

#[test]
fn sg3_revoked_and_expired_grant_state_rebuilds_identically_from_the_log() {
    // SG3 verification: a rebuild/replay reconstructs revoked AND expired grant
    // state identically from the event log. We append create -> revoke for one
    // grant and a bounded `expires_at` for another, then rebuild and assert the
    // reconstructed projections match the live ones (and carry the lifecycle
    // timestamps), with the original grant-created events preserved.
    let store = temp_store("sg3-revoke-expire-replay");

    let created = CapabilityGrantProjection {
        capability_grant_id: "grant-sg3-revoked".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: "[\"shell:execute:workspace\"]".to_string(),
        effect: "allow".to_string(),
        subject_json: "{\"session_id\":\"session-sg3\"}".to_string(),
        decision_source: "allow_trusted_local_profile".to_string(),
        persistence: "until_revoked".to_string(),
        explanation: "shell grant".to_string(),
        created_at: Some("1700000000000".to_string()),
        expires_at: None,
        revoked_at: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent::new(
                "event-sg3-revoke-create",
                EventKind::CapabilityGrantCreated,
                "test",
            ),
            &[ProjectionRecord::CapabilityGrant(created.clone())],
        )
        .expect("append create");
    // A used event before revocation; it must remain unchanged after revoke.
    store
        .append_event(
            NewEvent::new(
                "event-sg3-revoke-use",
                EventKind::CapabilityGrantUsed,
                "test",
            ),
            &[],
        )
        .expect("append use");
    // Revoke: stamp revoked_at on a re-emitted projection. Old events stay.
    let mut revoked = created.clone();
    revoked.revoked_at = Some("1700000500000".to_string());
    revoked.explanation = "revoked: stricter policy".to_string();
    store
        .append_event(
            NewEvent::new(
                "event-sg3-revoke-revoke",
                EventKind::CapabilityGrantRevoked,
                "test",
            ),
            &[ProjectionRecord::CapabilityGrant(revoked.clone())],
        )
        .expect("append revoke");

    // A second grant that carries a bounded `expires_at` (expiry as a denial
    // input is evaluated at decide time from this column).
    let expiring = CapabilityGrantProjection {
        capability_grant_id: "grant-sg3-expiring".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: "[\"network:connect:internet\"]".to_string(),
        effect: "allow".to_string(),
        subject_json: "{\"session_id\":\"session-sg3\"}".to_string(),
        decision_source: "allow_trusted_local_profile".to_string(),
        persistence: "until_time".to_string(),
        explanation: "bounded network grant".to_string(),
        created_at: Some("1700000000000".to_string()),
        expires_at: Some("1700000000500".to_string()),
        revoked_at: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent::new(
                "event-sg3-expiring-create",
                EventKind::CapabilityGrantCreated,
                "test",
            ),
            &[ProjectionRecord::CapabilityGrant(expiring.clone())],
        )
        .expect("append expiring");

    let live = store.capability_grants().expect("read grants");
    let revoked_live = live
        .iter()
        .find(|grant| grant.capability_grant_id == "grant-sg3-revoked")
        .expect("revoked grant present");
    assert_eq!(revoked_live.revoked_at.as_deref(), Some("1700000500000"));
    assert!(revoked_live.is_revoked());
    // A revoked allow grant is no longer an authorization.
    assert!(!revoked_live.is_active_allow("1700000600000"));

    let expiring_live = live
        .iter()
        .find(|grant| grant.capability_grant_id == "grant-sg3-expiring")
        .expect("expiring grant present");
    // Before expiry it authorizes; after `expires_at` it does not (expiry as a
    // denial input), with no explicit revoke.
    assert!(expiring_live.is_active_allow("1700000000400"));
    assert!(!expiring_live.is_active_allow("1700000000600"));
    assert!(expiring_live.is_expired("1700000000600"));

    // The original grant-created and grant-used events are preserved unchanged.
    let event_count = store.event_count().expect("event count");
    assert_eq!(event_count, 4);

    // Restart/replay: rebuild from the durable log reconstructs the revoked and
    // expired state identically.
    store.rebuild_projections().expect("rebuild");
    let rebuilt = store
        .capability_grants()
        .expect("read grants after rebuild");
    assert_eq!(rebuilt, live);
}

#[test]
fn sg7_run_scored_event_kind_round_trips() {
    // SG7: the new run-score event kind has a stable wire string and round-trips
    // through `as_str`/`from_wire`.
    assert_eq!(EventKind::RunScored.as_str(), "run.scored");
    assert_eq!(
        EventKind::from_wire("run.scored"),
        Some(EventKind::RunScored)
    );
}

#[test]
fn sg7_run_score_projection_persists_and_rebuilds_identically() {
    // SG7: a RunScore projection persists to the `run_scores` table AND rebuilds
    // byte-identically from the durable projection_records log on restart, so the
    // scored outcome is queryable and reproducible across a server restart.
    let store = temp_store("sg7-run-score");
    let score = RunScoreProjection {
        run_score_id: "run-score-run-sg7-abc123".to_string(),
        project_id: ProjectId::new("project-capo"),
        task_id: Some(TaskId::new("task-sg7")),
        session_id: SessionId::new("session-sg7"),
        run_id: RunId::new("run-sg7"),
        outcome: "passed".to_string(),
        passed: true,
        criteria_total: 2,
        criteria_met: 2,
        observed_evidence_count: 2,
        started_at: 1_700_000_000_000,
        completed_at: 1_700_000_003_500,
        duration_millis: 3_500,
        score_inputs_json:
            "{\"criteria\":[],\"observed_verdicts\":[],\"source\":\"observed-runner\"}".to_string(),
        updated_sequence: 0,
    };

    store
        .append_event(
            NewEvent::new("event-sg7-run-score", EventKind::RunScored, "test"),
            &[ProjectionRecord::RunScore(score.clone())],
        )
        .expect("append run score");

    // Queryable both by id and per-session.
    let by_id = store
        .run_score_by_id("run-score-run-sg7-abc123")
        .expect("run score by id")
        .expect("score present");
    assert_eq!(by_id.outcome, "passed");
    assert!(by_id.passed);
    assert_eq!(by_id.duration_millis, 3_500);
    assert_eq!(by_id.criteria_total, 2);
    assert_eq!(by_id.observed_evidence_count, 2);
    let live = store
        .run_scores_for_session(&SessionId::new("session-sg7"))
        .expect("scores for session");
    assert_eq!(live.len(), 1);

    // Restart/replay: a rebuild over the durable log reconstructs the score row
    // identically (the wall-clock timing and inputs survive verbatim).
    store.rebuild_projections().expect("rebuild");
    let rebuilt = store
        .run_scores_for_session(&SessionId::new("session-sg7"))
        .expect("scores after rebuild");
    assert_eq!(rebuilt, live);
}

#[test]
fn sg8_checkpoint_event_kinds_round_trip() {
    // SG8: the checkpoint event kinds have stable wire strings and round-trip
    // through `as_str`/`from_wire` so they survive a rebuild from the log.
    assert_eq!(EventKind::CheckpointCreated.as_str(), "checkpoint.created");
    assert_eq!(
        EventKind::CheckpointRestored.as_str(),
        "checkpoint.restored"
    );
    assert_eq!(
        EventKind::from_wire("checkpoint.created"),
        Some(EventKind::CheckpointCreated)
    );
    assert_eq!(
        EventKind::from_wire("checkpoint.restored"),
        Some(EventKind::CheckpointRestored)
    );
}

#[test]
fn sg8_checkpoint_projection_persists_and_rebuilds_identically() {
    // SG8: a checkpoint projection persists to the `checkpoints` table AND
    // rebuilds byte-identically from the durable projection_records log on
    // restart, so a checkpoint taken before a restart (its restorable commit ref)
    // survives and is still restorable after. The restore event re-emits the SAME
    // row with `restored_at` stamped.
    let store = temp_store("sg8-checkpoint");
    let created = CheckpointProjection {
        checkpoint_id: "checkpoint-run-sg8-abc123".to_string(),
        project_id: ProjectId::new("project-capo"),
        session_id: SessionId::new("session-sg8"),
        run_id: RunId::new("run-sg8"),
        turn_id: Some("turn-1".to_string()),
        kind: "shadow_git".to_string(),
        commit_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        workspace_root: "/work/capo".to_string(),
        shadow_git_dir: "/state/shadow/2f776f726b2f6361706f".to_string(),
        content_hash: "fedcba9876543210fedcba9876543210fedcba98".to_string(),
        created_at: Some("1700000000000".to_string()),
        restored_at: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent::new(
                "event-sg8-checkpoint-created",
                EventKind::CheckpointCreated,
                "test",
            ),
            &[ProjectionRecord::Checkpoint(created.clone())],
        )
        .expect("append checkpoint created");

    // Restore re-emits the same row with restored_at stamped.
    let mut restored = created.clone();
    restored.restored_at = Some("1700000005000".to_string());
    store
        .append_event(
            NewEvent::new(
                "event-sg8-checkpoint-restored",
                EventKind::CheckpointRestored,
                "test",
            ),
            &[ProjectionRecord::Checkpoint(restored.clone())],
        )
        .expect("append checkpoint restored");

    // Queryable by id and per-run, reflecting the restored state.
    let by_id = store
        .checkpoint_by_id("checkpoint-run-sg8-abc123")
        .expect("checkpoint by id")
        .expect("checkpoint present");
    assert_eq!(by_id.commit_ref, created.commit_ref);
    assert!(by_id.is_restored(), "restored_at stamped");
    let live = store
        .checkpoints_for_run(&RunId::new("run-sg8"))
        .expect("checkpoints for run");
    assert_eq!(live.len(), 1, "restore updates the SAME row in place");

    // Restart/replay: a rebuild over the durable log reconstructs the checkpoint
    // row identically (commit ref + content hash + restored_at survive verbatim).
    store.rebuild_projections().expect("rebuild");
    let rebuilt = store
        .checkpoints_for_run(&RunId::new("run-sg8"))
        .expect("checkpoints after rebuild");
    assert_eq!(rebuilt, live);
}

fn temp_store(name: &str) -> SqliteStateStore {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("capo-state-{name}-{nanos}"));
    SqliteStateStore::open(root).expect("open temp store")
}

fn reviewed_memory_record(
    project_id: &ProjectId,
    memory_record_id: &str,
    source_count: i64,
) -> MemoryRecordProjection {
    MemoryRecordProjection {
        memory_record_id: memory_record_id.to_string(),
        project_id: project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: project_id.to_string(),
        subject_ref: Some("workpads/prototype/knowledge.md".to_string()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "fact".to_string(),
        subject: "prototype gate".to_string(),
        predicate: "requires".to_string(),
        object: "source-linked memory".to_string(),
        body: "Prototype memory must stay source linked.".to_string(),
        confidence: "high".to_string(),
        review_state: "reviewed".to_string(),
        source_count,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Safe.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{memory_record_id}")),
        updated_sequence: 0,
    }
}

/// Append a minimal, distinctly-keyed event for the ST4 event-tail tests.
fn append_tail_event(store: &SqliteStateStore, project_id: &ProjectId, ordinal: usize) -> i64 {
    store
        .append_event(
            NewEvent {
                event_id: format!("event-tail-{ordinal}"),
                kind: EventKind::SessionSummaryUpdated,
                actor: "tail-test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(SessionId::new("session-tail")),
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: format!("{{\"ordinal\":{ordinal}}}"),
                idempotency_key: Some(format!("tail:{ordinal}")),
                redaction_state: RedactionState::Safe,
            },
            &[],
        )
        .expect("append tail event")
}

#[test]
fn events_after_returns_only_events_strictly_after_the_watermark_in_order() {
    let store = temp_store("events-after");
    let project_id = ProjectId::new("project-capo");

    let mut sequences = Vec::new();
    for ordinal in 0..5 {
        sequences.push(append_tail_event(&store, &project_id, ordinal));
    }
    // Sequences are monotonic and strictly increasing (the append-only log).
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "sequences must be strictly increasing: {sequences:?}"
    );

    // A watermark in the middle returns exactly the events after it, in order.
    let watermark = sequences[1];
    let after = store
        .events_after(watermark, 1024)
        .expect("events_after returns");
    let returned: Vec<i64> = after.iter().map(|event| event.sequence).collect();
    assert_eq!(returned, sequences[2..].to_vec());
    assert!(
        after.iter().all(|event| event.sequence > watermark),
        "every returned event must be strictly after the watermark"
    );

    // A watermark of 0 returns the whole log (no event has sequence 0).
    let from_zero = store.events_after(0, 1024).expect("events_after(0)");
    assert_eq!(
        from_zero
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences,
    );

    // A watermark at/after the tail returns nothing (no gap-filling, no replay).
    let tail = *sequences.last().expect("at least one event");
    assert!(
        store
            .events_after(tail, 1024)
            .expect("after tail")
            .is_empty(),
        "no events exist after the latest sequence"
    );

    // The `limit` bounds the catch-up page; callers advance the watermark to page.
    let first_page = store.events_after(0, 2).expect("first page");
    assert_eq!(
        first_page
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences[..2].to_vec(),
    );
}

#[test]
fn committed_events_fan_out_to_live_subscribers_after_append() {
    let store = temp_store("events-broadcast");
    let project_id = ProjectId::new("project-capo");

    // Subscribing before any write means the subscriber sees every event the
    // store commits, fanned out after the transaction commits.
    let subscription = store.event_broadcaster().subscribe();
    let seq0 = append_tail_event(&store, &project_id, 0);
    let seq1 = append_tail_event(&store, &project_id, 1);

    let delivered = subscription.drain_pending();
    let delivered_sequences: Vec<i64> = delivered.iter().map(|event| event.sequence).collect();
    assert_eq!(delivered_sequences, vec![seq0, seq1]);
    // A live-delivered event is identical to the catch-up read for that sequence.
    let backlog = store.events_after(0, 1024).expect("backlog");
    assert_eq!(delivered, backlog);

    // A duplicate (idempotent) append commits nothing new and fans nothing out.
    append_tail_event(&store, &project_id, 1);
    assert!(
        subscription.drain_pending().is_empty(),
        "an idempotent no-op append must not be broadcast"
    );

    // Dropping the subscription unsubscribes it on the next publish (pruned).
    drop(subscription);
    append_tail_event(&store, &project_id, 2);
    assert_eq!(
        store.event_broadcaster().subscriber_count(),
        0,
        "a dropped subscriber is pruned from the fan-out on the next publish"
    );
}

/// Append a turn-keyed conversation event for the ST5 thread-projection tests.
fn append_turn_event(
    store: &SqliteStateStore,
    session_id: &SessionId,
    event_id: &str,
    kind: EventKind,
    turn_id: &str,
    payload_json: &str,
) -> i64 {
    store
        .append_event(
            NewEvent {
                event_id: event_id.to_string(),
                kind,
                actor: "thread-test".to_string(),
                project_id: Some(ProjectId::new("project-capo")),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: Some(turn_id.to_string()),
                item_id: None,
                payload_json: payload_json.to_string(),
                idempotency_key: Some(format!("thread:{event_id}")),
                redaction_state: RedactionState::Safe,
            },
            &[],
        )
        .expect("append turn event")
}

#[test]
fn session_thread_rebuilds_identically_from_the_persisted_log_on_restart() {
    let store = temp_store("session-thread");
    let session_id = SessionId::new("session-thread");

    // A scripted two-turn conversation persisted to the durable event log.
    append_turn_event(
        &store,
        &session_id,
        "te1",
        EventKind::SessionSummaryUpdated,
        "turn-a",
        "{\"latest_summary\":\"first reply\"}",
    );
    append_turn_event(
        &store,
        &session_id,
        "te2",
        EventKind::ToolCallCompleted,
        "turn-a",
        "{\"tool_name\":\"shell\",\"status\":\"completed\"}",
    );
    append_turn_event(
        &store,
        &session_id,
        "te3",
        EventKind::EvidenceRecorded,
        "turn-a",
        "{\"detail\":\"turn done\"}",
    );
    append_turn_event(
        &store,
        &session_id,
        "te4",
        EventKind::SessionSummaryUpdated,
        "turn-b",
        "{\"latest_summary\":\"second reply\"}",
    );

    let thread = store
        .session_thread(&session_id, 0, 1024)
        .expect("session thread");
    assert_eq!(thread.turns.len(), 2);
    assert_eq!(thread.turns[0].turn_id, "turn-a");
    assert_eq!(thread.turns[0].status, ThreadTurnStatus::Completed);
    assert_eq!(thread.turns[1].turn_id, "turn-b");
    assert_eq!(thread.turns[1].status, ThreadTurnStatus::InProgress);

    // Restart/replay: a fresh store over the same root reconstructs the
    // identical thread purely from the durable log (rebuildable read model).
    let reopened = SqliteStateStore::open(store.db_path().parent().expect("db parent"))
        .expect("reopen state store");
    let rebuilt = reopened
        .session_thread(&session_id, 0, 1024)
        .expect("rebuilt session thread");
    assert_eq!(thread, rebuilt);

    // Incremental read composes with a tail: reading after turn-a's last
    // sequence yields only turn-b, carrying the watermark through.
    let watermark = thread.turns[0].last_sequence;
    let tail = store
        .session_thread(&session_id, watermark, 1024)
        .expect("incremental thread");
    assert_eq!(tail.since_sequence, watermark);
    assert_eq!(tail.turns.len(), 1);
    assert_eq!(tail.turns[0].turn_id, "turn-b");
}

#[test]
fn sg5_workspace_lease_event_kinds_round_trip() {
    // SG5: the single-writer workspace-lock event kinds have stable wire strings
    // and round-trip through `as_str`/`from_wire`.
    assert_eq!(
        EventKind::WorkspaceLeaseAcquired.as_str(),
        "workspace.lease_acquired"
    );
    assert_eq!(
        EventKind::WorkspaceLeaseReleased.as_str(),
        "workspace.lease_released"
    );
    assert_eq!(
        EventKind::from_wire("workspace.lease_acquired"),
        Some(EventKind::WorkspaceLeaseAcquired)
    );
    assert_eq!(
        EventKind::from_wire("workspace.lease_released"),
        Some(EventKind::WorkspaceLeaseReleased)
    );
}

#[test]
fn sg5_workspace_lease_projection_persists_and_rebuilds_identically() {
    // SG5: a WorkspaceLease projection persists to the `workspace_leases` table
    // AND rebuilds identically from the durable projection_records log on
    // restart, so the single-writer lock survives a server restart.
    let store = temp_store("sg5-lease-columns");
    let project_id = ProjectId::new("project-capo");

    // Acquire: held by session-holder.
    let held = WorkspaceLeaseProjection {
        workspace_lease_id: "workspace-lease-capo".to_string(),
        project_id: project_id.clone(),
        holder_session_id: SessionId::new("session-holder"),
        holder_run_id: Some(RunId::new("run-holder")),
        status: WorkspaceLeaseProjection::HELD.to_string(),
        acquired_at: Some("1700000000000".to_string()),
        released_at: None,
        release_reason: String::new(),
        updated_sequence: 0,
    };
    store
        .append_event(
            NewEvent::new(
                "event-sg5-acquire",
                EventKind::WorkspaceLeaseAcquired,
                "test",
            ),
            &[ProjectionRecord::WorkspaceLease(held.clone())],
        )
        .expect("append acquire");

    let live = store.workspace_leases(&project_id).expect("leases");
    assert_eq!(live.len(), 1);
    assert!(live[0].is_held());
    assert!(live[0].is_held_by(&SessionId::new("session-holder")));
    assert!(!live[0].is_held_by(&SessionId::new("session-other")));

    // Single-lease accessor returns the same row.
    let by_id = store
        .workspace_lease_by_id("workspace-lease-capo")
        .expect("by id")
        .expect("present");
    assert_eq!(by_id, live[0]);

    // Release: re-emit the SAME row with released_at + reason stamped.
    let mut released = held.clone();
    released.status = WorkspaceLeaseProjection::RELEASED.to_string();
    released.released_at = Some("1700000500000".to_string());
    released.release_reason = "turn complete".to_string();
    store
        .append_event(
            NewEvent::new(
                "event-sg5-release",
                EventKind::WorkspaceLeaseReleased,
                "test",
            ),
            &[ProjectionRecord::WorkspaceLease(released.clone())],
        )
        .expect("append release");

    let after_release = store.workspace_leases(&project_id).expect("leases");
    assert_eq!(after_release.len(), 1);
    assert!(!after_release[0].is_held(), "released lease reads as free");
    assert_eq!(after_release[0].release_reason, "turn complete");

    // Restart/replay: a rebuild reconstructs the lease state identically from the
    // event log (the acquire + release events yield the same released lease).
    store.rebuild_projections().expect("rebuild");
    let rebuilt = store.workspace_leases(&project_id).expect("leases rebuilt");
    assert_eq!(rebuilt, after_release);
    assert!(!rebuilt[0].is_held());
}
