//! SG6: the controller-owned `VerificationRunner` -- the verification GATE.
//!
//! Where it lives (the SG6 open question, resolved): the runner lives in
//! `capo-controller`, the LOOP owner, beside the other safety gates SG1-SG5
//! (`permission_round_trip`, `grant_lifecycle`, `resource_ceiling`,
//! `workspace_lock`). The loop -- not `capo-eval` (a descriptive reporting stub)
//! and not `capo-server` (transport) -- is what decides whether a run passed and
//! must own the gate that derives that verdict, so it sits with the rest of the
//! loop's decide-style seams. `capo-eval`'s `score_run` (SG7) CONSUMES the
//! observed evidence this gate persists; it does not produce the verdict.
//!
//! What the gate does and, more importantly, what it REFUSES to trust:
//!
//! - It executes the project's configured check/lint/test command through the
//!   existing `capo-runtime` async local process runner (via the synchronous
//!   [`AsyncLocalProcessRunner::run_to_completion`] seam, so process execution,
//!   redaction, the output cap, and the tokio bridging all stay behind
//!   `capo-runtime`; the controller calls one sync method and never hand-rolls a
//!   runtime or touches a child process). `passed` is derived STRICTLY from the
//!   real process exit status (`exit_code == Some(0)`), never from an operator
//!   `--status passed` assertion or an agent-reported claim.
//! - It emits OBSERVED verification evidence (`evidence.recorded`, kind `test`
//!   for check/test commands and `smoke` for smoke commands) carrying the
//!   command, the real exit status, and a redacted output artifact ref. The
//!   event's actor is [`VERIFICATION_EVIDENCE_ACTOR`] -- a runner source distinct
//!   from any agent-reported channel -- and the payload records
//!   `source = "observed-runner"`, so SG7's `score_run` (which consumes observed
//!   evidence only) can tell this apart from agent claims.
//! - A successful run whose output exceeds the runtime cap is NOT failed: the
//!   runner truncates the artifact (recording `truncated` as metadata) while
//!   pass/fail stays keyed off exit status, so a long green test log is still
//!   `passed`.
//! - It can also CONSUME the typed `capo.test_run` / `capo.check` record that
//!   `tools-aci` emits ([`TestRunRecord`]) as an input -- the ACI tool owns
//!   evidence emission, the runner owns the GATE -- and it RE-DERIVES pass/fail
//!   from that record's `exit_status`, ignoring the record's own `passed` field
//!   if it disagrees. An agent that hand-writes `passed: true` with a non-zero
//!   exit status cannot make the gate pass.

use capo_runtime::{AsyncLocalProcessRunner, LocalProcessConfig, LocalProcessRequest};
use capo_state::EvidenceProjection;

use super::*;

/// The actor recorded on a verification `evidence.recorded` event.
///
/// Distinct from any agent or operator actor so SG7's observed-evidence-only
/// `score_run` can filter verification evidence to this runner source and never
/// confuse it with an agent-reported claim.
pub const VERIFICATION_EVIDENCE_ACTOR: &str = "capo-controller-verification";

/// The provenance marker stamped into the evidence payload's `source` field,
/// mirroring [`VERIFICATION_EVIDENCE_ACTOR`] inside the durable payload so a
/// rebuild from the event log can tell observed verification evidence from
/// agent-reported claims without consulting the actor column.
pub const VERIFICATION_EVIDENCE_SOURCE: &str = "observed-runner";

/// Which kind of verification command ran. Selects the evidence `kind` so SG7
/// can score test/check separately from smoke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationKind {
    /// A `cargo check`-style compile/type gate.
    Check,
    /// A linter (`clippy`, `eslint`, ...).
    Lint,
    /// A test run (`cargo test`, `pytest`, ...).
    Test,
    /// An end-to-end / smoke run.
    Smoke,
}

impl VerificationKind {
    /// The `evidence.recorded` kind for this command.
    ///
    /// Check/lint/test are recorded as `test` evidence (the compile-and-test
    /// gate); a smoke run is recorded as `smoke`. These are the two kinds the
    /// SG6 acceptance names (`evidence.recorded(kind=test/smoke)`).
    pub fn evidence_kind(self) -> &'static str {
        match self {
            Self::Check | Self::Lint | Self::Test => "test",
            Self::Smoke => "smoke",
        }
    }

    /// A stable label for the command kind, used in ids and the payload.
    pub fn label(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Smoke => "smoke",
        }
    }
}

/// A configured verification command the gate runs through the local runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationCommand {
    pub kind: VerificationKind,
    pub program: String,
    pub argv: Vec<String>,
    /// The working directory; must be inside the runner's workspace root.
    pub cwd: PathBuf,
}

impl VerificationCommand {
    pub fn new(
        kind: VerificationKind,
        program: impl Into<String>,
        argv: Vec<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            kind,
            program: program.into(),
            argv,
            cwd: cwd.into(),
        }
    }

    /// The human-readable command line, for the evidence payload.
    fn display(&self) -> String {
        if self.argv.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.argv.join(" "))
        }
    }
}

/// Where a verification run hangs on the loop's scope tree, so the persisted
/// `evidence.recorded` event carries the same task/agent/session/run/turn
/// provenance the rest of the loop uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
}

/// The typed `capo.test_run` / `capo.check` record `tools-aci` emits (ACI6).
///
/// This is the runner's INPUT when scoring a result the ACI tool already
/// executed: the ACI tool owns evidence emission and reports `exit_status`, and
/// the runner owns the GATE. The gate RE-DERIVES `passed` from `exit_status`
/// here and never trusts a supplied `passed` flag (see
/// [`TestRunRecord::observed_passed`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestRunRecord {
    pub kind: VerificationKind,
    pub command: String,
    /// The real process exit status the ACI tool observed (`None` = killed by a
    /// signal with no code).
    pub exit_status: Option<i32>,
    /// The ACI tool's own `passed` field. RECORDED for audit, but the gate does
    /// NOT trust it -- pass/fail is recomputed from `exit_status`.
    pub claimed_passed: bool,
    /// Whether the ACI tool truncated the output at the cap.
    pub truncated: bool,
    /// The redacted output artifact id the ACI tool persisted.
    pub output_artifact_id: Option<String>,
}

impl TestRunRecord {
    /// The TRUSTWORTHY pass/fail: derived strictly from the observed exit status,
    /// independent of [`Self::claimed_passed`]. Exit code 0 (and only 0) passes.
    pub fn observed_passed(&self) -> bool {
        self.exit_status == Some(0)
    }
}

/// The terminal verdict of one verification run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationOutcome {
    /// True iff the run's real exit status was 0.
    pub passed: bool,
    /// The process exit code, or `None` if killed by a signal.
    pub exit_code: Option<i32>,
    /// Whether the output was truncated at the runtime cap (metadata only --
    /// truncation never turns a successful run into a failed one).
    pub truncated: bool,
    /// The redacted output artifact ref the evidence points at.
    pub output_artifact_id: Option<String>,
    /// The persisted `evidence.recorded` evidence id.
    pub evidence_id: EvidenceId,
    /// The evidence kind (`test` / `smoke`).
    pub evidence_kind: String,
    /// The command line that was verified.
    pub command: String,
}

impl FakeBoundaryController {
    /// SG6: run a configured check/lint/test/smoke command through the local
    /// process runner and record observed verification evidence.
    ///
    /// Executes `command` via the `capo-runtime` async runner's synchronous seam
    /// ([`AsyncLocalProcessRunner::run_to_completion`]), derives `passed`
    /// STRICTLY from the real exit status, and persists an `evidence.recorded`
    /// event (kind `test`/`smoke`) carrying the command, exit status, truncation
    /// flag, and the redacted output artifact ref. The evidence is OBSERVED (actor
    /// [`VERIFICATION_EVIDENCE_ACTOR`], payload `source = observed-runner`), so
    /// SG7 can score against it.
    pub fn run_verification(
        &self,
        scope: &VerificationScope,
        config: LocalProcessConfig,
        command: &VerificationCommand,
    ) -> StateResult<VerificationOutcome> {
        let runner = AsyncLocalProcessRunner::new(config);
        let request = LocalProcessRequest::new(
            scope.run_id.clone(),
            command.program.clone(),
            command.argv.clone(),
            command.cwd.clone(),
            std::collections::HashMap::new(),
        )
        .with_turn_id(scope.turn_id.to_string());

        let outcome = runner.run_to_completion(request).map_err(|error| {
            // A runtime execution failure (spawn/IO/cwd-outside-workspace/env) is
            // surfaced as an IO StateError -- it is a failure to RUN the gate, not
            // a failed verdict. A run that executed but exited non-zero is a
            // legitimate `passed = false` outcome below, not an error.
            StateError::Io(std::io::Error::other(format!(
                "verification runner failed to execute `{}`: {error:?}",
                command.display()
            )))
        })?;

        // pass/fail is the EXIT STATUS, full stop: a clean exit passes even if
        // the artifact was truncated at the cap; any non-zero / signal fails.
        let passed = outcome.exit_code == Some(0);
        self.persist_verification_evidence(
            scope,
            command.kind,
            &command.display(),
            passed,
            outcome.exit_code,
            outcome.truncated,
            Some(outcome.stdout.artifact_id.clone()),
        )
    }

    /// SG6: consume the typed `capo.test_run` / `capo.check` record `tools-aci`
    /// emitted and record observed verification evidence from it.
    ///
    /// The ACI tool owns evidence emission and already ran the command; the GATE
    /// re-derives pass/fail from the record's observed `exit_status`
    /// ([`TestRunRecord::observed_passed`]) and IGNORES the record's own
    /// `claimed_passed` flag. An agent that reports `passed: true` over a
    /// non-zero exit status cannot make the gate pass.
    pub fn verify_from_test_run_record(
        &self,
        scope: &VerificationScope,
        record: &TestRunRecord,
    ) -> StateResult<VerificationOutcome> {
        let passed = record.observed_passed();
        self.persist_verification_evidence(
            scope,
            record.kind,
            &record.command,
            passed,
            record.exit_status,
            record.truncated,
            record.output_artifact_id.clone(),
        )
    }

    /// Persist one verification verdict as an observed `evidence.recorded` event
    /// + `EvidenceProjection`.
    #[allow(clippy::too_many_arguments)]
    fn persist_verification_evidence(
        &self,
        scope: &VerificationScope,
        kind: VerificationKind,
        command: &str,
        passed: bool,
        exit_code: Option<i32>,
        truncated: bool,
        output_artifact_id: Option<String>,
    ) -> StateResult<VerificationOutcome> {
        let evidence_kind = kind.evidence_kind();
        let exit_label = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        // The evidence id is keyed on (scope, kind, command, exit, artifact) so
        // re-recording the same observed verdict is idempotent and re-projects
        // identically on replay, while a different verdict gets a distinct id.
        let evidence_id = format!(
            "evidence-verification-{}-{}-{}",
            scope.run_id,
            kind.label(),
            stable_verification_hash(&format!(
                "{}:{}:{}:{}:{}:{}",
                scope.session_id,
                kind.label(),
                command,
                passed,
                exit_label,
                output_artifact_id.as_deref().unwrap_or("none")
            ))
        );

        let payload = serde_json::json!({
            "source": VERIFICATION_EVIDENCE_SOURCE,
            "verification_kind": kind.label(),
            "command": command,
            "passed": passed,
            "exit_status": exit_label,
            "truncated": truncated,
            "output_artifact_id": output_artifact_id.as_deref().unwrap_or("none"),
        })
        .to_string();

        // Higher confidence for an observed pass than an observed fail mirrors the
        // existing evidence-confidence convention; both are observed, not claimed.
        let confidence = if passed { 90 } else { 80 };

        let projection = EvidenceProjection {
            evidence_id: EvidenceId::new(evidence_id.clone()),
            project_id: self.project_id.clone(),
            task_id: Some(scope.task_id.clone()),
            session_id: Some(scope.session_id.clone()),
            run_id: Some(scope.run_id.clone()),
            kind: evidence_kind.to_string(),
            artifact_id: output_artifact_id.clone(),
            confidence,
            updated_sequence: 0,
        };

        let mut event = scoped_event(
            &format!("event-{evidence_id}"),
            EventKind::EvidenceRecorded,
            &self.project_id,
            &scope.task_id,
            &scope.agent_id,
            &scope.session_id,
            &scope.run_id,
        )
        .with_turn(scope.turn_id.to_string())
        .with_item(evidence_id.clone())
        .with_payload(payload);
        // OBSERVED, not agent-reported: stamp the runner-source actor so SG7's
        // observed-evidence-only score can distinguish this from agent claims.
        event.actor = VERIFICATION_EVIDENCE_ACTOR.to_string();

        self.state
            .append_event(event, &[ProjectionRecord::Evidence(projection)])?;

        Ok(VerificationOutcome {
            passed,
            exit_code,
            truncated,
            output_artifact_id,
            evidence_id: EvidenceId::new(evidence_id),
            evidence_kind: evidence_kind.to_string(),
            command: command.to_string(),
        })
    }
}

/// FNV-1a hash for stable verification evidence ids (no extra dependency; same
/// shape as `stable_eval_hash` in `capo-eval`).
fn stable_verification_hash(value: &str) -> String {
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

    use capo_state::SqliteStateStore;

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-sg6-{name}-{nanos}-{n}"))
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

    fn scope() -> VerificationScope {
        VerificationScope {
            task_id: TaskId::new("task-sg6"),
            agent_id: AgentId::new("agent-sg6"),
            session_id: SessionId::new("session-sg6"),
            run_id: RunId::new("run-sg6"),
            turn_id: TurnId::new("turn-sg6"),
        }
    }

    fn config(workspace: &Path, artifacts: PathBuf) -> LocalProcessConfig {
        LocalProcessConfig::for_test(workspace.to_path_buf(), artifacts)
    }

    fn evidence_payload(state: &SqliteStateStore, evidence_id: &EvidenceId) -> serde_json::Value {
        let event = state
            .recent_events_for_session(&SessionId::new("session-sg6"), 1000)
            .expect("events")
            .into_iter()
            .find(|event| event.item_id.as_deref() == Some(evidence_id.as_str()))
            .expect("verification evidence event");
        // OBSERVED, not agent-reported: the event actor is the runner source.
        assert_eq!(event.actor, VERIFICATION_EVIDENCE_ACTOR);
        assert_eq!(event.kind, "evidence.recorded");
        serde_json::from_str(&event.payload_json).expect("payload json")
    }

    #[test]
    fn scripted_command_pass_and_fail_classified_from_exit_status() {
        let (controller, workspace, artifacts) = controller();
        let scope = scope();

        // A command that exits 0 PASSES.
        let pass = controller
            .run_verification(
                &scope,
                config(&workspace, artifacts.clone()),
                &VerificationCommand::new(
                    VerificationKind::Test,
                    "/bin/sh",
                    vec!["-c".to_string(), "printf 'ok'; exit 0".to_string()],
                    workspace.clone(),
                ),
            )
            .expect("run pass");
        assert!(pass.passed, "exit 0 must classify passed");
        assert_eq!(pass.exit_code, Some(0));
        assert_eq!(pass.evidence_kind, "test");
        assert!(!pass.truncated);
        let pass_payload = evidence_payload(controller.state(), &pass.evidence_id);
        assert_eq!(pass_payload["passed"], serde_json::Value::Bool(true));
        assert_eq!(pass_payload["exit_status"], "0");
        assert_eq!(pass_payload["source"], VERIFICATION_EVIDENCE_SOURCE);

        // A command that exits non-zero FAILS.
        let fail = controller
            .run_verification(
                &scope,
                config(&workspace, artifacts.clone()),
                &VerificationCommand::new(
                    VerificationKind::Check,
                    "/bin/sh",
                    vec!["-c".to_string(), "printf 'boom' >&2; exit 5".to_string()],
                    workspace.clone(),
                ),
            )
            .expect("run fail");
        assert!(!fail.passed, "non-zero exit must classify failed");
        assert_eq!(fail.exit_code, Some(5));
        let fail_payload = evidence_payload(controller.state(), &fail.evidence_id);
        assert_eq!(fail_payload["passed"], serde_json::Value::Bool(false));
        assert_eq!(fail_payload["exit_status"], "5");

        // Pass and fail are distinct observed evidence rows.
        assert_ne!(pass.evidence_id, fail.evidence_id);
    }

    #[test]
    fn over_cap_successful_run_is_passed_and_truncated_not_failed() {
        let (controller, workspace, artifacts) = controller();
        let scope = scope();
        let mut config = config(&workspace, artifacts);
        config.output_limit_bytes = 16;

        let outcome = controller
            .run_verification(
                &scope,
                config,
                &VerificationCommand::new(
                    VerificationKind::Test,
                    "/bin/sh",
                    vec![
                        "-c".to_string(),
                        // Far more than the 16-byte cap, but exits 0.
                        "printf 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'; exit 0".to_string(),
                    ],
                    workspace.clone(),
                ),
            )
            .expect("run over-cap success");
        assert!(
            outcome.passed,
            "a successful over-cap run must be PASSED, not failed: truncation is metadata"
        );
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.truncated, "over-cap run must record truncation");
        let payload = evidence_payload(controller.state(), &outcome.evidence_id);
        assert_eq!(payload["passed"], serde_json::Value::Bool(true));
        assert_eq!(payload["truncated"], serde_json::Value::Bool(true));
    }

    #[test]
    fn typed_test_run_record_is_scored_from_exit_status_not_claimed_passed() {
        // The anti-spoofing control: a typed ACI record that CLAIMS passed=true
        // over a non-zero exit status is scored FAILED -- the gate re-derives
        // pass/fail from the observed exit status and ignores the claim.
        let (controller, _workspace, _artifacts) = controller();
        let scope = scope();

        let lying = TestRunRecord {
            kind: VerificationKind::Test,
            command: "cargo test --workspace".to_string(),
            exit_status: Some(101),
            claimed_passed: true, // an agent-reported lie
            truncated: false,
            output_artifact_id: Some("artifact-test-out".to_string()),
        };
        let outcome = controller
            .verify_from_test_run_record(&scope, &lying)
            .expect("verify lying record");
        assert!(
            !outcome.passed,
            "a claimed-passed record over a non-zero exit must score FAILED"
        );
        assert_eq!(outcome.exit_code, Some(101));
        let payload = evidence_payload(controller.state(), &outcome.evidence_id);
        assert_eq!(payload["passed"], serde_json::Value::Bool(false));
        assert_eq!(payload["exit_status"], "101");

        // And an honest exit-0 record is scored passed.
        let honest = TestRunRecord {
            kind: VerificationKind::Test,
            command: "cargo test --workspace".to_string(),
            exit_status: Some(0),
            claimed_passed: false,
            truncated: false,
            output_artifact_id: Some("artifact-test-out".to_string()),
        };
        let outcome = controller
            .verify_from_test_run_record(&scope, &honest)
            .expect("verify honest record");
        assert!(outcome.passed, "an exit-0 record scores passed");
    }

    #[test]
    fn observed_verification_evidence_survives_restart_and_reprojects_identically() {
        let (controller, workspace, artifacts) = controller();
        let scope = scope();
        let state_db = controller.state().db_path().to_path_buf();

        let outcome = controller
            .run_verification(
                &scope,
                config(&workspace, artifacts),
                &VerificationCommand::new(
                    VerificationKind::Smoke,
                    "/bin/sh",
                    vec!["-c".to_string(), "printf 'smoke-ok'; exit 0".to_string()],
                    workspace.clone(),
                ),
            )
            .expect("run smoke");
        assert_eq!(outcome.evidence_kind, "smoke");

        // Reopen the store from disk (a restart) and re-read the projection.
        let reopened =
            SqliteStateStore::open(state_db.parent().expect("db dir")).expect("reopen state");
        let evidence: Vec<_> = reopened
            .evidence_for_session(&scope.session_id)
            .expect("evidence")
            .into_iter()
            .filter(|item| item.evidence_id == outcome.evidence_id)
            .collect();
        assert_eq!(evidence.len(), 1, "exactly one observed verification row");
        assert_eq!(evidence[0].kind, "smoke");
        assert_eq!(evidence[0].run_id.as_ref(), Some(&scope.run_id));

        // Re-recording the SAME observed verdict is idempotent (same id, no dup).
        let again = controller
            .verify_from_test_run_record(
                &scope,
                &TestRunRecord {
                    kind: VerificationKind::Smoke,
                    command: "/bin/sh -c printf 'smoke-ok'; exit 0".to_string(),
                    exit_status: Some(0),
                    claimed_passed: true,
                    truncated: false,
                    output_artifact_id: outcome.output_artifact_id.clone(),
                },
            )
            .expect("re-record");
        // Same inputs (kind/command/exit/artifact) -> same evidence id, so the row
        // count is unchanged on the second record.
        let after: Vec<_> = controller
            .state()
            .evidence_for_session(&scope.session_id)
            .expect("evidence")
            .into_iter()
            .filter(|item| item.evidence_id == again.evidence_id)
            .collect();
        assert_eq!(after.len(), 1, "idempotent re-record must not duplicate");
    }
}
