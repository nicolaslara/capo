//! SG7: `score_run` -- the run outcome signal, scored from OBSERVED evidence
//! only, with real wall-clock timing.
//!
//! Where it lives (the SG7 open question, resolved): `score_run` lives in
//! `capo-controller`, the LOOP owner, beside the SG6 `VerificationRunner` gate
//! that produces the observed evidence it consumes. SG6 already resolved that
//! the verification GATE belongs to the loop owner (not `capo-eval`, the
//! descriptive reporting layer, and not `capo-server`, transport) because the
//! loop is what decides whether a run passed. SG7 follows the same boundary:
//! the score is the loop's verdict over the gate's observed evidence, so the
//! computation that derives the verdict sits with the gate, not in the
//! descriptive report. `capo-eval` may later RENDER a stored `RunScoreProjection`
//! into its markdown roll-up, but it does not compute the score.
//!
//! What `score_run` does and, critically, what it REFUSES to trust:
//!
//! - It reads the OBSERVED verification evidence the SG6 gate persisted for the
//!   run -- `evidence.recorded` events whose actor is
//!   [`VERIFICATION_EVIDENCE_ACTOR`] AND whose payload `source` is
//!   [`VERIFICATION_EVIDENCE_SOURCE`]. Every other evidence event (an
//!   agent-reported summary, an adapter-reported claim, anything not stamped by
//!   the observed runner) is FILTERED OUT before scoring and cannot contribute.
//!   This is the anti-spoofing boundary the SG7 acceptance requires: injecting
//!   only agent-reported claims never raises the score.
//! - It compares the observed verdicts to a set of acceptance criteria. A
//!   criterion is MET only when an observed verdict of its required kind PASSED
//!   (exit status 0, re-derived by the gate, never an agent claim). A criterion
//!   with no observed evidence, or whose only observed verdict failed, is unmet.
//! - The run `passed` iff every required criterion was met by observed passing
//!   evidence.
//! - It records REAL wall-clock timing (`started_at`/`completed_at`, supplied by
//!   the caller's clock) and the derived `duration_millis`, replacing the
//!   `capo-eval` event-sequence-delta "duration."
//! - It persists the score and its inputs as a durable `run.scored` event +
//!   [`capo_state::RunScoreProjection`], so the outcome is queryable and survives
//!   restart, and the score-inputs digest makes the score REPRODUCIBLE: rebuilding
//!   from the event log yields the same score for the same observed evidence.

use capo_state::{EventRecord, RunScoreProjection};

use super::*;

/// One acceptance criterion the run is scored against.
///
/// A criterion is MET only when the OBSERVED verification evidence for the run
/// contains at least one PASSED verdict of [`Self::required_kind`]. The
/// human-readable [`Self::label`] is recorded in the score inputs so the
/// outcome is auditable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptanceCriterion {
    /// A stable label for the criterion (e.g. "cargo test passes").
    pub label: String,
    /// The verification kind whose observed PASS satisfies this criterion.
    pub required_kind: VerificationKind,
}

impl AcceptanceCriterion {
    pub fn new(label: impl Into<String>, required_kind: VerificationKind) -> Self {
        Self {
            label: label.into(),
            required_kind,
        }
    }
}

/// SG7: where a run-score hangs on the loop's scope tree, plus the wall-clock
/// window the scored run occupied.
///
/// `started_at`/`completed_at` are wall-clock millis-since-epoch supplied by the
/// caller's clock (a real clock in the loop, a controlled clock in tests), so
/// the scored outcome carries REAL timing instead of the event-sequence delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunScoreScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    /// Wall-clock millis-since-epoch the scored run started.
    pub started_at: i64,
    /// Wall-clock millis-since-epoch the scored run completed.
    pub completed_at: i64,
}

/// How one acceptance criterion was scored against the observed evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoredCriterion {
    pub label: String,
    pub required_kind: VerificationKind,
    /// True iff an observed PASSED verdict of `required_kind` satisfied it.
    pub met: bool,
    /// The id of the observed evidence row that satisfied it, when met.
    pub evidence_id: Option<String>,
}

/// SG7: the terminal run outcome signal computed by [`FakeBoundaryController::
/// score_run`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunScore {
    /// True iff every required acceptance criterion was met by observed passing
    /// evidence.
    pub passed: bool,
    /// `passed` / `failed` / `inconclusive`.
    pub outcome: RunScoreOutcome,
    pub criteria_total: usize,
    pub criteria_met: usize,
    /// Per-criterion scoring detail.
    pub scored_criteria: Vec<ScoredCriterion>,
    /// How many OBSERVED verification verdicts were considered (agent-reported
    /// claims are excluded, so they never count here).
    pub observed_evidence_count: usize,
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_millis: i64,
    /// The persisted score projection.
    pub projection: RunScoreProjection,
}

/// The run outcome signal `score_run` produces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunScoreOutcome {
    /// Every required criterion was met by observed passing evidence.
    Passed,
    /// At least one required criterion was unmet (no observed pass, or an
    /// observed fail).
    Failed,
    /// No acceptance criteria were supplied, so there is nothing to score
    /// against. Distinct from `Failed`: an empty criteria set is not a failing
    /// run, it is an un-scorable one.
    Inconclusive,
}

impl RunScoreOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Inconclusive => "inconclusive",
        }
    }
}

/// One observed verification verdict, parsed from an OBSERVED `evidence.recorded`
/// event the SG6 gate persisted.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedVerdict {
    evidence_id: String,
    verification_kind: String,
    passed: bool,
    exit_status: String,
}

impl FakeBoundaryController {
    /// SG7: score a run against acceptance criteria using OBSERVED evidence only.
    ///
    /// Reads the observed verification verdicts the SG6 gate persisted for the
    /// run, filtering out everything that is not stamped by the observed runner
    /// (so agent-reported claims cannot contribute), scores each acceptance
    /// criterion as MET only when an observed PASS of its kind exists, derives the
    /// run `passed` signal and real wall-clock timing, and persists a durable
    /// `run.scored` event + [`RunScoreProjection`]. Re-scoring the SAME observed
    /// evidence is idempotent (same score id, no duplicate row), and the score
    /// rebuilds identically from the event log.
    pub fn score_run(
        &self,
        scope: &RunScoreScope,
        criteria: &[AcceptanceCriterion],
    ) -> StateResult<RunScore> {
        let observed = self.observed_verdicts_for_run(&scope.session_id, &scope.run_id)?;

        // Score each criterion: MET only when an observed PASS of its kind exists.
        let scored_criteria: Vec<ScoredCriterion> = criteria
            .iter()
            .map(|criterion| {
                let kind_label = criterion.required_kind.label();
                let matched = observed
                    .iter()
                    .find(|verdict| verdict.verification_kind == kind_label && verdict.passed);
                ScoredCriterion {
                    label: criterion.label.clone(),
                    required_kind: criterion.required_kind,
                    met: matched.is_some(),
                    evidence_id: matched.map(|verdict| verdict.evidence_id.clone()),
                }
            })
            .collect();

        let criteria_total = scored_criteria.len();
        let criteria_met = scored_criteria.iter().filter(|item| item.met).count();
        let outcome = if criteria_total == 0 {
            RunScoreOutcome::Inconclusive
        } else if criteria_met == criteria_total {
            RunScoreOutcome::Passed
        } else {
            RunScoreOutcome::Failed
        };
        let passed = matches!(outcome, RunScoreOutcome::Passed);
        let duration_millis = scope.completed_at.saturating_sub(scope.started_at).max(0);

        // The score-inputs digest is the reproducibility + audit anchor: it
        // records each criterion, whether observed evidence met it (and which
        // evidence row), and the observed verdicts that fed the score. The same
        // observed evidence + criteria always serialize to the same digest, so
        // the score id is stable and a rebuild reconstructs the score identically.
        let score_inputs = serde_json::json!({
            "source": VERIFICATION_EVIDENCE_SOURCE,
            "criteria": scored_criteria
                .iter()
                .map(|item| serde_json::json!({
                    "label": item.label,
                    "required_kind": item.required_kind.label(),
                    "met": item.met,
                    "evidence_id": item.evidence_id,
                }))
                .collect::<Vec<_>>(),
            "observed_verdicts": observed
                .iter()
                .map(|verdict| serde_json::json!({
                    "evidence_id": verdict.evidence_id,
                    "verification_kind": verdict.verification_kind,
                    "passed": verdict.passed,
                    "exit_status": verdict.exit_status,
                }))
                .collect::<Vec<_>>(),
        });
        let score_inputs_json = score_inputs.to_string();

        // Key the score id on (run, scored inputs) so re-scoring the SAME observed
        // evidence + criteria is idempotent and re-projects identically, while a
        // different verdict set gets a distinct id. Timing is deliberately NOT in
        // the id (a re-score with a different clock window must dedupe on the same
        // observed evidence, not split into a second row).
        let run_score_id = format!(
            "run-score-{}-{}",
            scope.run_id,
            stable_score_hash(&score_inputs_json)
        );

        let projection = RunScoreProjection {
            run_score_id: run_score_id.clone(),
            project_id: self.project_id.clone(),
            task_id: Some(scope.task_id.clone()),
            session_id: scope.session_id.clone(),
            run_id: scope.run_id.clone(),
            outcome: outcome.as_str().to_string(),
            passed,
            criteria_total: criteria_total as i64,
            criteria_met: criteria_met as i64,
            observed_evidence_count: observed.len() as i64,
            started_at: scope.started_at,
            completed_at: scope.completed_at,
            duration_millis,
            score_inputs_json: score_inputs_json.clone(),
            updated_sequence: 0,
        };

        let payload = serde_json::json!({
            "source": VERIFICATION_EVIDENCE_SOURCE,
            "outcome": outcome.as_str(),
            "passed": passed,
            "criteria_total": criteria_total,
            "criteria_met": criteria_met,
            "observed_evidence_count": observed.len(),
            "started_at": scope.started_at,
            "completed_at": scope.completed_at,
            "duration_millis": duration_millis,
            "score_inputs": score_inputs,
        })
        .to_string();

        let event = scoped_event(
            &format!("event-{run_score_id}"),
            EventKind::RunScored,
            &self.project_id,
            &scope.task_id,
            &scope.agent_id,
            &scope.session_id,
            &scope.run_id,
        )
        .with_turn(scope.turn_id.to_string())
        .with_item(run_score_id.clone())
        .with_payload(payload);

        self.state
            .append_event(event, &[ProjectionRecord::RunScore(projection.clone())])?;

        Ok(RunScore {
            passed,
            outcome,
            criteria_total,
            criteria_met,
            scored_criteria,
            observed_evidence_count: observed.len(),
            started_at: scope.started_at,
            completed_at: scope.completed_at,
            duration_millis,
            projection,
        })
    }

    /// Read the OBSERVED verification verdicts the SG6 gate persisted for a run.
    ///
    /// Only events stamped by the observed runner -- actor
    /// [`VERIFICATION_EVIDENCE_ACTOR`] AND payload `source =`
    /// [`VERIFICATION_EVIDENCE_SOURCE`] -- are returned. This is the single point
    /// where agent-reported evidence is excluded: anything not stamped observed is
    /// dropped before it can influence the score.
    fn observed_verdicts_for_run(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> StateResult<Vec<ObservedVerdict>> {
        let events = self.state.recent_events_for_session(session_id, 1000)?;
        let mut verdicts = Vec::new();
        for event in &events {
            if let Some(verdict) = parse_observed_verdict(event, run_id) {
                verdicts.push(verdict);
            }
        }
        Ok(verdicts)
    }
}

/// Parse one observed verification verdict from an event, returning `None` for
/// anything that is not an OBSERVED verification verdict for `run_id`.
///
/// The observed-only filter is enforced here: an event must be
/// `evidence.recorded`, scoped to `run_id`, carry the observed runner's actor,
/// AND carry `source = observed-runner` in its payload. An agent-reported
/// evidence event (different actor / missing or different `source`) returns
/// `None`, so it never reaches the score.
fn parse_observed_verdict(event: &EventRecord, run_id: &RunId) -> Option<ObservedVerdict> {
    if event.kind != EventKind::EvidenceRecorded.as_str() {
        return None;
    }
    if event.run_id.as_ref() != Some(run_id) {
        return None;
    }
    // ANTI-SPOOFING: the actor must be the observed runner. An agent-reported
    // evidence event carries a different actor and is rejected here.
    if event.actor != VERIFICATION_EVIDENCE_ACTOR {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(&event.payload_json).ok()?;
    // Defense in depth: the payload must also self-identify as observed, so a
    // payload that lacks (or forges a different) source is rejected even if the
    // actor column somehow matched.
    if payload.get("source").and_then(serde_json::Value::as_str)
        != Some(VERIFICATION_EVIDENCE_SOURCE)
    {
        return None;
    }
    let verification_kind = payload
        .get("verification_kind")
        .and_then(serde_json::Value::as_str)?
        .to_string();
    let passed = payload
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let exit_status = payload
        .get("exit_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    Some(ObservedVerdict {
        evidence_id: event.item_id.clone()?,
        verification_kind,
        passed,
        exit_status,
    })
}

/// FNV-1a hash for stable run-score ids (no extra dependency; same shape as
/// `stable_verification_hash`).
fn stable_score_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use capo_runtime::LocalProcessConfig;
    use capo_state::SqliteStateStore;

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-sg7-{name}-{nanos}-{n}"))
    }

    fn controller() -> (FakeBoundaryController, PathBuf, PathBuf) {
        let state_root = temp_root("state");
        let workspace = temp_root("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &state_root)
            .expect("controller");
        let artifacts = temp_root("artifacts");
        (controller, workspace, artifacts)
    }

    fn verification_scope() -> VerificationScope {
        VerificationScope {
            task_id: TaskId::new("task-sg7"),
            agent_id: AgentId::new("agent-sg7"),
            session_id: SessionId::new("session-sg7"),
            run_id: RunId::new("run-sg7"),
            turn_id: TurnId::new("turn-sg7"),
        }
    }

    fn score_scope(started_at: i64, completed_at: i64) -> RunScoreScope {
        RunScoreScope {
            task_id: TaskId::new("task-sg7"),
            agent_id: AgentId::new("agent-sg7"),
            session_id: SessionId::new("session-sg7"),
            run_id: RunId::new("run-sg7"),
            turn_id: TurnId::new("turn-sg7"),
            started_at,
            completed_at,
        }
    }

    fn config(workspace: &Path, artifacts: PathBuf) -> LocalProcessConfig {
        LocalProcessConfig::for_test(workspace.to_path_buf(), artifacts)
    }

    /// Run a scripted verification command that exits with `code`, persisting an
    /// OBSERVED verdict the way the SG6 gate does.
    fn observe(
        controller: &FakeBoundaryController,
        workspace: &Path,
        artifacts: PathBuf,
        kind: VerificationKind,
        code: i32,
    ) -> VerificationOutcome {
        controller
            .run_verification(
                &verification_scope(),
                config(workspace, artifacts),
                &VerificationCommand::new(
                    kind,
                    "/bin/sh",
                    vec!["-c".to_string(), format!("printf out; exit {code}")],
                    workspace.to_path_buf(),
                ),
            )
            .expect("run verification")
    }

    #[test]
    fn passing_observed_evidence_scores_the_run_passed() {
        let (controller, workspace, artifacts) = controller();
        // OBSERVED: a real exit-0 verification verdict.
        observe(
            &controller,
            &workspace,
            artifacts,
            VerificationKind::Test,
            0,
        );

        let score = controller
            .score_run(
                &score_scope(1_000, 4_500),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        assert!(
            score.passed,
            "a met criterion over observed pass scores passed"
        );
        assert_eq!(score.outcome, RunScoreOutcome::Passed);
        assert_eq!(score.criteria_total, 1);
        assert_eq!(score.criteria_met, 1);
        assert_eq!(score.observed_evidence_count, 1);
        assert!(score.scored_criteria[0].met);
        assert!(score.scored_criteria[0].evidence_id.is_some());
        assert_eq!(score.projection.outcome, "passed");
        assert!(score.projection.passed);
    }

    #[test]
    fn failing_observed_evidence_scores_the_run_failed() {
        let (controller, workspace, artifacts) = controller();
        // OBSERVED: a real non-zero verification verdict.
        observe(
            &controller,
            &workspace,
            artifacts,
            VerificationKind::Test,
            5,
        );

        let score = controller
            .score_run(
                &score_scope(1_000, 2_000),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        assert!(
            !score.passed,
            "an observed FAIL does not meet the criterion"
        );
        assert_eq!(score.outcome, RunScoreOutcome::Failed);
        assert_eq!(score.criteria_total, 1);
        assert_eq!(score.criteria_met, 0);
        // The failing verdict is still OBSERVED evidence that was considered.
        assert_eq!(score.observed_evidence_count, 1);
        assert!(!score.scored_criteria[0].met);
        assert!(score.scored_criteria[0].evidence_id.is_none());
    }

    #[test]
    fn agent_reported_claims_alone_do_not_raise_the_score() {
        // SG7 anti-spoofing: inject ONLY an agent-reported evidence event that
        // CLAIMS the test passed (right kind, passed=true), but with an agent
        // actor and NOT the observed-runner source. The score must NOT count it,
        // so the criterion stays unmet and the run is not raised to passed.
        let (controller, _workspace, _artifacts) = controller();
        let scope = verification_scope();

        let agent_payload = serde_json::json!({
            // An agent forging the observed shape: right kind, passed, but the
            // source is an agent channel, not the observed runner.
            "source": "agent-reported",
            "verification_kind": "test",
            "command": "cargo test --workspace",
            "passed": true,
            "exit_status": "0",
        })
        .to_string();
        let evidence_id = "evidence-agent-claim";
        let mut event = scoped_event(
            "event-agent-claim",
            EventKind::EvidenceRecorded,
            &controller.project_id,
            &scope.task_id,
            &scope.agent_id,
            &scope.session_id,
            &scope.run_id,
        )
        .with_turn(scope.turn_id.to_string())
        .with_item(evidence_id)
        .with_payload(agent_payload);
        // An agent actor, NOT the observed-runner actor.
        event.actor = "agent-sg7-claims".to_string();
        controller
            .state
            .append_event(event, &[])
            .expect("append agent claim");

        let score = controller
            .score_run(
                &score_scope(0, 100),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        assert!(
            !score.passed,
            "agent-reported claims must not raise the score"
        );
        assert_eq!(score.outcome, RunScoreOutcome::Failed);
        assert_eq!(
            score.observed_evidence_count, 0,
            "no OBSERVED evidence existed; the agent claim was filtered out"
        );
        assert_eq!(score.criteria_met, 0);

        // Now add a REAL observed pass; re-scoring DOES raise it (proving the
        // filter excludes only the agent claim, not the observed runner).
        let (controller2, workspace2, artifacts2) = controller_reusing(&controller);
        observe(
            &controller2,
            &workspace2,
            artifacts2,
            VerificationKind::Test,
            0,
        );
        let raised = controller2
            .score_run(
                &score_scope(0, 100),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("re-score");
        assert!(raised.passed, "an observed pass raises the score");
        assert_eq!(raised.observed_evidence_count, 1);
    }

    /// Open a second controller handle over the SAME state db as `base` (the
    /// store is a path handle), so a test can add observed evidence and re-score
    /// against the same log the agent claim lives in.
    fn controller_reusing(
        base: &FakeBoundaryController,
    ) -> (FakeBoundaryController, PathBuf, PathBuf) {
        let state_dir = base
            .state()
            .db_path()
            .parent()
            .expect("db dir")
            .to_path_buf();
        let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &state_dir)
            .expect("reopen");
        let workspace = temp_root("workspace2");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let artifacts = temp_root("artifacts2");
        (controller, workspace, artifacts)
    }

    #[test]
    fn score_records_real_wall_clock_timing_from_a_controlled_clock() {
        let (controller, workspace, artifacts) = controller();
        observe(
            &controller,
            &workspace,
            artifacts,
            VerificationKind::Test,
            0,
        );

        // A controlled clock window: started at 10_000, completed at 13_500 ms.
        let score = controller
            .score_run(
                &score_scope(10_000, 13_500),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        assert_eq!(score.started_at, 10_000);
        assert_eq!(score.completed_at, 13_500);
        assert_eq!(
            score.duration_millis, 3_500,
            "duration is real wall-clock millis, not an event-sequence delta"
        );
        assert_eq!(score.projection.started_at, 10_000);
        assert_eq!(score.projection.completed_at, 13_500);
        assert_eq!(score.projection.duration_millis, 3_500);
    }

    #[test]
    fn score_is_durable_queryable_and_reproducible_across_restart() {
        let (controller, workspace, artifacts) = controller();
        let state_db = controller.state().db_path().to_path_buf();
        observe(
            &controller,
            &workspace,
            artifacts,
            VerificationKind::Test,
            0,
        );

        let score = controller
            .score_run(
                &score_scope(1_000, 5_000),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("score run");

        // Queryable: the score is readable from the durable projection. (The
        // queried row carries the real `updated_sequence`; the in-memory return
        // value is constructed with sequence 0, so compare the content fields.)
        let stored = controller
            .state()
            .run_score_by_id(&score.projection.run_score_id)
            .expect("query score")
            .expect("score present");
        assert_eq!(stored.run_score_id, score.projection.run_score_id);
        assert_eq!(stored.outcome, "passed");
        assert!(stored.passed);
        assert_eq!(stored.duration_millis, 4_000);
        assert_eq!(stored.score_inputs_json, score.projection.score_inputs_json);

        // Survives restart + rebuilds identically from the event log.
        let reopened =
            SqliteStateStore::open(state_db.parent().expect("db dir")).expect("reopen state");
        reopened.rebuild_projections().expect("rebuild");
        let rebuilt = reopened
            .run_score_by_id(&score.projection.run_score_id)
            .expect("query rebuilt")
            .expect("rebuilt present");
        assert_eq!(
            rebuilt, stored,
            "the score rebuilds identically from the durable log"
        );

        // Reproducible: re-scoring the SAME observed evidence yields the SAME
        // score id (idempotent, no duplicate row).
        let again = controller
            .score_run(
                &score_scope(9_999, 99_999),
                &[AcceptanceCriterion::new(
                    "cargo test passes",
                    VerificationKind::Test,
                )],
            )
            .expect("re-score");
        assert_eq!(
            again.projection.run_score_id, score.projection.run_score_id,
            "the same observed evidence yields the same stable score id"
        );
        let all = controller
            .state()
            .run_scores_for_session(&SessionId::new("session-sg7"))
            .expect("scores");
        assert_eq!(
            all.len(),
            1,
            "re-scoring the same evidence does not duplicate"
        );
    }
}
