//! SQLite-backed event store and projection scaffold.
//!
//! P2 keeps the store deliberately small but real: events are append-only,
//! projection updates are stored as replayable records, artifacts are explicit
//! rows, and read models can be rebuilt from the projection log.

use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{
    AgentId, BoundaryBinding, BoundaryKind, EvidenceId, MemoryPacketId, ProjectId, RunId,
    SessionId, TaskId, ToolCallId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

mod codec;
mod error;
mod event;
mod projections;
mod schema;

pub use error::{StateError, StateResult};
pub use event::{
    ArtifactRecord, EventKind, EventRecord, NewEvent, RecoveryAttempt, RedactionState,
};
pub use projections::*;

use codec::{projection_record_from_row, projection_record_to_row, validate_projection_json};
use schema::{clear_projection_tables, migrate};

/// Name of the first durable local state backend.
pub const PROTOTYPE_STATE_BACKEND: &str = "sqlite";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateStore {
    Fake(FakeStateStore),
    Sqlite(SqliteStateStore),
}

impl StateStore {
    pub fn fake() -> Self {
        Self::Fake(FakeStateStore)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(store) => store.binding(),
            Self::Sqlite(store) => store.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeStateStore;

impl FakeStateStore {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::StateStore, "fake-state")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteStateStore {
    root: PathBuf,
    db_path: PathBuf,
}

impl SqliteStateStore {
    pub fn open(root: impl AsRef<Path>) -> StateResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("artifacts"))?;
        let db_path = root.join("capo.sqlite");
        let mut connection = Connection::open(&db_path)?;
        migrate(&mut connection)?;
        Ok(Self { root, db_path })
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::StateStore,
            variant: "sqlite",
            fake: false,
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn artifact_root(&self) -> PathBuf {
        self.root.join("artifacts")
    }

    pub fn append_event(
        &self,
        event: NewEvent,
        projection_records: &[ProjectionRecord],
    ) -> StateResult<i64> {
        let mut connection = Connection::open(&self.db_path)?;
        let transaction = connection.transaction()?;
        if let (Some(project_id), Some(idempotency_key)) =
            (&event.project_id, &event.idempotency_key)
            && let Some(existing) = transaction
                .query_row(
                    "SELECT sequence
                     FROM events
                     WHERE project_id = ?1 AND idempotency_key = ?2
                     LIMIT 1",
                    params![project_id.as_str(), idempotency_key],
                    |row| row.get(0),
                )
                .optional()?
        {
            transaction.commit()?;
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO events (
                event_id, kind, actor, project_id, task_id, agent_id, session_id,
                run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                event.event_id,
                event.kind.as_str(),
                event.actor,
                event.project_id.as_ref().map(ProjectId::as_str),
                event.task_id.as_ref().map(TaskId::as_str),
                event.agent_id.as_ref().map(AgentId::as_str),
                event.session_id.as_ref().map(SessionId::as_str),
                event.run_id.as_ref().map(RunId::as_str),
                event.turn_id.as_deref(),
                event.item_id.as_deref(),
                event.payload_json,
                event.idempotency_key,
                event.redaction_state.as_str(),
            ],
        )?;
        let sequence = transaction.last_insert_rowid();

        for record in projection_records {
            insert_projection_record(&transaction, sequence, record)?;
            apply_projection_record(&transaction, sequence, record)?;
        }
        update_watermark(&transaction, "default", sequence)?;
        transaction.commit()?;
        Ok(sequence)
    }

    pub fn decide_permission_approval(
        &self,
        approval_id: &str,
        decided_event: NewEvent,
        grant_event: Option<NewEvent>,
        decided_approval: PermissionApprovalProjection,
        grant: Option<CapabilityGrantProjection>,
    ) -> StateResult<i64> {
        let mut connection = Connection::open(&self.db_path)?;
        let transaction = connection.transaction()?;
        let guarded = transaction.execute(
            "UPDATE permission_approvals
             SET status = status
             WHERE approval_id = ?1 AND project_id = ?2 AND status = 'pending'",
            params![approval_id, decided_approval.project_id.as_str()],
        )?;
        if guarded == 0 {
            let status = transaction
                .query_row(
                    "SELECT status FROM permission_approvals
                     WHERE approval_id = ?1 AND project_id = ?2",
                    params![approval_id, decided_approval.project_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .ok_or_else(|| StateError::MissingReadModel {
                    kind: "permission_approval",
                    id: approval_id.to_string(),
                })?;
            return Err(StateError::PermissionApprovalNotPending {
                approval_id: approval_id.to_string(),
                status,
            });
        }

        transaction.execute(
            "INSERT INTO events (
                event_id, kind, actor, project_id, task_id, agent_id, session_id,
                run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                decided_event.event_id,
                decided_event.kind.as_str(),
                decided_event.actor,
                decided_event.project_id.as_ref().map(ProjectId::as_str),
                decided_event.task_id.as_ref().map(TaskId::as_str),
                decided_event.agent_id.as_ref().map(AgentId::as_str),
                decided_event.session_id.as_ref().map(SessionId::as_str),
                decided_event.run_id.as_ref().map(RunId::as_str),
                decided_event.turn_id.as_deref(),
                decided_event.item_id.as_deref(),
                decided_event.payload_json,
                decided_event.idempotency_key,
                decided_event.redaction_state.as_str(),
            ],
        )?;
        let sequence = transaction.last_insert_rowid();
        let approval_record = ProjectionRecord::PermissionApproval(decided_approval);
        insert_projection_record(&transaction, sequence, &approval_record)?;
        apply_projection_record(&transaction, sequence, &approval_record)?;

        let final_sequence = if let (Some(grant_event), Some(grant)) = (grant_event, grant) {
            transaction.execute(
                "INSERT INTO events (
                    event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    grant_event.event_id,
                    grant_event.kind.as_str(),
                    grant_event.actor,
                    grant_event.project_id.as_ref().map(ProjectId::as_str),
                    grant_event.task_id.as_ref().map(TaskId::as_str),
                    grant_event.agent_id.as_ref().map(AgentId::as_str),
                    grant_event.session_id.as_ref().map(SessionId::as_str),
                    grant_event.run_id.as_ref().map(RunId::as_str),
                    grant_event.turn_id.as_deref(),
                    grant_event.item_id.as_deref(),
                    grant_event.payload_json,
                    grant_event.idempotency_key,
                    grant_event.redaction_state.as_str(),
                ],
            )?;
            let grant_sequence = transaction.last_insert_rowid();
            let grant_record = ProjectionRecord::CapabilityGrant(grant);
            insert_projection_record(&transaction, grant_sequence, &grant_record)?;
            apply_projection_record(&transaction, grant_sequence, &grant_record)?;
            grant_sequence
        } else {
            sequence
        };
        update_watermark(&transaction, "default", final_sequence)?;
        transaction.commit()?;
        Ok(final_sequence)
    }

    pub fn mark_active_runs_exited_unknown(
        &self,
        project_id: &ProjectId,
        recovery_attempt_id: &str,
    ) -> StateResult<Vec<RunProjection>> {
        let active_runs = self.active_looking_runs_for_project(project_id)?;
        let mut recovered = Vec::new();
        for run in active_runs {
            let recovered_run = RunProjection {
                status: "exited_unknown".to_string(),
                ..run.clone()
            };
            let event_id = format!("event-{recovery_attempt_id}-exited-{}", run.run_id);
            self.append_event(
                NewEvent {
                    event_id: event_id.clone(),
                    kind: EventKind::RunExited,
                    actor: "capo-recovery".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(run.session_id.clone()),
                    run_id: Some(run.run_id.clone()),
                    turn_id: None,
                    item_id: None,
                    payload_json: format!(
                        "{{\"recovery_attempt_id\":\"{}\",\"previous_status\":\"{}\",\"status\":\"exited_unknown\"}}",
                        escape_json(recovery_attempt_id),
                        escape_json(&run.status)
                    ),
                    idempotency_key: Some(format!(
                        "recovery:{recovery_attempt_id}:run:{}:exited_unknown",
                        run.run_id
                    )),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Run(recovered_run.clone())],
            )?;
            recovered.push(recovered_run);
        }
        Ok(recovered)
    }

    pub fn record_artifact(&self, artifact: ArtifactRecord) -> StateResult<()> {
        if !artifact.redaction_state.is_persistable_artifact() {
            return Err(StateError::UnsafeArtifactRedactionState(
                artifact.redaction_state,
            ));
        }

        let connection = Connection::open(&self.db_path)?;
        connection.execute(
            "INSERT OR REPLACE INTO artifacts (
                artifact_id, project_id, session_id, run_id, kind, uri, content_hash,
                size_bytes, redaction_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                artifact.artifact_id,
                artifact.project_id.as_ref().map(ProjectId::as_str),
                artifact.session_id.as_ref().map(SessionId::as_str),
                artifact.run_id.as_ref().map(RunId::as_str),
                artifact.kind,
                artifact.uri,
                artifact.content_hash,
                artifact.size_bytes,
                artifact.redaction_state.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn rebuild_projections(&self) -> StateResult<()> {
        let mut connection = Connection::open(&self.db_path)?;
        let transaction = connection.transaction()?;
        clear_projection_tables(&transaction)?;

        {
            let mut statement = transaction.prepare(
                "SELECT sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
                 FROM projection_records
                 ORDER BY sequence ASC, rowid ASC",
            )?;
            let rows = statement.query_map([], |row| {
                let record = projection_record_from_row(
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                )
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
                let sequence = row.get::<_, i64>(0)?;
                Ok((sequence, record))
            })?;

            for row in rows {
                let (sequence, record) = row?;
                apply_projection_record(&transaction, sequence, &record)?;
                update_watermark(&transaction, "default", sequence)?;
            }
        }

        let last_sequence = self.last_sequence_with_transaction(&transaction)?;
        update_watermark(&transaction, "default", last_sequence)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn begin_recovery(&self, recovery_attempt_id: &str) -> StateResult<RecoveryAttempt> {
        let connection = Connection::open(&self.db_path)?;
        let last_sequence = self.last_sequence()?;
        connection.execute(
            "INSERT INTO recovery_attempts (
                recovery_attempt_id, status, started_sequence, completed_sequence, notes
             ) VALUES (?1, 'started', ?2, NULL, '')",
            params![recovery_attempt_id, last_sequence],
        )?;
        Ok(RecoveryAttempt {
            recovery_attempt_id: recovery_attempt_id.to_string(),
            status: "started".to_string(),
            started_sequence: last_sequence,
            completed_sequence: None,
        })
    }

    pub fn complete_recovery(&self, recovery_attempt_id: &str) -> StateResult<RecoveryAttempt> {
        let mut connection = Connection::open(&self.db_path)?;
        let transaction = connection.transaction()?;
        let started_sequence = transaction
            .query_row(
                "SELECT started_sequence
                 FROM recovery_attempts
                 WHERE recovery_attempt_id = ?1",
                params![recovery_attempt_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StateError::MissingRecoveryAttempt(recovery_attempt_id.to_string()))?;
        let last_sequence = self.last_sequence_with_transaction(&transaction)?;
        transaction.execute(
            "UPDATE recovery_attempts
             SET status = 'completed', completed_sequence = ?2
             WHERE recovery_attempt_id = ?1",
            params![recovery_attempt_id, last_sequence],
        )?;
        transaction.commit()?;
        Ok(RecoveryAttempt {
            recovery_attempt_id: recovery_attempt_id.to_string(),
            status: "completed".to_string(),
            started_sequence,
            completed_sequence: Some(last_sequence),
        })
    }

    pub fn event_count(&self) -> StateResult<i64> {
        let connection = Connection::open(&self.db_path)?;
        let count = connection.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn last_sequence(&self) -> StateResult<i64> {
        let connection = Connection::open(&self.db_path)?;
        self.last_sequence_with_connection(&connection)
    }

    fn last_sequence_with_transaction(&self, transaction: &Transaction<'_>) -> StateResult<i64> {
        transaction
            .query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |row| {
                row.get(0)
            })
            .map_err(StateError::from)
    }

    fn last_sequence_with_connection(&self, connection: &Connection) -> StateResult<i64> {
        connection
            .query_row("SELECT COALESCE(MAX(sequence), 0) FROM events", [], |row| {
                row.get(0)
            })
            .map_err(StateError::from)
    }

    pub fn watermark(&self, name: &str) -> StateResult<Option<i64>> {
        let connection = Connection::open(&self.db_path)?;
        let watermark = connection
            .query_row(
                "SELECT last_sequence FROM projection_watermarks WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(watermark)
    }

    pub fn session(&self, session_id: &SessionId) -> StateResult<Option<SessionProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let session = connection
            .query_row(
                "SELECT session_id, project_id, task_id, agent_id, title, status, current_goal,
                        latest_summary, latest_confidence, latest_blocker, updated_sequence
                 FROM sessions
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(SessionProjection {
                        session_id: SessionId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        task_id: optional_id(row.get::<_, Option<String>>(2)?),
                        agent_id: AgentId::new(row.get::<_, String>(3)?),
                        title: row.get(4)?,
                        status: row.get(5)?,
                        current_goal: row.get(6)?,
                        latest_summary: row.get(7)?,
                        latest_confidence: row.get(8)?,
                        latest_blocker: row.get(9)?,
                        updated_sequence: row.get(10)?,
                    })
                },
            )
            .optional()?;
        Ok(session)
    }

    pub fn task(&self, task_id: &TaskId) -> StateResult<Option<TaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let task = connection
            .query_row(
                "SELECT task_id, project_id, title, capo_execution_status, active_session_id,
                        latest_summary, evidence_id, updated_sequence
                 FROM tasks
                 WHERE task_id = ?1",
                params![task_id.as_str()],
                |row| {
                    Ok(TaskProjection {
                        task_id: TaskId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        title: row.get(2)?,
                        capo_execution_status: row.get(3)?,
                        active_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        latest_summary: row.get(5)?,
                        evidence_id: optional_id(row.get::<_, Option<String>>(6)?),
                        updated_sequence: row.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(task)
    }

    pub fn agent(&self, agent_id: &AgentId) -> StateResult<Option<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let agent = connection
            .query_row(
                "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
                 FROM agents
                 WHERE agent_id = ?1",
                params![agent_id.as_str()],
                |row| {
                    Ok(AgentProjection {
                        agent_id: AgentId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        name: row.get(2)?,
                        status: row.get(3)?,
                        current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        updated_sequence: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(agent)
    }

    pub fn agent_by_name(&self, name: &str) -> StateResult<Option<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let agent = connection
            .query_row(
                "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
                 FROM agents
                 WHERE name = ?1
                 ORDER BY updated_sequence DESC
                 LIMIT 1",
                params![name],
                |row| {
                    Ok(AgentProjection {
                        agent_id: AgentId::new(row.get::<_, String>(0)?),
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        name: row.get(2)?,
                        status: row.get(3)?,
                        current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                        updated_sequence: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(agent)
    }

    pub fn agents(&self) -> StateResult<Vec<AgentProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT agent_id, project_id, name, status, current_session_id, updated_sequence
             FROM agents
             ORDER BY name ASC, agent_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AgentProjection {
                agent_id: AgentId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                name: row.get(2)?,
                status: row.get(3)?,
                current_session_id: optional_id(row.get::<_, Option<String>>(4)?),
                updated_sequence: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn run(&self, run_id: &RunId) -> StateResult<Option<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let run = connection
            .query_row(
                "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
                 FROM runs
                 WHERE run_id = ?1",
                params![run_id.as_str()],
                |row| {
                    Ok(RunProjection {
                        run_id: RunId::new(row.get::<_, String>(0)?),
                        session_id: SessionId::new(row.get::<_, String>(1)?),
                        status: row.get(2)?,
                        recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                        updated_sequence: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(run)
    }

    pub fn run_for_session(&self, session_id: &SessionId) -> StateResult<Option<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let run = connection
            .query_row(
                "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
                 FROM runs
                 WHERE session_id = ?1
                 ORDER BY updated_sequence DESC
                 LIMIT 1",
                params![session_id.as_str()],
                |row| {
                    Ok(RunProjection {
                        run_id: RunId::new(row.get::<_, String>(0)?),
                        session_id: SessionId::new(row.get::<_, String>(1)?),
                        status: row.get(2)?,
                        recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                        updated_sequence: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(run)
    }

    pub fn active_looking_runs(&self) -> StateResult<Vec<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT run_id, session_id, status, recovery_of_run_id, updated_sequence
             FROM runs
             WHERE status IN ('starting', 'running', 'stopping', 'active')
             ORDER BY updated_sequence ASC, run_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RunProjection {
                run_id: RunId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                status: row.get(2)?,
                recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                updated_sequence: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn active_looking_runs_for_project(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<RunProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT runs.run_id, runs.session_id, runs.status, runs.recovery_of_run_id,
                    runs.updated_sequence
             FROM runs
             JOIN sessions ON sessions.session_id = runs.session_id
             WHERE sessions.project_id = ?1
                AND runs.status IN ('starting', 'running', 'stopping', 'active')
             ORDER BY runs.updated_sequence ASC, runs.run_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(RunProjection {
                run_id: RunId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                status: row.get(2)?,
                recovery_of_run_id: optional_id(row.get::<_, Option<String>>(3)?),
                updated_sequence: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn capability_grants(&self) -> StateResult<Vec<CapabilityGrantProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT capability_grant_id, capability_profile_id, scope_json, effect,
                    subject_json, decision_source, persistence, explanation, updated_sequence
             FROM capability_grants
             ORDER BY updated_sequence ASC, capability_grant_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CapabilityGrantProjection {
                capability_grant_id: row.get(0)?,
                capability_profile_id: row.get(1)?,
                scope_json: row.get(2)?,
                effect: row.get(3)?,
                subject_json: row.get(4)?,
                decision_source: row.get(5)?,
                persistence: row.get(6)?,
                explanation: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn permission_approvals(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<PermissionApprovalProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT approval_id, project_id, session_id, tool_call_id, capability_profile_id,
                    scope_json, subject_json, status, requested_by, reason, decision,
                    capability_grant_id, updated_sequence
             FROM permission_approvals
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, approval_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(PermissionApprovalProjection {
                approval_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                session_id: optional_id(row.get::<_, Option<String>>(2)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(3)?),
                capability_profile_id: row.get(4)?,
                scope_json: row.get(5)?,
                subject_json: row.get(6)?,
                status: row.get(7)?,
                requested_by: row.get(8)?,
                reason: row.get(9)?,
                decision: row.get(10)?,
                capability_grant_id: row.get(11)?,
                updated_sequence: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn permission_approval(
        &self,
        project_id: &ProjectId,
        approval_id: &str,
    ) -> StateResult<Option<PermissionApprovalProjection>> {
        Ok(self
            .permission_approvals(project_id)?
            .into_iter()
            .find(|approval| approval.approval_id == approval_id))
    }

    pub fn connectivity_exposures(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<ConnectivityExposureProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT exposure_id, project_id, connectivity_endpoint_id, owner_kind, owner_id,
                    channel_kind, exposure, permission_scope, status, capability_grant_id,
                    health_status, reachable, revoked_at, updated_sequence
             FROM connectivity_exposures
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, exposure_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(ConnectivityExposureProjection {
                exposure_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                connectivity_endpoint_id: row.get(2)?,
                owner_kind: row.get(3)?,
                owner_id: row.get(4)?,
                channel_kind: row.get(5)?,
                exposure: row.get(6)?,
                permission_scope: row.get(7)?,
                status: row.get(8)?,
                capability_grant_id: row.get(9)?,
                health_status: row.get(10)?,
                reachable: row.get::<_, i64>(11)? != 0,
                revoked_at: row.get(12)?,
                updated_sequence: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn runtime_targets(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<RuntimeTargetProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT runtime_target_id, project_id, name, runner_kind, workspace_root,
                    artifact_root, default_cwd, capability_profile_id, connectivity_endpoint_id,
                    status, updated_sequence
             FROM runtime_targets
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, runtime_target_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(RuntimeTargetProjection {
                runtime_target_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                name: row.get(2)?,
                runner_kind: row.get(3)?,
                workspace_root: row.get(4)?,
                artifact_root: row.get(5)?,
                default_cwd: row.get(6)?,
                capability_profile_id: row.get(7)?,
                connectivity_endpoint_id: row.get(8)?,
                status: row.get(9)?,
                updated_sequence: row.get(10)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_readiness(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterReadinessProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT adapter_kind, project_id, program, opt_in_env, opted_in, smoke_status,
                    credential_policy, expected_marker, env_allowlist_count,
                    redaction_rule_count, output_limit_bytes, dogfood_blocker, updated_sequence
             FROM adapter_readiness
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, adapter_kind ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterReadinessProjection {
                adapter_kind: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                program: row.get(2)?,
                opt_in_env: row.get(3)?,
                opted_in: row.get::<_, i64>(4)? != 0,
                smoke_status: row.get(5)?,
                credential_policy: row.get(6)?,
                expected_marker: row.get(7)?,
                env_allowlist_count: row.get(8)?,
                redaction_rule_count: row.get(9)?,
                output_limit_bytes: row.get(10)?,
                dogfood_blocker: row.get(11)?,
                updated_sequence: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_smoke_reports(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterSmokeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT smoke_report_id, project_id, adapter_kind, smoke_status,
                    credential_scan_status, marker_found, artifact_root, reason,
                    dogfood_readiness_effect, updated_sequence
             FROM adapter_smoke_reports
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, smoke_report_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterSmokeReportProjection {
                smoke_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                adapter_kind: row.get(2)?,
                smoke_status: row.get(3)?,
                credential_scan_status: row.get(4)?,
                marker_found: row.get::<_, i64>(5)? != 0,
                artifact_root: row.get(6)?,
                reason: row.get(7)?,
                dogfood_readiness_effect: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_plans(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPlanProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_plan_id, project_id, adapter_kind, provider_kind,
                    credential_scope, agent_id, agent_name, session_id, run_id,
                    runtime_program, runtime_arg_count, runtime_prompt_policy,
                    runtime_cwd, artifact_root, request_env_count, env_allowlist_count,
                    redaction_rule_count, stdout_format, stderr_policy,
                    provider_cli_executed, status, updated_sequence
             FROM adapter_dispatch_plans
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_plan_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPlanProjection {
                dispatch_plan_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                adapter_kind: row.get(2)?,
                provider_kind: row.get(3)?,
                credential_scope: row.get(4)?,
                agent_id: AgentId::new(row.get::<_, String>(5)?),
                agent_name: row.get(6)?,
                session_id: SessionId::new(row.get::<_, String>(7)?),
                run_id: RunId::new(row.get::<_, String>(8)?),
                runtime_program: row.get(9)?,
                runtime_arg_count: row.get(10)?,
                runtime_prompt_policy: row.get(11)?,
                runtime_cwd: row.get(12)?,
                artifact_root: row.get(13)?,
                request_env_count: row.get(14)?,
                env_allowlist_count: row.get(15)?,
                redaction_rule_count: row.get(16)?,
                stdout_format: row.get(17)?,
                stderr_policy: row.get(18)?,
                provider_cli_executed: row.get::<_, i64>(19)? != 0,
                status: row.get(20)?,
                updated_sequence: row.get(21)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_gates(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchGateProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_gate_id, project_id, dispatch_plan_id, adapter_kind,
                    provider_cli_execution_allowed, status, required_dogfood_gate,
                    reason_codes, provider_cli_executed, runtime_prompt_policy,
                    updated_sequence
             FROM adapter_dispatch_gates
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_gate_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchGateProjection {
                dispatch_gate_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                adapter_kind: row.get(3)?,
                provider_cli_execution_allowed: row.get::<_, i64>(4)? != 0,
                status: row.get(5)?,
                required_dogfood_gate: row.get(6)?,
                reason_codes: row.get(7)?,
                provider_cli_executed: row.get::<_, i64>(8)? != 0,
                runtime_prompt_policy: row.get(9)?,
                updated_sequence: row.get(10)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_replays(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchReplayProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_replay_id, project_id, dispatch_plan_id, dispatch_gate_id,
                    adapter_kind, session_id, run_id, fixture_path, fixture_hash,
                    input_event_count, appended_event_count, tool_event_count,
                    summary_event_count, completed_turn_count, provider_cli_executed,
                    raw_content_policy, updated_sequence
             FROM adapter_dispatch_replays
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_replay_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchReplayProjection {
                dispatch_replay_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                dispatch_gate_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                session_id: SessionId::new(row.get::<_, String>(5)?),
                run_id: RunId::new(row.get::<_, String>(6)?),
                fixture_path: row.get(7)?,
                fixture_hash: row.get(8)?,
                input_event_count: row.get(9)?,
                appended_event_count: row.get(10)?,
                tool_event_count: row.get(11)?,
                summary_event_count: row.get(12)?,
                completed_turn_count: row.get(13)?,
                provider_cli_executed: row.get::<_, i64>(14)? != 0,
                raw_content_policy: row.get(15)?,
                updated_sequence: row.get(16)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_execution_requests(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchExecutionRequestProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT execution_request_id, project_id, dispatch_plan_id, dispatch_gate_id,
                    adapter_kind, provider_cli_execution_allowed, provider_cli_executed,
                    status, opt_in_env, runtime_prompt_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_execution_requests
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, execution_request_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchExecutionRequestProjection {
                execution_request_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                dispatch_gate_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                provider_cli_execution_allowed: row.get::<_, i64>(5)? != 0,
                provider_cli_executed: row.get::<_, i64>(6)? != 0,
                status: row.get(7)?,
                opt_in_env: row.get(8)?,
                runtime_prompt_policy: row.get(9)?,
                reason_codes: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_executions(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchExecutionProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT dispatch_execution_id, project_id, dispatch_plan_id,
                    execution_request_id, adapter_kind, session_id, run_id,
                    provider_cli_execution_allowed, provider_cli_executed, status,
                    exit_code, runtime_process_ref, stdout_artifact_id,
                    stderr_artifact_id, artifact_root, credential_scan_status,
                    raw_prompt_policy, raw_output_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_executions
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, dispatch_execution_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchExecutionProjection {
                dispatch_execution_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                execution_request_id: row.get(3)?,
                adapter_kind: row.get(4)?,
                session_id: SessionId::new(row.get::<_, String>(5)?),
                run_id: RunId::new(row.get::<_, String>(6)?),
                provider_cli_execution_allowed: row.get::<_, i64>(7)? != 0,
                provider_cli_executed: row.get::<_, i64>(8)? != 0,
                status: row.get(9)?,
                exit_code: row.get(10)?,
                runtime_process_ref: row.get(11)?,
                stdout_artifact_id: row.get(12)?,
                stderr_artifact_id: row.get(13)?,
                artifact_root: row.get(14)?,
                credential_scan_status: row.get(15)?,
                raw_prompt_policy: row.get(16)?,
                raw_output_policy: row.get(17)?,
                reason_codes: row.get(18)?,
                updated_sequence: row.get(19)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_prompt_sources(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPromptSourceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT prompt_source_id, project_id, dispatch_plan_id, prompt_hash,
                    source_kind, source_ref, source_hash, materialization_status,
                    raw_prompt_policy, updated_sequence
             FROM adapter_dispatch_prompt_sources
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, prompt_source_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPromptSourceProjection {
                prompt_source_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                prompt_hash: row.get(3)?,
                source_kind: row.get(4)?,
                source_ref: row.get(5)?,
                source_hash: row.get(6)?,
                materialization_status: row.get(7)?,
                raw_prompt_policy: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn adapter_dispatch_prompt_materializations(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<AdapterDispatchPromptMaterializationProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT materialization_id, project_id, dispatch_plan_id, prompt_source_id,
                    source_kind, source_ref, expected_source_hash, observed_source_hash,
                    expected_prompt_hash, materialized_prompt_hash, status,
                    raw_prompt_policy, reason_codes, updated_sequence
             FROM adapter_dispatch_prompt_materializations
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, materialization_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(AdapterDispatchPromptMaterializationProjection {
                materialization_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                dispatch_plan_id: row.get(2)?,
                prompt_source_id: row.get(3)?,
                source_kind: row.get(4)?,
                source_ref: row.get(5)?,
                expected_source_hash: row.get(6)?,
                observed_source_hash: row.get(7)?,
                expected_prompt_hash: row.get(8)?,
                materialized_prompt_hash: row.get(9)?,
                status: row.get(10)?,
                raw_prompt_policy: row.get(11)?,
                reason_codes: row.get(12)?,
                updated_sequence: row.get(13)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn evidence_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<EvidenceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT evidence_id, project_id, task_id, session_id, run_id, kind, artifact_id,
                    confidence, updated_sequence
             FROM evidence
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, evidence_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(EvidenceProjection {
                evidence_id: EvidenceId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                session_id: optional_id(row.get::<_, Option<String>>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                kind: row.get(5)?,
                artifact_id: row.get(6)?,
                confidence: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn project_evidence(&self, project_id: &ProjectId) -> StateResult<Vec<EvidenceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT evidence_id, project_id, task_id, session_id, run_id, kind, artifact_id,
                    confidence, updated_sequence
             FROM evidence
             WHERE project_id = ?1 AND session_id IS NULL
             ORDER BY updated_sequence ASC, evidence_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(EvidenceProjection {
                evidence_id: EvidenceId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                session_id: optional_id(row.get::<_, Option<String>>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                kind: row.get(5)?,
                artifact_id: row.get(6)?,
                confidence: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_packets_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<MemoryPacketProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_packet_id, project_id, task_id, agent_id, session_id, run_id,
                    turn_id, packet_artifact_id, purpose, updated_sequence
             FROM memory_packet_refs
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, memory_packet_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new(row.get::<_, String>(0)?),
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: optional_id(row.get::<_, Option<String>>(2)?),
                agent_id: optional_id(row.get::<_, Option<String>>(3)?),
                session_id: optional_id(row.get::<_, Option<String>>(4)?),
                run_id: optional_id(row.get::<_, Option<String>>(5)?),
                turn_id: row.get(6)?,
                packet_artifact_id: row.get(7)?,
                purpose: row.get(8)?,
                updated_sequence: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_records_for_project(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<MemoryRecordProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                    sensitivity_classification, record_kind, subject, predicate, object,
                    body, confidence, review_state, source_count, valid_from, valid_until,
                    supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                    invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             FROM memory_records
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, memory_record_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(MemoryRecordProjection {
                memory_record_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                scope: row.get(2)?,
                scope_owner_ref: row.get(3)?,
                subject_ref: row.get(4)?,
                sensitivity_classification: row.get(5)?,
                record_kind: row.get(6)?,
                subject: row.get(7)?,
                predicate: row.get(8)?,
                object: row.get(9)?,
                body: row.get(10)?,
                confidence: row.get(11)?,
                review_state: row.get(12)?,
                source_count: row.get(13)?,
                valid_from: row.get(14)?,
                valid_until: row.get(15)?,
                supersedes_memory_record_id: row.get(16)?,
                revoked_by_memory_record_id: row.get(17)?,
                redaction_state: row.get(18)?,
                invalidated_at: row.get(19)?,
                invalidation_reason: row.get(20)?,
                packet_item_ref: row.get(21)?,
                updated_sequence: row.get(22)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn packet_eligible_memory_records(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<MemoryRecordProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                    sensitivity_classification, record_kind, subject, predicate, object,
                    body, confidence, review_state, source_count, valid_from, valid_until,
                    supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                    invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             FROM memory_records
             WHERE project_id = ?1
                AND review_state = 'reviewed'
                AND source_count > 0
                AND valid_until IS NULL
                AND revoked_by_memory_record_id IS NULL
                AND invalidated_at IS NULL
                AND packet_item_ref IS NOT NULL
                AND sensitivity_classification != 'secret_derived'
                AND redaction_state NOT IN ('unknown', 'contains_sensitive')
                AND EXISTS (
                    SELECT 1
                    FROM memory_sources
                    WHERE memory_sources.memory_record_id = memory_records.memory_record_id
                      AND memory_sources.source_content_hash IS NOT NULL
                      AND (
                        memory_sources.source_anchor IS NOT NULL
                        OR memory_sources.source_event_id IS NOT NULL
                        OR memory_sources.source_artifact_id IS NOT NULL
                      )
                )
             ORDER BY updated_sequence ASC, memory_record_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(MemoryRecordProjection {
                memory_record_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                scope: row.get(2)?,
                scope_owner_ref: row.get(3)?,
                subject_ref: row.get(4)?,
                sensitivity_classification: row.get(5)?,
                record_kind: row.get(6)?,
                subject: row.get(7)?,
                predicate: row.get(8)?,
                object: row.get(9)?,
                body: row.get(10)?,
                confidence: row.get(11)?,
                review_state: row.get(12)?,
                source_count: row.get(13)?,
                valid_from: row.get(14)?,
                valid_until: row.get(15)?,
                supersedes_memory_record_id: row.get(16)?,
                revoked_by_memory_record_id: row.get(17)?,
                redaction_state: row.get(18)?,
                invalidated_at: row.get(19)?,
                invalidation_reason: row.get(20)?,
                packet_item_ref: row.get(21)?,
                updated_sequence: row.get(22)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn memory_sources_for_record(
        &self,
        memory_record_id: &str,
    ) -> StateResult<Vec<MemorySourceProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT memory_source_id, memory_record_id, source_kind, source_event_id,
                    source_artifact_id, source_path, source_anchor, source_content_hash,
                    source_sequence, quote_artifact_id, observed_at, updated_sequence
             FROM memory_sources
             WHERE memory_record_id = ?1
             ORDER BY source_sequence ASC, memory_source_id ASC",
        )?;
        let rows = statement.query_map(params![memory_record_id], |row| {
            Ok(MemorySourceProjection {
                memory_source_id: row.get(0)?,
                memory_record_id: row.get(1)?,
                source_kind: row.get(2)?,
                source_event_id: row.get(3)?,
                source_artifact_id: row.get(4)?,
                source_path: row.get(5)?,
                source_anchor: row.get(6)?,
                source_content_hash: row.get(7)?,
                source_sequence: row.get(8)?,
                quote_artifact_id: row.get(9)?,
                observed_at: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports_for_task(
        &self,
        task_id: &TaskId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE task_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![task_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn task_outcome_reports_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<TaskOutcomeReportProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT task_outcome_report_id, project_id, task_id, session_id, run_id,
                    outcome_status, started_sequence, completed_sequence,
                    duration_sequence_span, action_count, tool_call_count, evidence_count,
                    memory_packet_count, confidence, blocker, review_outcome, report_artifact_id,
                    updated_sequence
             FROM task_outcome_reports
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, task_outcome_report_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(TaskOutcomeReportProjection {
                task_outcome_report_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: RunId::new(row.get::<_, String>(4)?),
                outcome_status: row.get(5)?,
                started_sequence: row.get(6)?,
                completed_sequence: row.get(7)?,
                duration_sequence_span: row.get(8)?,
                action_count: row.get(9)?,
                tool_call_count: row.get(10)?,
                evidence_count: row.get(11)?,
                memory_packet_count: row.get(12)?,
                confidence: row.get(13)?,
                blocker: row.get(14)?,
                review_outcome: row.get(15)?,
                report_artifact_id: row.get(16)?,
                updated_sequence: row.get(17)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn review_findings_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ReviewFindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                    workpad_task_id, reviewer, finding_kind, severity, summary, status,
                    evidence_artifact_id, follow_up, updated_sequence
             FROM review_findings
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, review_finding_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ReviewFindingProjection {
                review_finding_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(5)?),
                workpad_task_id: row.get(6)?,
                reviewer: row.get(7)?,
                finding_kind: row.get(8)?,
                severity: row.get(9)?,
                summary: row.get(10)?,
                status: row.get(11)?,
                evidence_artifact_id: row.get(12)?,
                follow_up: row.get(13)?,
                updated_sequence: row.get(14)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn review_findings(
        &self,
        project_id: &ProjectId,
    ) -> StateResult<Vec<ReviewFindingProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                    workpad_task_id, reviewer, finding_kind, severity, summary, status,
                    evidence_artifact_id, follow_up, updated_sequence
             FROM review_findings
             WHERE project_id = ?1
             ORDER BY updated_sequence ASC, review_finding_id ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(ReviewFindingProjection {
                review_finding_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                task_id: TaskId::new(row.get::<_, String>(2)?),
                session_id: SessionId::new(row.get::<_, String>(3)?),
                run_id: optional_id(row.get::<_, Option<String>>(4)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(5)?),
                workpad_task_id: row.get(6)?,
                reviewer: row.get(7)?,
                finding_kind: row.get(8)?,
                severity: row.get(9)?,
                summary: row.get(10)?,
                status: row.get(11)?,
                evidence_artifact_id: row.get(12)?,
                follow_up: row.get(13)?,
                updated_sequence: row.get(14)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn tool_calls_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ToolCallProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT tool_call_id, session_id, turn_id, tool_name, tool_origin, status,
                    input_artifact_id, output_artifact_id, updated_sequence
             FROM tool_calls
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, tool_call_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ToolCallProjection {
                tool_call_id: ToolCallId::new(row.get::<_, String>(0)?),
                session_id: SessionId::new(row.get::<_, String>(1)?),
                turn_id: row.get(2)?,
                tool_name: row.get(3)?,
                tool_origin: row.get(4)?,
                status: row.get(5)?,
                input_artifact_id: row.get(6)?,
                output_artifact_id: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn tool_observations_for_session(
        &self,
        session_id: &SessionId,
    ) -> StateResult<Vec<ToolObservationProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT tool_observation_id, session_id, tool_call_id, source, external_tool_ref,
                    tool_name, observed_status, instrumentation_level, confidence,
                    raw_event_hash, artifact_id, updated_sequence
             FROM tool_observations
             WHERE session_id = ?1
             ORDER BY updated_sequence ASC, tool_observation_id ASC",
        )?;
        let rows = statement.query_map(params![session_id.as_str()], |row| {
            Ok(ToolObservationProjection {
                tool_observation_id: row.get(0)?,
                session_id: SessionId::new(row.get::<_, String>(1)?),
                tool_call_id: optional_id(row.get::<_, Option<String>>(2)?),
                source: row.get(3)?,
                external_tool_ref: row.get(4)?,
                tool_name: row.get(5)?,
                observed_status: row.get(6)?,
                instrumentation_level: row.get(7)?,
                confidence: row.get(8)?,
                raw_event_hash: row.get(9)?,
                artifact_id: row.get(10)?,
                updated_sequence: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_files(&self, project_id: &ProjectId) -> StateResult<Vec<WorkpadFileProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT path, project_id, content_hash, headings, objective, observed_unix, updated_sequence
             FROM workpad_files
             WHERE project_id = ?1
             ORDER BY path ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(WorkpadFileProjection {
                path: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                content_hash: row.get(2)?,
                headings: row.get(3)?,
                objective: row.get(4)?,
                observed_unix: row.get(5)?,
                updated_sequence: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_file(
        &self,
        project_id: &ProjectId,
        path: &str,
    ) -> StateResult<Option<WorkpadFileProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let file = connection
            .query_row(
                "SELECT path, project_id, content_hash, headings, objective, observed_unix, updated_sequence
                 FROM workpad_files
                 WHERE project_id = ?1 AND path = ?2",
                params![project_id.as_str(), path],
                |row| {
                    Ok(WorkpadFileProjection {
                        path: row.get(0)?,
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        content_hash: row.get(2)?,
                        headings: row.get(3)?,
                        objective: row.get(4)?,
                        observed_unix: row.get(5)?,
                        updated_sequence: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(file)
    }

    pub fn workpad_tasks(&self, project_id: &ProjectId) -> StateResult<Vec<WorkpadTaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT workpad_task_id, project_id, path, source_anchor, title, observed_status,
                    capo_execution_status, observed_unix, updated_sequence
             FROM workpad_tasks
             WHERE project_id = ?1
             ORDER BY path ASC, source_anchor ASC",
        )?;
        let rows = statement.query_map(params![project_id.as_str()], |row| {
            Ok(WorkpadTaskProjection {
                workpad_task_id: row.get(0)?,
                project_id: ProjectId::new(row.get::<_, String>(1)?),
                path: row.get(2)?,
                source_anchor: row.get(3)?,
                title: row.get(4)?,
                observed_status: row.get(5)?,
                capo_execution_status: row.get(6)?,
                observed_unix: row.get(7)?,
                updated_sequence: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn workpad_task(
        &self,
        project_id: &ProjectId,
        workpad_task_id: &str,
    ) -> StateResult<Option<WorkpadTaskProjection>> {
        let connection = Connection::open(&self.db_path)?;
        let task = connection
            .query_row(
                "SELECT workpad_task_id, project_id, path, source_anchor, title, observed_status,
                        capo_execution_status, observed_unix, updated_sequence
                 FROM workpad_tasks
                 WHERE project_id = ?1 AND workpad_task_id = ?2",
                params![project_id.as_str(), workpad_task_id],
                |row| {
                    Ok(WorkpadTaskProjection {
                        workpad_task_id: row.get(0)?,
                        project_id: ProjectId::new(row.get::<_, String>(1)?),
                        path: row.get(2)?,
                        source_anchor: row.get(3)?,
                        title: row.get(4)?,
                        observed_status: row.get(5)?,
                        capo_execution_status: row.get(6)?,
                        observed_unix: row.get(7)?,
                        updated_sequence: row.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(task)
    }

    pub fn recent_events_for_session(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> StateResult<Vec<EventRecord>> {
        let connection = Connection::open(&self.db_path)?;
        let mut statement = connection.prepare(
            "SELECT sequence, event_id, kind, actor, project_id, task_id, agent_id, session_id,
                    run_id, turn_id, item_id, payload_json, idempotency_key, redaction_state
             FROM events
             WHERE session_id = ?1
             ORDER BY sequence DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![session_id.as_str(), limit as i64], |row| {
            Ok(EventRecord {
                sequence: row.get(0)?,
                event_id: row.get(1)?,
                kind: row.get(2)?,
                actor: row.get(3)?,
                project_id: optional_id(row.get::<_, Option<String>>(4)?),
                task_id: optional_id(row.get::<_, Option<String>>(5)?),
                agent_id: optional_id(row.get::<_, Option<String>>(6)?),
                session_id: optional_id(row.get::<_, Option<String>>(7)?),
                run_id: optional_id(row.get::<_, Option<String>>(8)?),
                turn_id: row.get(9)?,
                item_id: row.get(10)?,
                payload_json: row.get(11)?,
                idempotency_key: row.get(12)?,
                redaction_state: row.get(13)?,
            })
        })?;
        let mut events = rows.collect::<Result<Vec<_>, _>>()?;
        events.reverse();
        Ok(events)
    }
}

fn update_watermark(transaction: &Transaction<'_>, name: &str, sequence: i64) -> StateResult<()> {
    transaction.execute(
        "INSERT INTO projection_watermarks(name, last_sequence)
         VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET last_sequence = excluded.last_sequence",
        params![name, sequence],
    )?;
    Ok(())
}

fn insert_projection_record(
    transaction: &Transaction<'_>,
    sequence: i64,
    record: &ProjectionRecord,
) -> StateResult<()> {
    let row = projection_record_to_row(record);
    transaction.execute(
        "INSERT INTO projection_records (
            sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            sequence,
            row.kind,
            row.record_id,
            row.a,
            row.b,
            row.c,
            row.d,
            row.e,
            row.f,
            row.g,
            row.h,
            row.payload_json,
        ],
    )?;
    Ok(())
}

fn apply_projection_record(
    transaction: &Transaction<'_>,
    sequence: i64,
    record: &ProjectionRecord,
) -> StateResult<()> {
    match record {
        ProjectionRecord::Project(project) => transaction.execute(
            "INSERT INTO projects(project_id, name, status, updated_sequence)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET
                name = excluded.name,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                project.project_id.as_str(),
                project.name,
                project.status,
                sequence
            ],
        )?,
        ProjectionRecord::Task(task) => transaction.execute(
            "INSERT INTO tasks(
                task_id, project_id, title, capo_execution_status, active_session_id,
                latest_summary, evidence_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(task_id) DO UPDATE SET
                project_id = excluded.project_id,
                title = excluded.title,
                capo_execution_status = excluded.capo_execution_status,
                active_session_id = excluded.active_session_id,
                latest_summary = excluded.latest_summary,
                evidence_id = excluded.evidence_id,
                updated_sequence = excluded.updated_sequence",
            params![
                task.task_id.as_str(),
                task.project_id.as_str(),
                task.title,
                task.capo_execution_status,
                task.active_session_id.as_ref().map(SessionId::as_str),
                task.latest_summary,
                task.evidence_id.as_ref().map(EvidenceId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::Agent(agent) => transaction.execute(
            "INSERT INTO agents(agent_id, project_id, name, status, current_session_id, updated_sequence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(agent_id) DO UPDATE SET
                project_id = excluded.project_id,
                name = excluded.name,
                status = excluded.status,
                current_session_id = excluded.current_session_id,
                updated_sequence = excluded.updated_sequence",
            params![
                agent.agent_id.as_str(),
                agent.project_id.as_str(),
                agent.name,
                agent.status,
                agent.current_session_id.as_ref().map(SessionId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::Session(session) => transaction.execute(
            "INSERT INTO sessions(
                session_id, project_id, task_id, agent_id, title, status, current_goal,
                latest_summary, latest_confidence, latest_blocker, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(session_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                agent_id = excluded.agent_id,
                title = excluded.title,
                status = excluded.status,
                current_goal = excluded.current_goal,
                latest_summary = excluded.latest_summary,
                latest_confidence = excluded.latest_confidence,
                latest_blocker = excluded.latest_blocker,
                updated_sequence = excluded.updated_sequence",
            params![
                session.session_id.as_str(),
                session.project_id.as_str(),
                session.task_id.as_ref().map(TaskId::as_str),
                session.agent_id.as_str(),
                session.title,
                session.status,
                session.current_goal,
                session.latest_summary,
                session.latest_confidence,
                session.latest_blocker,
                sequence,
            ],
        )?,
        ProjectionRecord::Run(run) => transaction.execute(
            "INSERT INTO runs(run_id, session_id, status, recovery_of_run_id, updated_sequence)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id) DO UPDATE SET
                session_id = excluded.session_id,
                status = excluded.status,
                recovery_of_run_id = excluded.recovery_of_run_id,
                updated_sequence = excluded.updated_sequence",
            params![
                run.run_id.as_str(),
                run.session_id.as_str(),
                run.status,
                run.recovery_of_run_id.as_ref().map(RunId::as_str),
                sequence,
            ],
        )?,
        ProjectionRecord::CapabilityGrant(grant) => {
            validate_projection_json(
                "capability_grant",
                &grant.capability_grant_id,
                "scope_json",
                &grant.scope_json,
            )?;
            validate_projection_json(
                "capability_grant",
                &grant.capability_grant_id,
                "subject_json",
                &grant.subject_json,
            )?;
            transaction.execute(
                "INSERT INTO capability_grants(
                capability_grant_id, capability_profile_id, scope_json, effect,
                subject_json, decision_source, persistence, explanation, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(capability_grant_id) DO UPDATE SET
                capability_profile_id = excluded.capability_profile_id,
                scope_json = excluded.scope_json,
                effect = excluded.effect,
                subject_json = excluded.subject_json,
                decision_source = excluded.decision_source,
                persistence = excluded.persistence,
                explanation = excluded.explanation,
                updated_sequence = excluded.updated_sequence",
            params![
                grant.capability_grant_id,
                grant.capability_profile_id,
                grant.scope_json,
                grant.effect,
                grant.subject_json,
                grant.decision_source,
                grant.persistence,
                grant.explanation,
                sequence,
            ],
            )?
        }
        ProjectionRecord::PermissionApproval(approval) => {
            validate_projection_json(
                "permission_approval",
                &approval.approval_id,
                "scope_json",
                &approval.scope_json,
            )?;
            validate_projection_json(
                "permission_approval",
                &approval.approval_id,
                "subject_json",
                &approval.subject_json,
            )?;
            transaction.execute(
                "INSERT INTO permission_approvals(
                approval_id, project_id, session_id, tool_call_id, capability_profile_id,
                scope_json, subject_json, status, requested_by, reason, decision,
                capability_grant_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(approval_id) DO UPDATE SET
                project_id = excluded.project_id,
                session_id = excluded.session_id,
                tool_call_id = excluded.tool_call_id,
                capability_profile_id = excluded.capability_profile_id,
                scope_json = excluded.scope_json,
                subject_json = excluded.subject_json,
                status = excluded.status,
                requested_by = excluded.requested_by,
                reason = excluded.reason,
                decision = excluded.decision,
                capability_grant_id = excluded.capability_grant_id,
                updated_sequence = excluded.updated_sequence",
            params![
                approval.approval_id,
                approval.project_id.as_str(),
                approval.session_id.as_ref().map(SessionId::as_str),
                approval.tool_call_id.as_ref().map(ToolCallId::as_str),
                approval.capability_profile_id,
                approval.scope_json,
                approval.subject_json,
                approval.status,
                approval.requested_by,
                approval.reason,
                approval.decision,
                approval.capability_grant_id,
                sequence,
            ],
            )?
        }
        ProjectionRecord::ConnectivityExposure(exposure) => transaction.execute(
            "INSERT INTO connectivity_exposures(
                exposure_id, project_id, connectivity_endpoint_id, owner_kind, owner_id,
                channel_kind, exposure, permission_scope, status, capability_grant_id,
                health_status, reachable, revoked_at, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(exposure_id) DO UPDATE SET
                project_id = excluded.project_id,
                connectivity_endpoint_id = excluded.connectivity_endpoint_id,
                owner_kind = excluded.owner_kind,
                owner_id = excluded.owner_id,
                channel_kind = excluded.channel_kind,
                exposure = excluded.exposure,
                permission_scope = excluded.permission_scope,
                status = excluded.status,
                capability_grant_id = excluded.capability_grant_id,
                health_status = excluded.health_status,
                reachable = excluded.reachable,
                revoked_at = excluded.revoked_at,
                updated_sequence = excluded.updated_sequence",
            params![
                exposure.exposure_id,
                exposure.project_id.as_str(),
                exposure.connectivity_endpoint_id,
                exposure.owner_kind,
                exposure.owner_id,
                exposure.channel_kind,
                exposure.exposure,
                exposure.permission_scope,
                exposure.status,
                exposure.capability_grant_id,
                exposure.health_status,
                if exposure.reachable { 1 } else { 0 },
                exposure.revoked_at,
                sequence,
            ],
        )?,
        ProjectionRecord::RuntimeTarget(target) => transaction.execute(
            "INSERT INTO runtime_targets(
                runtime_target_id, project_id, name, runner_kind, workspace_root,
                artifact_root, default_cwd, capability_profile_id, connectivity_endpoint_id,
                status, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(runtime_target_id) DO UPDATE SET
                project_id = excluded.project_id,
                name = excluded.name,
                runner_kind = excluded.runner_kind,
                workspace_root = excluded.workspace_root,
                artifact_root = excluded.artifact_root,
                default_cwd = excluded.default_cwd,
                capability_profile_id = excluded.capability_profile_id,
                connectivity_endpoint_id = excluded.connectivity_endpoint_id,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                target.runtime_target_id,
                target.project_id.as_str(),
                target.name,
                target.runner_kind,
                target.workspace_root,
                target.artifact_root,
                target.default_cwd,
                target.capability_profile_id,
                target.connectivity_endpoint_id,
                target.status,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterReadiness(readiness) => transaction.execute(
            "INSERT INTO adapter_readiness(
                adapter_kind, project_id, program, opt_in_env, opted_in, smoke_status,
                credential_policy, expected_marker, env_allowlist_count, redaction_rule_count,
                output_limit_bytes, dogfood_blocker, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(adapter_kind, project_id) DO UPDATE SET
                program = excluded.program,
                opt_in_env = excluded.opt_in_env,
                opted_in = excluded.opted_in,
                smoke_status = excluded.smoke_status,
                credential_policy = excluded.credential_policy,
                expected_marker = excluded.expected_marker,
                env_allowlist_count = excluded.env_allowlist_count,
                redaction_rule_count = excluded.redaction_rule_count,
                output_limit_bytes = excluded.output_limit_bytes,
                dogfood_blocker = excluded.dogfood_blocker,
                updated_sequence = excluded.updated_sequence",
            params![
                readiness.adapter_kind,
                readiness.project_id.as_str(),
                readiness.program,
                readiness.opt_in_env,
                if readiness.opted_in { 1 } else { 0 },
                readiness.smoke_status,
                readiness.credential_policy,
                readiness.expected_marker,
                readiness.env_allowlist_count,
                readiness.redaction_rule_count,
                readiness.output_limit_bytes,
                readiness.dogfood_blocker,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterSmokeReport(report) => transaction.execute(
            "INSERT INTO adapter_smoke_reports(
                smoke_report_id, project_id, adapter_kind, smoke_status,
                credential_scan_status, marker_found, artifact_root, reason,
                dogfood_readiness_effect, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(smoke_report_id) DO UPDATE SET
                project_id = excluded.project_id,
                adapter_kind = excluded.adapter_kind,
                smoke_status = excluded.smoke_status,
                credential_scan_status = excluded.credential_scan_status,
                marker_found = excluded.marker_found,
                artifact_root = excluded.artifact_root,
                reason = excluded.reason,
                dogfood_readiness_effect = excluded.dogfood_readiness_effect,
                updated_sequence = excluded.updated_sequence",
            params![
                report.smoke_report_id,
                report.project_id.as_str(),
                report.adapter_kind,
                report.smoke_status,
                report.credential_scan_status,
                if report.marker_found { 1 } else { 0 },
                report.artifact_root,
                report.reason,
                report.dogfood_readiness_effect,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPlan(plan) => transaction.execute(
            "INSERT INTO adapter_dispatch_plans(
                dispatch_plan_id, project_id, adapter_kind, provider_kind,
                credential_scope, agent_id, agent_name, session_id, run_id,
                runtime_program, runtime_arg_count, runtime_prompt_policy, runtime_cwd,
                artifact_root, request_env_count, env_allowlist_count, redaction_rule_count,
                stdout_format, stderr_policy, provider_cli_executed, status, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
             ON CONFLICT(dispatch_plan_id) DO UPDATE SET
                project_id = excluded.project_id,
                adapter_kind = excluded.adapter_kind,
                provider_kind = excluded.provider_kind,
                credential_scope = excluded.credential_scope,
                agent_id = excluded.agent_id,
                agent_name = excluded.agent_name,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                runtime_program = excluded.runtime_program,
                runtime_arg_count = excluded.runtime_arg_count,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                runtime_cwd = excluded.runtime_cwd,
                artifact_root = excluded.artifact_root,
                request_env_count = excluded.request_env_count,
                env_allowlist_count = excluded.env_allowlist_count,
                redaction_rule_count = excluded.redaction_rule_count,
                stdout_format = excluded.stdout_format,
                stderr_policy = excluded.stderr_policy,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                updated_sequence = excluded.updated_sequence",
            params![
                plan.dispatch_plan_id,
                plan.project_id.as_str(),
                plan.adapter_kind,
                plan.provider_kind,
                plan.credential_scope,
                plan.agent_id.as_str(),
                plan.agent_name,
                plan.session_id.as_str(),
                plan.run_id.as_str(),
                plan.runtime_program,
                plan.runtime_arg_count,
                plan.runtime_prompt_policy,
                plan.runtime_cwd,
                plan.artifact_root,
                plan.request_env_count,
                plan.env_allowlist_count,
                plan.redaction_rule_count,
                plan.stdout_format,
                plan.stderr_policy,
                if plan.provider_cli_executed { 1 } else { 0 },
                plan.status,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchGate(gate) => transaction.execute(
            "INSERT INTO adapter_dispatch_gates(
                dispatch_gate_id, project_id, dispatch_plan_id, adapter_kind,
                provider_cli_execution_allowed, status, required_dogfood_gate,
                reason_codes, provider_cli_executed, runtime_prompt_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(dispatch_gate_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                adapter_kind = excluded.adapter_kind,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                status = excluded.status,
                required_dogfood_gate = excluded.required_dogfood_gate,
                reason_codes = excluded.reason_codes,
                provider_cli_executed = excluded.provider_cli_executed,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                gate.dispatch_gate_id,
                gate.project_id.as_str(),
                gate.dispatch_plan_id,
                gate.adapter_kind,
                if gate.provider_cli_execution_allowed { 1 } else { 0 },
                gate.status,
                gate.required_dogfood_gate,
                gate.reason_codes,
                if gate.provider_cli_executed { 1 } else { 0 },
                gate.runtime_prompt_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchReplay(replay) => transaction.execute(
            "INSERT INTO adapter_dispatch_replays(
                dispatch_replay_id, project_id, dispatch_plan_id, dispatch_gate_id,
                adapter_kind, session_id, run_id, fixture_path, fixture_hash,
                input_event_count, appended_event_count, tool_event_count,
                summary_event_count, completed_turn_count, provider_cli_executed,
                raw_content_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(dispatch_replay_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                dispatch_gate_id = excluded.dispatch_gate_id,
                adapter_kind = excluded.adapter_kind,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                fixture_path = excluded.fixture_path,
                fixture_hash = excluded.fixture_hash,
                input_event_count = excluded.input_event_count,
                appended_event_count = excluded.appended_event_count,
                tool_event_count = excluded.tool_event_count,
                summary_event_count = excluded.summary_event_count,
                completed_turn_count = excluded.completed_turn_count,
                provider_cli_executed = excluded.provider_cli_executed,
                raw_content_policy = excluded.raw_content_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                replay.dispatch_replay_id,
                replay.project_id.as_str(),
                replay.dispatch_plan_id,
                replay.dispatch_gate_id,
                replay.adapter_kind,
                replay.session_id.as_str(),
                replay.run_id.as_str(),
                replay.fixture_path,
                replay.fixture_hash,
                replay.input_event_count,
                replay.appended_event_count,
                replay.tool_event_count,
                replay.summary_event_count,
                replay.completed_turn_count,
                if replay.provider_cli_executed { 1 } else { 0 },
                replay.raw_content_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchExecutionRequest(request) => transaction.execute(
            "INSERT INTO adapter_dispatch_execution_requests(
                execution_request_id, project_id, dispatch_plan_id, dispatch_gate_id,
                adapter_kind, provider_cli_execution_allowed, provider_cli_executed,
                status, opt_in_env, runtime_prompt_policy, reason_codes, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(execution_request_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                dispatch_gate_id = excluded.dispatch_gate_id,
                adapter_kind = excluded.adapter_kind,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                opt_in_env = excluded.opt_in_env,
                runtime_prompt_policy = excluded.runtime_prompt_policy,
                reason_codes = excluded.reason_codes,
                updated_sequence = excluded.updated_sequence",
            params![
                request.execution_request_id,
                request.project_id.as_str(),
                request.dispatch_plan_id,
                request.dispatch_gate_id,
                request.adapter_kind,
                if request.provider_cli_execution_allowed { 1 } else { 0 },
                if request.provider_cli_executed { 1 } else { 0 },
                request.status,
                request.opt_in_env,
                request.runtime_prompt_policy,
                request.reason_codes,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchExecution(execution) => transaction.execute(
            "INSERT INTO adapter_dispatch_executions(
                dispatch_execution_id, project_id, dispatch_plan_id,
                execution_request_id, adapter_kind, session_id, run_id,
                provider_cli_execution_allowed, provider_cli_executed, status,
                exit_code, runtime_process_ref, stdout_artifact_id, stderr_artifact_id,
                artifact_root, credential_scan_status, raw_prompt_policy,
                raw_output_policy, reason_codes, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
             ON CONFLICT(dispatch_execution_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                execution_request_id = excluded.execution_request_id,
                adapter_kind = excluded.adapter_kind,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                provider_cli_execution_allowed = excluded.provider_cli_execution_allowed,
                provider_cli_executed = excluded.provider_cli_executed,
                status = excluded.status,
                exit_code = excluded.exit_code,
                runtime_process_ref = excluded.runtime_process_ref,
                stdout_artifact_id = excluded.stdout_artifact_id,
                stderr_artifact_id = excluded.stderr_artifact_id,
                artifact_root = excluded.artifact_root,
                credential_scan_status = excluded.credential_scan_status,
                raw_prompt_policy = excluded.raw_prompt_policy,
                raw_output_policy = excluded.raw_output_policy,
                reason_codes = excluded.reason_codes,
                updated_sequence = excluded.updated_sequence",
            params![
                execution.dispatch_execution_id,
                execution.project_id.as_str(),
                execution.dispatch_plan_id,
                execution.execution_request_id,
                execution.adapter_kind,
                execution.session_id.as_str(),
                execution.run_id.as_str(),
                if execution.provider_cli_execution_allowed { 1 } else { 0 },
                if execution.provider_cli_executed { 1 } else { 0 },
                execution.status,
                execution.exit_code,
                execution.runtime_process_ref,
                execution.stdout_artifact_id,
                execution.stderr_artifact_id,
                execution.artifact_root,
                execution.credential_scan_status,
                execution.raw_prompt_policy,
                execution.raw_output_policy,
                execution.reason_codes,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPromptSource(source) => transaction.execute(
            "INSERT INTO adapter_dispatch_prompt_sources(
                prompt_source_id, project_id, dispatch_plan_id, prompt_hash,
                source_kind, source_ref, source_hash, materialization_status,
                raw_prompt_policy, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(prompt_source_id) DO UPDATE SET
                project_id = excluded.project_id,
                dispatch_plan_id = excluded.dispatch_plan_id,
                prompt_hash = excluded.prompt_hash,
                source_kind = excluded.source_kind,
                source_ref = excluded.source_ref,
                source_hash = excluded.source_hash,
                materialization_status = excluded.materialization_status,
                raw_prompt_policy = excluded.raw_prompt_policy,
                updated_sequence = excluded.updated_sequence",
            params![
                source.prompt_source_id,
                source.project_id.as_str(),
                source.dispatch_plan_id,
                source.prompt_hash,
                source.source_kind,
                source.source_ref,
                source.source_hash,
                source.materialization_status,
                source.raw_prompt_policy,
                sequence,
            ],
        )?,
        ProjectionRecord::AdapterDispatchPromptMaterialization(materialization) => {
            transaction.execute(
                "INSERT INTO adapter_dispatch_prompt_materializations(
                    materialization_id, project_id, dispatch_plan_id, prompt_source_id,
                    source_kind, source_ref, expected_source_hash, observed_source_hash,
                    expected_prompt_hash, materialized_prompt_hash, status,
                    raw_prompt_policy, reason_codes, updated_sequence
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(materialization_id) DO UPDATE SET
                    project_id = excluded.project_id,
                    dispatch_plan_id = excluded.dispatch_plan_id,
                    prompt_source_id = excluded.prompt_source_id,
                    source_kind = excluded.source_kind,
                    source_ref = excluded.source_ref,
                    expected_source_hash = excluded.expected_source_hash,
                    observed_source_hash = excluded.observed_source_hash,
                    expected_prompt_hash = excluded.expected_prompt_hash,
                    materialized_prompt_hash = excluded.materialized_prompt_hash,
                    status = excluded.status,
                    raw_prompt_policy = excluded.raw_prompt_policy,
                    reason_codes = excluded.reason_codes,
                    updated_sequence = excluded.updated_sequence",
                params![
                    materialization.materialization_id,
                    materialization.project_id.as_str(),
                    materialization.dispatch_plan_id,
                    materialization.prompt_source_id,
                    materialization.source_kind,
                    materialization.source_ref,
                    materialization.expected_source_hash,
                    materialization.observed_source_hash,
                    materialization.expected_prompt_hash,
                    materialization.materialized_prompt_hash,
                    materialization.status,
                    materialization.raw_prompt_policy,
                    materialization.reason_codes,
                    sequence,
                ],
            )
        }?,
        ProjectionRecord::ToolCall(tool_call) => transaction.execute(
            "INSERT INTO tool_calls(
                tool_call_id, session_id, turn_id, tool_name, tool_origin, status,
                input_artifact_id, output_artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(tool_call_id) DO UPDATE SET
                session_id = excluded.session_id,
                turn_id = excluded.turn_id,
                tool_name = excluded.tool_name,
                tool_origin = excluded.tool_origin,
                status = excluded.status,
                input_artifact_id = excluded.input_artifact_id,
                output_artifact_id = excluded.output_artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                tool_call.tool_call_id.as_str(),
                tool_call.session_id.as_str(),
                tool_call.turn_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id,
                tool_call.output_artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::ToolObservation(observation) => transaction.execute(
            "INSERT INTO tool_observations(
                tool_observation_id, session_id, tool_call_id, source, external_tool_ref,
                tool_name, observed_status, instrumentation_level, confidence,
                raw_event_hash, artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(tool_observation_id) DO UPDATE SET
                session_id = excluded.session_id,
                tool_call_id = excluded.tool_call_id,
                source = excluded.source,
                external_tool_ref = excluded.external_tool_ref,
                tool_name = excluded.tool_name,
                observed_status = excluded.observed_status,
                instrumentation_level = excluded.instrumentation_level,
                confidence = excluded.confidence,
                raw_event_hash = excluded.raw_event_hash,
                artifact_id = excluded.artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                observation.tool_observation_id,
                observation.session_id.as_str(),
                observation.tool_call_id.as_ref().map(ToolCallId::as_str),
                observation.source,
                observation.external_tool_ref,
                observation.tool_name,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.raw_event_hash,
                observation.artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::MemoryPacketRef(packet) => transaction.execute(
            "INSERT INTO memory_packet_refs(
                memory_packet_id, project_id, task_id, agent_id, session_id, run_id,
                turn_id, packet_artifact_id, purpose, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(memory_packet_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                agent_id = excluded.agent_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                turn_id = excluded.turn_id,
                packet_artifact_id = excluded.packet_artifact_id,
                purpose = excluded.purpose,
                updated_sequence = excluded.updated_sequence",
            params![
                packet.memory_packet_id.as_str(),
                packet.project_id.as_str(),
                packet.task_id.as_ref().map(TaskId::as_str),
                packet.agent_id.as_ref().map(AgentId::as_str),
                packet.session_id.as_ref().map(SessionId::as_str),
                packet.run_id.as_ref().map(RunId::as_str),
                packet.turn_id,
                packet.packet_artifact_id,
                packet.purpose,
                sequence,
            ],
        )?,
        ProjectionRecord::MemoryRecord(memory_record) => transaction.execute(
            "INSERT INTO memory_records(
                memory_record_id, project_id, scope, scope_owner_ref, subject_ref,
                sensitivity_classification, record_kind, subject, predicate, object, body,
                confidence, review_state, source_count, valid_from, valid_until,
                supersedes_memory_record_id, revoked_by_memory_record_id, redaction_state,
                invalidated_at, invalidation_reason, packet_item_ref, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
             ON CONFLICT(memory_record_id) DO UPDATE SET
                project_id = excluded.project_id,
                scope = excluded.scope,
                scope_owner_ref = excluded.scope_owner_ref,
                subject_ref = excluded.subject_ref,
                sensitivity_classification = excluded.sensitivity_classification,
                record_kind = excluded.record_kind,
                subject = excluded.subject,
                predicate = excluded.predicate,
                object = excluded.object,
                body = excluded.body,
                confidence = excluded.confidence,
                review_state = excluded.review_state,
                source_count = excluded.source_count,
                valid_from = excluded.valid_from,
                valid_until = excluded.valid_until,
                supersedes_memory_record_id = excluded.supersedes_memory_record_id,
                revoked_by_memory_record_id = excluded.revoked_by_memory_record_id,
                redaction_state = excluded.redaction_state,
                invalidated_at = excluded.invalidated_at,
                invalidation_reason = excluded.invalidation_reason,
                packet_item_ref = excluded.packet_item_ref,
                updated_sequence = excluded.updated_sequence",
            params![
                memory_record.memory_record_id,
                memory_record.project_id.as_str(),
                memory_record.scope,
                memory_record.scope_owner_ref,
                memory_record.subject_ref,
                memory_record.sensitivity_classification,
                memory_record.record_kind,
                memory_record.subject,
                memory_record.predicate,
                memory_record.object,
                memory_record.body,
                memory_record.confidence,
                memory_record.review_state,
                memory_record.source_count,
                memory_record.valid_from,
                memory_record.valid_until,
                memory_record.supersedes_memory_record_id,
                memory_record.revoked_by_memory_record_id,
                memory_record.redaction_state,
                memory_record.invalidated_at,
                memory_record.invalidation_reason,
                memory_record.packet_item_ref,
                sequence,
            ],
        )?,
        ProjectionRecord::MemorySource(source) => transaction.execute(
            "INSERT INTO memory_sources(
                memory_source_id, memory_record_id, source_kind, source_event_id,
                source_artifact_id, source_path, source_anchor, source_content_hash,
                source_sequence, quote_artifact_id, observed_at, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(memory_source_id) DO UPDATE SET
                memory_record_id = excluded.memory_record_id,
                source_kind = excluded.source_kind,
                source_event_id = excluded.source_event_id,
                source_artifact_id = excluded.source_artifact_id,
                source_path = excluded.source_path,
                source_anchor = excluded.source_anchor,
                source_content_hash = excluded.source_content_hash,
                source_sequence = excluded.source_sequence,
                quote_artifact_id = excluded.quote_artifact_id,
                observed_at = excluded.observed_at,
                updated_sequence = excluded.updated_sequence",
            params![
                source.memory_source_id,
                source.memory_record_id,
                source.source_kind,
                source.source_event_id,
                source.source_artifact_id,
                source.source_path,
                source.source_anchor,
                source.source_content_hash,
                source.source_sequence,
                source.quote_artifact_id,
                source.observed_at,
                sequence,
            ],
        )?,
        ProjectionRecord::Evidence(evidence) => transaction.execute(
            "INSERT INTO evidence(
                evidence_id, project_id, task_id, session_id, run_id, kind,
                artifact_id, confidence, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(evidence_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                kind = excluded.kind,
                artifact_id = excluded.artifact_id,
                confidence = excluded.confidence,
                updated_sequence = excluded.updated_sequence",
            params![
                evidence.evidence_id.as_str(),
                evidence.project_id.as_str(),
                evidence.task_id.as_ref().map(TaskId::as_str),
                evidence.session_id.as_ref().map(SessionId::as_str),
                evidence.run_id.as_ref().map(RunId::as_str),
                evidence.kind,
                evidence.artifact_id,
                evidence.confidence,
                sequence,
            ],
        )?,
        ProjectionRecord::TaskOutcomeReport(report) => transaction.execute(
            "INSERT INTO task_outcome_reports(
                task_outcome_report_id, project_id, task_id, session_id, run_id,
                outcome_status, started_sequence, completed_sequence, duration_sequence_span,
                action_count, tool_call_count, evidence_count, memory_packet_count,
                confidence, blocker, review_outcome, report_artifact_id, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(task_outcome_report_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                outcome_status = excluded.outcome_status,
                started_sequence = excluded.started_sequence,
                completed_sequence = excluded.completed_sequence,
                duration_sequence_span = excluded.duration_sequence_span,
                action_count = excluded.action_count,
                tool_call_count = excluded.tool_call_count,
                evidence_count = excluded.evidence_count,
                memory_packet_count = excluded.memory_packet_count,
                confidence = excluded.confidence,
                blocker = excluded.blocker,
                review_outcome = excluded.review_outcome,
                report_artifact_id = excluded.report_artifact_id,
                updated_sequence = excluded.updated_sequence",
            params![
                report.task_outcome_report_id,
                report.project_id.as_str(),
                report.task_id.as_str(),
                report.session_id.as_str(),
                report.run_id.as_str(),
                report.outcome_status,
                report.started_sequence,
                report.completed_sequence,
                report.duration_sequence_span,
                report.action_count,
                report.tool_call_count,
                report.evidence_count,
                report.memory_packet_count,
                report.confidence,
                report.blocker,
                report.review_outcome,
                report.report_artifact_id,
                sequence,
            ],
        )?,
        ProjectionRecord::ReviewFinding(finding) => transaction.execute(
            "INSERT INTO review_findings(
                review_finding_id, project_id, task_id, session_id, run_id, tool_call_id,
                workpad_task_id, reviewer, finding_kind, severity, summary, status,
                evidence_artifact_id, follow_up, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(review_finding_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                session_id = excluded.session_id,
                run_id = excluded.run_id,
                tool_call_id = excluded.tool_call_id,
                workpad_task_id = excluded.workpad_task_id,
                reviewer = excluded.reviewer,
                finding_kind = excluded.finding_kind,
                severity = excluded.severity,
                summary = excluded.summary,
                status = excluded.status,
                evidence_artifact_id = excluded.evidence_artifact_id,
                follow_up = excluded.follow_up,
                updated_sequence = excluded.updated_sequence",
            params![
                finding.review_finding_id,
                finding.project_id.as_str(),
                finding.task_id.as_str(),
                finding.session_id.as_str(),
                finding.run_id.as_ref().map(RunId::as_str),
                finding.tool_call_id.as_ref().map(ToolCallId::as_str),
                finding.workpad_task_id,
                finding.reviewer,
                finding.finding_kind,
                finding.severity,
                finding.summary,
                finding.status,
                finding.evidence_artifact_id,
                finding.follow_up,
                sequence,
            ],
        )?,
        ProjectionRecord::WorkpadIndexReset(reset) => {
            transaction.execute(
                "DELETE FROM workpad_files WHERE project_id = ?1",
                params![reset.project_id.as_str()],
            )?;
            transaction.execute(
                "DELETE FROM workpad_tasks WHERE project_id = ?1",
                params![reset.project_id.as_str()],
            )?
        }
        ProjectionRecord::WorkpadFile(file) => transaction.execute(
            "INSERT INTO workpad_files(
                path, project_id, content_hash, headings, objective, observed_unix,
                updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                project_id = excluded.project_id,
                content_hash = excluded.content_hash,
                headings = excluded.headings,
                objective = excluded.objective,
                observed_unix = excluded.observed_unix,
                updated_sequence = excluded.updated_sequence",
            params![
                file.path,
                file.project_id.as_str(),
                file.content_hash,
                file.headings,
                file.objective,
                file.observed_unix,
                sequence,
            ],
        )?,
        ProjectionRecord::WorkpadTask(task) => transaction.execute(
            "INSERT INTO workpad_tasks(
                workpad_task_id, project_id, path, source_anchor, title, observed_status,
                capo_execution_status, observed_unix, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(workpad_task_id) DO UPDATE SET
                project_id = excluded.project_id,
                path = excluded.path,
                source_anchor = excluded.source_anchor,
                title = excluded.title,
                observed_status = excluded.observed_status,
                capo_execution_status = excluded.capo_execution_status,
                observed_unix = excluded.observed_unix,
                updated_sequence = excluded.updated_sequence",
            params![
                task.workpad_task_id,
                task.project_id.as_str(),
                task.path,
                task.source_anchor,
                task.title,
                task.observed_status,
                task.capo_execution_status,
                task.observed_unix,
                sequence,
            ],
        )?,
    };
    Ok(())
}

fn optional_id<T>(value: Option<String>) -> Option<T>
where
    T: FromStringId,
{
    value.map(T::from_string_id)
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

trait FromStringId {
    fn from_string_id(value: String) -> Self;
}

macro_rules! impl_from_string_id {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl FromStringId for $ty {
                fn from_string_id(value: String) -> Self {
                    Self::new(value)
                }
            }
        )+
    };
}

impl_from_string_id!(
    AgentId,
    EvidenceId,
    MemoryPacketId,
    ProjectId,
    RunId,
    SessionId,
    TaskId,
    ToolCallId,
);

#[cfg(test)]
mod tests;
