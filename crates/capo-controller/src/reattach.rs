//! GA6 (goal-orchestration GO13): reattach-after-compaction.
//!
//! This module carries the GA6 deterministic tests. GA6 has no NEW runtime type of
//! its own: the load-bearing GA6 properties are realized by code that already
//! exists -- the objective + audit contract are reconstructed from persisted goal
//! state by the GA3 continuation context packet
//! ([`crate::continuation_context`]), the auditor and scheduler read that same
//! persisted state ([`crate::completion_auditor`] / [`crate::continuation_scheduler`]),
//! and the cross-attempt observed-evidence reads are task-scoped so a fresh attempt
//! session does not drop prior-attempt evidence. The remaining GA6 GAP was an
//! ARTIFACT-OVERWRITE bug: the adapter-replay `adapter.turn_completed` evidence row
//! was keyed only by `(adapter_kind, session_id)`, so successive provider turns in
//! one session collapsed onto a single evidence row -- the next turn's
//! `ON CONFLICT(evidence_id) DO UPDATE` destroyed the prior turn's observed
//! evidence (the observed `stdout.txt`-reuse pattern). [`crate::adapter_replay`]
//! now keys that row PER TURN, so every turn's evidence remains recoverable for the
//! auditor and historical report across restart + rebuild.
//!
//! These tests prove, with deterministic scripted/seeded agents and no live
//! provider:
//!
//! - multiple provider turns do NOT overwrite each other's observed evidence, and
//!   every turn's evidence survives a server restart + full projection rebuild;
//! - after an adapter/provider SESSION restart (a continuation rebinds the goal to
//!   a fresh attempt session) the active objective, success criteria, and audit
//!   contract re-inject from persisted goal state -- not from any in-memory
//!   transcript -- and the auditor and scheduler operate on the rebuilt state.

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent, ScriptedMockTurn};
    use capo_core::{
        AgentId, EvidenceId, GoalId, ProjectId, RequirementId, RunId, SessionId, TaskId,
    };
    use capo_state::{
        EventKind, EvidenceProjection, GoalProjection, NewEvent, ProjectionRecord,
        RequirementLedgerProjection, SqliteStateStore,
    };

    use crate::{ContinuationConditions, ContinuationDecision, FakeBoundaryController, GoalBudget};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-ga6-{name}-{nanos}-{n}"))
    }

    const PROJECT: &str = "project-capo";

    /// GA6: multiple provider turns must NOT overwrite each other's observed
    /// evidence, and every turn's evidence must survive a server restart + full
    /// projection rebuild.
    ///
    /// This is the artifact-retention test the GA6 Verification names: three
    /// scripted mock turns each emit a terminal `adapter.turn_completed`, which the
    /// adapter-replay projects as an OBSERVED evidence row. Before the GA6 fix the
    /// evidence id was keyed only by `(adapter_kind, session_id)`, so all three
    /// turns collapsed onto ONE row (the latest turn's `ON CONFLICT DO UPDATE`
    /// destroyed the earlier turns' evidence). With per-turn keying all three rows
    /// persist and rebuild identically.
    #[test]
    fn reattach_multiple_provider_turns_do_not_overwrite_earlier_turn_evidence() {
        let state_root = temp_root("turn-evidence");
        let controller = FakeBoundaryController::open_with_adapter(
            ProjectId::new(PROJECT),
            &state_root,
            AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new("reattach-evidence-session")),
        )
        .expect("open controller");
        let registration = controller
            .register_agent("reattach-evidence-worker")
            .expect("agent");
        let refs = controller
            .send_task(&registration, "Run three deterministic provider turns")
            .expect("send task");

        // Three distinct provider turns, each terminating with its own
        // `turn_completed`. These all run on the SAME session/run -- exactly the
        // case that previously overwrote prior-turn evidence.
        for turn in ["turn-1", "turn-2", "turn-3"] {
            let batch = ScriptedMockTurn::new(turn)
                .message_completed(format!("msg-{turn}"), format!("work for {turn}"))
                .turn_completed(format!("done-{turn}"))
                .normalized_events(&refs.external_session_ref);
            controller
                .apply_normalized_adapter_events_with_turn(&refs, &batch, Some(turn))
                .unwrap_or_else(|err| panic!("apply {turn}: {err:?}"));
        }

        // Every turn's observed evidence is present: three DISTINCT
        // `adapter_replay:mock` evidence rows, one per turn, not one collapsed row.
        let evidence_before = controller
            .state()
            .evidence_for_session(&refs.session_id)
            .expect("evidence");
        let replay_evidence: Vec<&EvidenceProjection> = evidence_before
            .iter()
            .filter(|row| row.kind == "adapter_replay:mock")
            .collect();
        assert_eq!(
            replay_evidence.len(),
            3,
            "each provider turn keeps its own observed evidence row (no overwrite); \
             got {replay_evidence:?}"
        );
        let distinct_ids: std::collections::BTreeSet<&str> = replay_evidence
            .iter()
            .map(|row| row.evidence_id.as_str())
            .collect();
        assert_eq!(
            distinct_ids.len(),
            3,
            "the per-turn evidence ids are distinct, so no turn overwrites another"
        );

        // The auditor and historical report depend on this evidence surviving a
        // restart. Reopen over the same state root and rebuild every projection from
        // the event log; all three turn-evidence rows must rebuild identically.
        drop(controller);
        let reopened = SqliteStateStore::open(&state_root).expect("reopen state");
        reopened.rebuild_projections().expect("rebuild projections");
        let evidence_after = reopened
            .evidence_for_session(&refs.session_id)
            .expect("evidence after rebuild");
        assert_eq!(
            evidence_after, evidence_before,
            "every provider turn's evidence rebuilds identically after restart"
        );
        let replay_after = evidence_after
            .iter()
            .filter(|row| row.kind == "adapter_replay:mock")
            .count();
        assert_eq!(
            replay_after, 3,
            "all three turns' evidence is still recoverable after restart + rebuild"
        );
    }

    /// GA6: after an adapter/provider SESSION restart, the objective + success
    /// criteria + audit contract re-inject from PERSISTED goal state, and the
    /// auditor and scheduler operate on the rebuilt state without any in-memory
    /// transcript.
    ///
    /// A continuation rebinds the goal to a fresh attempt session (the
    /// `ON CONFLICT(goal_id) DO UPDATE SET session_id = ...` path) -- this is the
    /// adapter/provider session restart. We then drop the controller, reopen over
    /// the same state root, and rebuild ALL projections from the event log (the
    /// server restart / compaction). After both, the reconstructed continuation
    /// packet, the scheduler decision, and the auditor verdict must all be derivable
    /// purely from persisted state.
    #[test]
    fn reattach_reinjects_objective_and_audit_contract_after_session_restart_and_rebuild() {
        let state_root = temp_root("reinject");
        let goal_id = GoalId::new("goal-reattach");
        let task_id = TaskId::new("task-reattach");
        let session_1 = SessionId::new("session-reattach-attempt-1");
        let session_2 = SessionId::new("session-reattach-attempt-2");
        let agent_id = AgentId::new("agent-reattach");
        let req_id = RequirementId::new("req-tests-pass");

        let controller =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("controller");

        let goal_in_session = |session: &SessionId, run: &str| GoalProjection {
            goal_id: goal_id.clone(),
            project_id: ProjectId::new(PROJECT),
            task_id: Some(task_id.clone()),
            agent_id: Some(agent_id.clone()),
            session_id: Some(session.clone()),
            parent_goal_id: None,
            attempt_run_id: Some(RunId::new(run)),
            objective: "Ship the reattach milestone".to_string(),
            status: GoalProjection::ACTIVE.to_string(),
            success_criteria_json: r#"{"must":["all tests pass"]}"#.to_string(),
            constraints_json: r#"{"no_network":true}"#.to_string(),
            verification_surface_json: r#"{"cmd":"cargo test"}"#.to_string(),
            budget_json: r#"{"max_turns":8}"#.to_string(),
            stop_conditions_json: r#"{"on":"blocker"}"#.to_string(),
            blocker_reason: String::new(),
            updated_sequence: 0,
        };

        let seed = |event_id: &str, kind: EventKind, records: &[ProjectionRecord]| {
            let mut event = NewEvent::new(event_id, kind, "test-seed");
            event.project_id = Some(ProjectId::new(PROJECT));
            event.idempotency_key = Some(event_id.to_string());
            event.item_id = Some(event_id.to_string());
            controller
                .state()
                .append_event(event, records)
                .expect("seed append");
        };

        // Attempt 1: the goal is created bound to session 1, with one requirement
        // and one OBSERVED evidence row tagged to the goal's task (the stable
        // cross-attempt key).
        seed(
            "seed-goal",
            EventKind::GoalCreated,
            &[
                ProjectionRecord::Goal(goal_in_session(&session_1, "run-attempt-1")),
                ProjectionRecord::RequirementLedger(RequirementLedgerProjection {
                    requirement_id: req_id.clone(),
                    goal_id: goal_id.clone(),
                    project_id: ProjectId::new(PROJECT),
                    summary: "All tests pass".to_string(),
                    status: RequirementLedgerProjection::VALIDATED.to_string(),
                    last_status_source: "runtime_output".to_string(),
                    updated_sequence: 0,
                }),
            ],
        );
        seed(
            "seed-attempt1-evidence",
            EventKind::EvidenceRecorded,
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: EvidenceId::new("attempt1-observed-evidence"),
                project_id: ProjectId::new(PROJECT),
                task_id: Some(task_id.clone()),
                session_id: Some(session_1.clone()),
                run_id: Some(RunId::new("run-attempt-1")),
                kind: "test".to_string(),
                artifact_id: None,
                confidence: 100,
                updated_sequence: 0,
            })],
        );

        // Capture the objective + audit contract BEFORE the restart, as the live
        // running session sees it.
        let before = controller
            .continuation_context_packet(&goal_id)
            .expect("packet before restart");
        assert_eq!(
            before.audit_contract.objective,
            "Ship the reattach milestone"
        );

        // The adapter/provider SESSION restarts: a continuation rebinds the SAME
        // goal to a fresh attempt session (and run). This is the
        // `ON CONFLICT(goal_id) DO UPDATE SET session_id = ...` path.
        seed(
            "seed-rebind",
            EventKind::GoalUpdated,
            &[ProjectionRecord::Goal(goal_in_session(
                &session_2,
                "run-attempt-2",
            ))],
        );

        // The SERVER restarts: drop the controller, reopen over the same state root,
        // and rebuild every projection from the event log. Nothing in-memory
        // survives.
        drop(controller);
        let restarted =
            FakeBoundaryController::open(ProjectId::new(PROJECT), &state_root).expect("restarted");
        restarted
            .state()
            .rebuild_projections()
            .expect("rebuild projections");

        // 1) The objective, success criteria, and audit contract re-inject from
        //    persisted goal state -- identical to before, even though the goal is now
        //    bound to a different attempt session and nothing in-memory survived.
        let after = restarted
            .continuation_context_packet(&goal_id)
            .expect("packet after restart");
        assert_eq!(
            after.audit_contract.objective,
            "Ship the reattach milestone"
        );
        assert_eq!(
            after.audit_contract.success_criteria_json,
            r#"{"must":["all tests pass"]}"#
        );
        assert_eq!(
            before.audit_contract, after.audit_contract,
            "the objective + audit contract survive the session restart + rebuild byte-for-byte"
        );
        // The rebuilt prompt is grounded in the objective from persisted state, not
        // a model transcript.
        assert!(
            after
                .render_prompt()
                .contains("Objective: Ship the reattach milestone")
        );

        // 2) The prior-attempt OBSERVED evidence is still reachable after the
        //    session rebind: it was tagged to the stable task id, so the new attempt
        //    session does not drop it.
        assert!(
            after
                .fragments
                .iter()
                .any(|fragment| fragment.source_ref == "attempt1-observed-evidence"),
            "prior-attempt observed evidence survives the session restart"
        );

        // 3) The AUDITOR operates on the rebuilt state without any transcript: the
        //    single requirement is `validated` AND backed by observed evidence, so
        //    the goal audits COMPLETE purely from persisted projections.
        let verdict = restarted
            .audit_goal_completion(&goal_id)
            .expect("audit after restart");
        assert!(
            verdict.verdict.is_complete(),
            "the auditor completes the goal from rebuilt observed evidence, not a transcript: {verdict:?}"
        );

        // 4) The SCHEDULER also operates on the rebuilt state: the goal is active, so
        //    a safe-boundary evaluation continues -- decided purely from persisted
        //    state plus the caller's live conditions.
        let conditions = ContinuationConditions {
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
        };
        let outcome = restarted
            .evaluate_continuation(&goal_id, &conditions, None)
            .expect("scheduler after restart");
        assert_eq!(
            outcome.decision,
            ContinuationDecision::Continue,
            "the scheduler decides from rebuilt persisted goal state without an in-memory transcript"
        );
    }
}
