//! SQLite-backed event store and projection scaffold.
//!
//! P2 keeps the store deliberately small but real: events are append-only,
//! projection updates are stored as replayable records, artifacts are explicit
//! rows, and read models can be rebuilt from the projection log.

use std::fs;
use std::path::{Path, PathBuf};
use std::{error, fmt};

use capo_core::{
    AgentId, BoundaryBinding, BoundaryKind, EvidenceId, MemoryPacketId, ProjectId, RunId,
    SessionId, TaskId, ToolCallId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

/// Name of the first durable local state backend.
pub const PROTOTYPE_STATE_BACKEND: &str = "sqlite";

pub type StateResult<T> = Result<T, StateError>;

#[derive(Debug)]
pub enum StateError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    MissingRecoveryAttempt(String),
    MissingReadModel {
        kind: &'static str,
        id: String,
    },
    PermissionApprovalNotPending {
        approval_id: String,
        status: String,
    },
    InvalidProjectionJson {
        kind: &'static str,
        id: String,
        field: &'static str,
        error: String,
    },
    UnsafeArtifactRedactionState(RedactionState),
}

impl From<std::io::Error> for StateError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StateError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    ProjectRegistered,
    TaskDiscovered,
    AgentRegistered,
    SessionStarted,
    SessionRedirected,
    SessionSummaryUpdated,
    RunStarted,
    RunExited,
    PermissionRequested,
    PermissionDecided,
    PermissionApprovalQueued,
    CapabilityGrantCreated,
    CapabilityGrantUsed,
    ToolCallRequested,
    ToolInvocationStarted,
    ToolOutputArtifactRecorded,
    ToolOutputObserved,
    ToolCallCompleted,
    ToolResultDelivered,
    MemoryPacketBuilt,
    MemoryRecordIngested,
    MemoryRecordInvalidated,
    TaskOutcomeReportGenerated,
    ReviewFindingRecorded,
    EvidenceRecorded,
    WorkpadIndexed,
    WorkpadTaskImported,
    WorkpadProposalWritten,
    RecoveryStarted,
    RecoveryCompleted,
    SessionInterrupted,
    SessionStopped,
}

impl EventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRegistered => "project.registered",
            Self::TaskDiscovered => "task.discovered",
            Self::AgentRegistered => "agent.registered",
            Self::SessionStarted => "session.started",
            Self::SessionRedirected => "session.redirected",
            Self::SessionSummaryUpdated => "session.summary_updated",
            Self::RunStarted => "run.started",
            Self::RunExited => "run.exited",
            Self::PermissionRequested => "permission.requested",
            Self::PermissionDecided => "permission.decided",
            Self::PermissionApprovalQueued => "permission.approval_queued",
            Self::CapabilityGrantCreated => "capability.grant_created",
            Self::CapabilityGrantUsed => "capability.grant_used",
            Self::ToolCallRequested => "tool.call_requested",
            Self::ToolInvocationStarted => "tool.invocation_started",
            Self::ToolOutputArtifactRecorded => "tool.output_artifact_recorded",
            Self::ToolOutputObserved => "tool.output_observed",
            Self::ToolCallCompleted => "tool.call_completed",
            Self::ToolResultDelivered => "tool.result_delivered",
            Self::MemoryPacketBuilt => "memory.packet_built",
            Self::MemoryRecordIngested => "memory.record_ingested",
            Self::MemoryRecordInvalidated => "memory.record_invalidated",
            Self::TaskOutcomeReportGenerated => "task.outcome_report_generated",
            Self::ReviewFindingRecorded => "review.finding_recorded",
            Self::EvidenceRecorded => "evidence.recorded",
            Self::WorkpadIndexed => "workpad.indexed",
            Self::WorkpadTaskImported => "workpad.task_imported",
            Self::WorkpadProposalWritten => "workpad.proposal_written",
            Self::RecoveryStarted => "recovery.started",
            Self::RecoveryCompleted => "recovery.completed",
            Self::SessionInterrupted => "session.interrupted",
            Self::SessionStopped => "session.stopped",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionState {
    Safe,
    Redacted,
    Unknown,
    ContainsSensitive,
}

impl RedactionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Redacted => "redacted",
            Self::Unknown => "unknown",
            Self::ContainsSensitive => "contains_sensitive",
        }
    }

    pub const fn is_persistable_artifact(self) -> bool {
        matches!(self, Self::Safe | Self::Redacted)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewEvent {
    pub event_id: String,
    pub kind: EventKind,
    pub actor: String,
    pub project_id: Option<ProjectId>,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload_json: String,
    pub idempotency_key: Option<String>,
    pub redaction_state: RedactionState,
}

impl NewEvent {
    pub fn new(event_id: impl Into<String>, kind: EventKind, actor: impl Into<String>) -> Self {
        Self {
            event_id: event_id.into(),
            kind,
            actor: actor.into(),
            project_id: None,
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: None,
            payload_json: "{}".to_string(),
            idempotency_key: None,
            redaction_state: RedactionState::Safe,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionRecord {
    Project(ProjectProjection),
    Task(TaskProjection),
    Agent(AgentProjection),
    Session(SessionProjection),
    Run(RunProjection),
    CapabilityGrant(CapabilityGrantProjection),
    PermissionApproval(PermissionApprovalProjection),
    ToolCall(ToolCallProjection),
    MemoryPacketRef(MemoryPacketProjection),
    MemoryRecord(Box<MemoryRecordProjection>),
    MemorySource(MemorySourceProjection),
    TaskOutcomeReport(TaskOutcomeReportProjection),
    ReviewFinding(ReviewFindingProjection),
    Evidence(EvidenceProjection),
    WorkpadIndexReset(WorkpadIndexResetProjection),
    WorkpadFile(WorkpadFileProjection),
    WorkpadTask(WorkpadTaskProjection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProjection {
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskProjection {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub capo_execution_status: String,
    pub active_session_id: Option<SessionId>,
    pub latest_summary: Option<String>,
    pub evidence_id: Option<EvidenceId>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProjection {
    pub agent_id: AgentId,
    pub project_id: ProjectId,
    pub name: String,
    pub status: String,
    pub current_session_id: Option<SessionId>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionProjection {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: AgentId,
    pub title: String,
    pub status: String,
    pub current_goal: String,
    pub latest_summary: Option<String>,
    pub latest_confidence: Option<i64>,
    pub latest_blocker: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunProjection {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub status: String,
    pub recovery_of_run_id: Option<RunId>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityGrantProjection {
    pub capability_grant_id: String,
    pub capability_profile_id: String,
    pub scope_json: String,
    pub effect: String,
    pub subject_json: String,
    pub decision_source: String,
    pub persistence: String,
    pub explanation: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionApprovalProjection {
    pub approval_id: String,
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub tool_call_id: Option<ToolCallId>,
    pub capability_profile_id: String,
    pub scope_json: String,
    pub subject_json: String,
    pub status: String,
    pub requested_by: String,
    pub reason: String,
    pub decision: Option<String>,
    pub capability_grant_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallProjection {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub tool_origin: String,
    pub status: String,
    pub input_artifact_id: Option<String>,
    pub output_artifact_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPacketProjection {
    pub memory_packet_id: MemoryPacketId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub packet_artifact_id: Option<String>,
    pub purpose: String,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecordProjection {
    pub memory_record_id: String,
    pub project_id: ProjectId,
    pub scope: String,
    pub scope_owner_ref: String,
    pub subject_ref: Option<String>,
    pub sensitivity_classification: String,
    pub record_kind: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub body: String,
    pub confidence: String,
    pub review_state: String,
    pub source_count: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub supersedes_memory_record_id: Option<String>,
    pub revoked_by_memory_record_id: Option<String>,
    pub redaction_state: String,
    pub invalidated_at: Option<String>,
    pub invalidation_reason: Option<String>,
    pub packet_item_ref: Option<String>,
    pub updated_sequence: i64,
}

impl MemoryRecordProjection {
    pub fn is_packet_eligible(&self) -> bool {
        self.review_state == "reviewed"
            && self.invalidated_at.is_none()
            && self.valid_until.is_none()
            && self.revoked_by_memory_record_id.is_none()
            && self.redaction_state != RedactionState::ContainsSensitive.as_str()
            && self.redaction_state != RedactionState::Unknown.as_str()
            && self.sensitivity_classification != "secret_derived"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySourceProjection {
    pub memory_source_id: String,
    pub memory_record_id: String,
    pub source_kind: String,
    pub source_event_id: Option<String>,
    pub source_artifact_id: Option<String>,
    pub source_path: Option<String>,
    pub source_anchor: Option<String>,
    pub source_content_hash: Option<String>,
    pub source_sequence: Option<i64>,
    pub quote_artifact_id: Option<String>,
    pub observed_at: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceProjection {
    pub evidence_id: EvidenceId,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub kind: String,
    pub artifact_id: Option<String>,
    pub confidence: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskOutcomeReportProjection {
    pub task_outcome_report_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub outcome_status: String,
    pub started_sequence: i64,
    pub completed_sequence: i64,
    pub duration_sequence_span: i64,
    pub action_count: i64,
    pub tool_call_count: i64,
    pub evidence_count: i64,
    pub memory_packet_count: i64,
    pub confidence: Option<i64>,
    pub blocker: Option<String>,
    pub review_outcome: String,
    pub report_artifact_id: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewFindingProjection {
    pub review_finding_id: String,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub tool_call_id: Option<ToolCallId>,
    pub workpad_task_id: Option<String>,
    pub reviewer: String,
    pub finding_kind: String,
    pub severity: String,
    pub summary: String,
    pub status: String,
    pub evidence_artifact_id: Option<String>,
    pub follow_up: Option<String>,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadIndexResetProjection {
    pub project_id: ProjectId,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadFileProjection {
    pub path: String,
    pub project_id: ProjectId,
    pub content_hash: String,
    pub headings: String,
    pub objective: Option<String>,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadTaskProjection {
    pub workpad_task_id: String,
    pub project_id: ProjectId,
    pub path: String,
    pub source_anchor: String,
    pub title: String,
    pub observed_status: String,
    pub capo_execution_status: String,
    pub observed_unix: i64,
    pub updated_sequence: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventRecord {
    pub sequence: i64,
    pub event_id: String,
    pub kind: String,
    pub actor: String,
    pub project_id: Option<ProjectId>,
    pub task_id: Option<TaskId>,
    pub agent_id: Option<AgentId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub payload_json: String,
    pub idempotency_key: Option<String>,
    pub redaction_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub project_id: Option<ProjectId>,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub kind: String,
    pub uri: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub redaction_state: RedactionState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryAttempt {
    pub recovery_attempt_id: String,
    pub status: String,
    pub started_sequence: i64,
    pub completed_sequence: Option<i64>,
}

fn migrate(connection: &mut Connection) -> StateResult<()> {
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
        CREATE TABLE IF NOT EXISTS events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            actor TEXT NOT NULL,
            project_id TEXT,
            task_id TEXT,
            agent_id TEXT,
            session_id TEXT,
            run_id TEXT,
            turn_id TEXT,
            item_id TEXT,
            payload_json TEXT NOT NULL,
            idempotency_key TEXT,
            redaction_state TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_events_project_idempotency
        ON events(project_id, idempotency_key)
        WHERE project_id IS NOT NULL AND idempotency_key IS NOT NULL;
        CREATE TABLE IF NOT EXISTS projection_records (
            sequence INTEGER NOT NULL,
            projection_kind TEXT NOT NULL,
            record_id TEXT NOT NULL,
            a TEXT,
            b TEXT,
            c TEXT,
            d TEXT,
            e TEXT,
            f TEXT,
            g TEXT,
            h TEXT,
            payload_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS projection_watermarks (
            name TEXT PRIMARY KEY,
            last_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            artifact_id TEXT PRIMARY KEY,
            project_id TEXT,
            session_id TEXT,
            run_id TEXT,
            kind TEXT NOT NULL,
            uri TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            redaction_state TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS projects (
            project_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            title TEXT NOT NULL,
            capo_execution_status TEXT NOT NULL,
            active_session_id TEXT,
            latest_summary TEXT,
            evidence_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            current_session_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            agent_id TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL,
            current_goal TEXT NOT NULL,
            latest_summary TEXT,
            latest_confidence INTEGER,
            latest_blocker TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS runs (
            run_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            status TEXT NOT NULL,
            recovery_of_run_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS capability_grants (
            capability_grant_id TEXT PRIMARY KEY,
            capability_profile_id TEXT NOT NULL,
            scope_json TEXT NOT NULL,
            effect TEXT NOT NULL,
            subject_json TEXT NOT NULL,
            decision_source TEXT NOT NULL DEFAULT 'unknown',
            persistence TEXT NOT NULL DEFAULT 'unknown',
            explanation TEXT NOT NULL DEFAULT '',
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS permission_approvals (
            approval_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            session_id TEXT,
            tool_call_id TEXT,
            capability_profile_id TEXT NOT NULL,
            scope_json TEXT NOT NULL,
            subject_json TEXT NOT NULL,
            status TEXT NOT NULL,
            requested_by TEXT NOT NULL,
            reason TEXT NOT NULL,
            decision TEXT,
            capability_grant_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_calls (
            tool_call_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            turn_id TEXT,
            tool_name TEXT NOT NULL,
            tool_origin TEXT NOT NULL,
            status TEXT NOT NULL,
            input_artifact_id TEXT,
            output_artifact_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_packet_refs (
            memory_packet_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            agent_id TEXT,
            session_id TEXT,
            run_id TEXT,
            turn_id TEXT,
            packet_artifact_id TEXT,
            purpose TEXT NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_records (
            memory_record_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_owner_ref TEXT NOT NULL,
            subject_ref TEXT,
            sensitivity_classification TEXT NOT NULL,
            record_kind TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            body TEXT NOT NULL,
            confidence TEXT NOT NULL,
            review_state TEXT NOT NULL,
            source_count INTEGER NOT NULL,
            valid_from TEXT,
            valid_until TEXT,
            supersedes_memory_record_id TEXT,
            revoked_by_memory_record_id TEXT,
            redaction_state TEXT NOT NULL,
            invalidated_at TEXT,
            invalidation_reason TEXT,
            packet_item_ref TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_sources (
            memory_source_id TEXT PRIMARY KEY,
            memory_record_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_event_id TEXT,
            source_artifact_id TEXT,
            source_path TEXT,
            source_anchor TEXT,
            source_content_hash TEXT,
            source_sequence INTEGER,
            quote_artifact_id TEXT,
            observed_at TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS evidence (
            evidence_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT,
            session_id TEXT,
            run_id TEXT,
            kind TEXT NOT NULL,
            artifact_id TEXT,
            confidence INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS task_outcome_reports (
            task_outcome_report_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            outcome_status TEXT NOT NULL,
            started_sequence INTEGER NOT NULL,
            completed_sequence INTEGER NOT NULL,
            duration_sequence_span INTEGER NOT NULL,
            action_count INTEGER NOT NULL,
            tool_call_count INTEGER NOT NULL,
            evidence_count INTEGER NOT NULL,
            memory_packet_count INTEGER NOT NULL,
            confidence INTEGER,
            blocker TEXT,
            review_outcome TEXT NOT NULL,
            report_artifact_id TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS review_findings (
            review_finding_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT,
            tool_call_id TEXT,
            workpad_task_id TEXT,
            reviewer TEXT NOT NULL,
            finding_kind TEXT NOT NULL,
            severity TEXT NOT NULL,
            summary TEXT NOT NULL,
            status TEXT NOT NULL,
            evidence_artifact_id TEXT,
            follow_up TEXT,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workpad_files (
            path TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            headings TEXT NOT NULL,
            objective TEXT,
            observed_unix INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS workpad_tasks (
            workpad_task_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            path TEXT NOT NULL,
            source_anchor TEXT NOT NULL,
            title TEXT NOT NULL,
            observed_status TEXT NOT NULL,
            capo_execution_status TEXT NOT NULL,
            observed_unix INTEGER NOT NULL,
            updated_sequence INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS recovery_attempts (
            recovery_attempt_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            started_sequence INTEGER NOT NULL,
            completed_sequence INTEGER,
            notes TEXT NOT NULL
        );
        ",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "decision_source",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "persistence",
        "TEXT NOT NULL DEFAULT 'unknown'",
    )?;
    add_missing_column(
        connection,
        "capability_grants",
        "explanation",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn add_missing_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> StateResult<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn clear_projection_tables(transaction: &Transaction<'_>) -> StateResult<()> {
    for table in [
        "projects",
        "tasks",
        "agents",
        "sessions",
        "runs",
        "capability_grants",
        "permission_approvals",
        "tool_calls",
        "memory_packet_refs",
        "memory_records",
        "memory_sources",
        "evidence",
        "task_outcome_reports",
        "review_findings",
        "workpad_files",
        "workpad_tasks",
        "projection_watermarks",
    ] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
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

struct ProjectionRecordRow {
    kind: &'static str,
    record_id: String,
    a: Option<String>,
    b: Option<String>,
    c: Option<String>,
    d: Option<String>,
    e: Option<String>,
    f: Option<String>,
    g: Option<String>,
    h: Option<String>,
    payload_json: String,
}

fn projection_record_to_row(record: &ProjectionRecord) -> ProjectionRecordRow {
    match record {
        ProjectionRecord::Project(project) => ProjectionRecordRow {
            kind: "project",
            record_id: project.project_id.to_string(),
            a: Some(project.name.clone()),
            b: Some(project.status.clone()),
            c: None,
            d: None,
            e: None,
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::Task(task) => ProjectionRecordRow {
            kind: "task",
            record_id: task.task_id.to_string(),
            a: Some(task.project_id.to_string()),
            b: Some(task.title.clone()),
            c: Some(task.capo_execution_status.clone()),
            d: task.active_session_id.as_ref().map(ToString::to_string),
            e: task.latest_summary.clone(),
            f: task.evidence_id.as_ref().map(ToString::to_string),
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::Agent(agent) => ProjectionRecordRow {
            kind: "agent",
            record_id: agent.agent_id.to_string(),
            a: Some(agent.project_id.to_string()),
            b: Some(agent.name.clone()),
            c: Some(agent.status.clone()),
            d: agent.current_session_id.as_ref().map(ToString::to_string),
            e: None,
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::Session(session) => ProjectionRecordRow {
            kind: "session",
            record_id: session.session_id.to_string(),
            a: Some(session.project_id.to_string()),
            b: session.task_id.as_ref().map(ToString::to_string),
            c: Some(session.agent_id.to_string()),
            d: Some(session.title.clone()),
            e: Some(session.status.clone()),
            f: Some(session.current_goal.clone()),
            g: session.latest_summary.clone(),
            h: session.latest_confidence.map(|value| value.to_string()),
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::Run(run) => ProjectionRecordRow {
            kind: "run",
            record_id: run.run_id.to_string(),
            a: Some(run.session_id.to_string()),
            b: Some(run.status.clone()),
            c: run.recovery_of_run_id.as_ref().map(ToString::to_string),
            d: None,
            e: None,
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::CapabilityGrant(grant) => ProjectionRecordRow {
            kind: "capability_grant",
            record_id: grant.capability_grant_id.clone(),
            a: Some(grant.capability_profile_id.clone()),
            b: Some(grant.scope_json.clone()),
            c: Some(grant.effect.clone()),
            d: Some(grant.subject_json.clone()),
            e: Some(grant.decision_source.clone()),
            f: Some(grant.persistence.clone()),
            g: Some(grant.explanation.clone()),
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::PermissionApproval(approval) => ProjectionRecordRow {
            kind: "permission_approval",
            record_id: approval.approval_id.clone(),
            a: Some(approval.project_id.to_string()),
            b: approval.session_id.as_ref().map(ToString::to_string),
            c: approval.tool_call_id.as_ref().map(ToString::to_string),
            d: Some(approval.capability_profile_id.clone()),
            e: Some(approval.status.clone()),
            f: approval.decision.clone(),
            g: approval.capability_grant_id.clone(),
            h: Some(approval.scope_json.clone()),
            payload_json: format!(
                "{{\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\"}}",
                approval.subject_json,
                escape_json(&approval.requested_by),
                escape_json(&approval.reason)
            ),
        },
        ProjectionRecord::ToolCall(tool_call) => ProjectionRecordRow {
            kind: "tool_call",
            record_id: tool_call.tool_call_id.to_string(),
            a: Some(tool_call.session_id.to_string()),
            b: tool_call.turn_id.clone(),
            c: Some(tool_call.tool_name.clone()),
            d: Some(tool_call.tool_origin.clone()),
            e: Some(tool_call.status.clone()),
            f: tool_call.input_artifact_id.clone(),
            g: tool_call.output_artifact_id.clone(),
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::MemoryPacketRef(packet) => ProjectionRecordRow {
            kind: "memory_packet",
            record_id: packet.memory_packet_id.to_string(),
            a: Some(packet.project_id.to_string()),
            b: packet.task_id.as_ref().map(ToString::to_string),
            c: packet.agent_id.as_ref().map(ToString::to_string),
            d: packet.session_id.as_ref().map(ToString::to_string),
            e: packet.run_id.as_ref().map(ToString::to_string),
            f: packet.turn_id.clone(),
            g: packet.packet_artifact_id.clone(),
            h: Some(packet.purpose.clone()),
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::MemoryRecord(memory_record) => ProjectionRecordRow {
            kind: "memory_record",
            record_id: memory_record.memory_record_id.clone(),
            a: Some(memory_record.project_id.to_string()),
            b: Some(memory_record.scope.clone()),
            c: Some(memory_record.scope_owner_ref.clone()),
            d: memory_record.subject_ref.clone(),
            e: Some(memory_record.sensitivity_classification.clone()),
            f: Some(memory_record.record_kind.clone()),
            g: Some(memory_record.review_state.clone()),
            h: Some(memory_record.source_count.to_string()),
            payload_json: json!({
                "subject": memory_record.subject,
                "predicate": memory_record.predicate,
                "object": memory_record.object,
                "body": memory_record.body,
                "confidence": memory_record.confidence,
                "valid_from": memory_record.valid_from,
                "valid_until": memory_record.valid_until,
                "supersedes_memory_record_id": memory_record.supersedes_memory_record_id,
                "revoked_by_memory_record_id": memory_record.revoked_by_memory_record_id,
                "redaction_state": memory_record.redaction_state,
                "invalidated_at": memory_record.invalidated_at,
                "invalidation_reason": memory_record.invalidation_reason,
                "packet_item_ref": memory_record.packet_item_ref,
            })
            .to_string(),
        },
        ProjectionRecord::MemorySource(source) => ProjectionRecordRow {
            kind: "memory_source",
            record_id: source.memory_source_id.clone(),
            a: Some(source.memory_record_id.clone()),
            b: Some(source.source_kind.clone()),
            c: source.source_event_id.clone(),
            d: source.source_artifact_id.clone(),
            e: source.source_path.clone(),
            f: source.source_anchor.clone(),
            g: source.source_content_hash.clone(),
            h: source.source_sequence.map(|value| value.to_string()),
            payload_json: json!({
                "quote_artifact_id": source.quote_artifact_id,
                "observed_at": source.observed_at,
            })
            .to_string(),
        },
        ProjectionRecord::Evidence(evidence) => ProjectionRecordRow {
            kind: "evidence",
            record_id: evidence.evidence_id.to_string(),
            a: Some(evidence.project_id.to_string()),
            b: evidence.task_id.as_ref().map(ToString::to_string),
            c: evidence.session_id.as_ref().map(ToString::to_string),
            d: evidence.run_id.as_ref().map(ToString::to_string),
            e: Some(evidence.kind.clone()),
            f: evidence.artifact_id.clone(),
            g: Some(evidence.confidence.to_string()),
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::TaskOutcomeReport(report) => ProjectionRecordRow {
            kind: "task_outcome_report",
            record_id: report.task_outcome_report_id.clone(),
            a: Some(report.project_id.to_string()),
            b: Some(report.task_id.to_string()),
            c: Some(report.session_id.to_string()),
            d: Some(report.run_id.to_string()),
            e: Some(report.outcome_status.clone()),
            f: Some(report.started_sequence.to_string()),
            g: Some(report.completed_sequence.to_string()),
            h: Some(report.duration_sequence_span.to_string()),
            payload_json: json!({
                "action_count": report.action_count,
                "tool_call_count": report.tool_call_count,
                "evidence_count": report.evidence_count,
                "memory_packet_count": report.memory_packet_count,
                "confidence": report.confidence,
                "blocker": report.blocker,
                "review_outcome": report.review_outcome,
                "report_artifact_id": report.report_artifact_id,
            })
            .to_string(),
        },
        ProjectionRecord::ReviewFinding(finding) => ProjectionRecordRow {
            kind: "review_finding",
            record_id: finding.review_finding_id.clone(),
            a: Some(finding.project_id.to_string()),
            b: Some(finding.task_id.to_string()),
            c: Some(finding.session_id.to_string()),
            d: finding.run_id.as_ref().map(ToString::to_string),
            e: finding.tool_call_id.as_ref().map(ToString::to_string),
            f: finding.workpad_task_id.clone(),
            g: Some(finding.finding_kind.clone()),
            h: Some(finding.status.clone()),
            payload_json: json!({
                "reviewer": finding.reviewer,
                "severity": finding.severity,
                "summary": finding.summary,
                "evidence_artifact_id": finding.evidence_artifact_id,
                "follow_up": finding.follow_up,
            })
            .to_string(),
        },
        ProjectionRecord::WorkpadIndexReset(reset) => ProjectionRecordRow {
            kind: "workpad_index_reset",
            record_id: reset.project_id.to_string(),
            a: Some(reset.observed_unix.to_string()),
            b: None,
            c: None,
            d: None,
            e: None,
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::WorkpadFile(file) => ProjectionRecordRow {
            kind: "workpad_file",
            record_id: file.path.clone(),
            a: Some(file.project_id.to_string()),
            b: Some(file.content_hash.clone()),
            c: Some(file.headings.clone()),
            d: file.objective.clone(),
            e: Some(file.observed_unix.to_string()),
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
        },
        ProjectionRecord::WorkpadTask(task) => ProjectionRecordRow {
            kind: "workpad_task",
            record_id: task.workpad_task_id.clone(),
            a: Some(task.project_id.to_string()),
            b: Some(task.path.clone()),
            c: Some(task.source_anchor.clone()),
            d: Some(task.title.clone()),
            e: Some(task.observed_status.clone()),
            f: Some(task.capo_execution_status.clone()),
            g: Some(task.observed_unix.to_string()),
            h: None,
            payload_json: "{}".to_string(),
        },
    }
}

#[derive(Debug)]
struct ProjectionDecodeError(String);

impl fmt::Display for ProjectionDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for ProjectionDecodeError {}

#[allow(clippy::too_many_arguments)]
fn projection_record_from_row(
    projection_kind: String,
    record_id: String,
    a: Option<String>,
    b: Option<String>,
    c: Option<String>,
    d: Option<String>,
    e: Option<String>,
    f: Option<String>,
    g: Option<String>,
    h: Option<String>,
    payload_json: String,
) -> Result<ProjectionRecord, ProjectionDecodeError> {
    match projection_kind.as_str() {
        "project" => Ok(ProjectionRecord::Project(ProjectProjection {
            project_id: ProjectId::new(record_id),
            name: required_field(&projection_kind, "project", a, "name")?,
            status: required_field(&projection_kind, "project", b, "status")?,
            updated_sequence: 0,
        })),
        "task" => Ok(ProjectionRecord::Task(TaskProjection {
            task_id: TaskId::new(record_id),
            project_id: ProjectId::new(required_field(&projection_kind, "task", a, "project_id")?),
            title: required_field(&projection_kind, "task", b, "title")?,
            capo_execution_status: required_field(
                &projection_kind,
                "task",
                c,
                "capo_execution_status",
            )?,
            active_session_id: optional_id(d),
            latest_summary: e,
            evidence_id: optional_id(f),
            updated_sequence: 0,
        })),
        "agent" => Ok(ProjectionRecord::Agent(AgentProjection {
            agent_id: AgentId::new(record_id),
            project_id: ProjectId::new(required_field(&projection_kind, "agent", a, "project_id")?),
            name: required_field(&projection_kind, "agent", b, "name")?,
            status: required_field(&projection_kind, "agent", c, "status")?,
            current_session_id: optional_id(d),
            updated_sequence: 0,
        })),
        "session" => Ok(ProjectionRecord::Session(SessionProjection {
            session_id: SessionId::new(record_id),
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "session",
                a,
                "project_id",
            )?),
            task_id: optional_id(b),
            agent_id: AgentId::new(required_field(&projection_kind, "session", c, "agent_id")?),
            title: required_field(&projection_kind, "session", d, "title")?,
            status: required_field(&projection_kind, "session", e, "status")?,
            current_goal: required_field(&projection_kind, "session", f, "current_goal")?,
            latest_summary: g,
            latest_confidence: optional_i64(&projection_kind, "session", h, "latest_confidence")?,
            latest_blocker: None,
            updated_sequence: 0,
        })),
        "run" => Ok(ProjectionRecord::Run(RunProjection {
            run_id: RunId::new(record_id),
            session_id: SessionId::new(required_field(&projection_kind, "run", a, "session_id")?),
            status: required_field(&projection_kind, "run", b, "status")?,
            recovery_of_run_id: optional_id(c),
            updated_sequence: 0,
        })),
        "capability_grant" => Ok(ProjectionRecord::CapabilityGrant(
            CapabilityGrantProjection {
                capability_grant_id: record_id,
                capability_profile_id: required_field(
                    &projection_kind,
                    "capability_grant",
                    a,
                    "capability_profile_id",
                )?,
                scope_json: required_field(&projection_kind, "capability_grant", b, "scope_json")?,
                effect: required_field(&projection_kind, "capability_grant", c, "effect")?,
                subject_json: required_field(
                    &projection_kind,
                    "capability_grant",
                    d,
                    "subject_json",
                )?,
                decision_source: e.unwrap_or_else(|| "unknown".to_string()),
                persistence: f.unwrap_or_else(|| "unknown".to_string()),
                explanation: g.unwrap_or_default(),
                updated_sequence: 0,
            },
        )),
        "permission_approval" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "permission_approval",
                        a,
                        "project_id",
                    )?),
                    session_id: optional_id(b),
                    tool_call_id: optional_id(c),
                    capability_profile_id: required_field(
                        &projection_kind,
                        "permission_approval",
                        d,
                        "capability_profile_id",
                    )?,
                    scope_json: required_field(
                        &projection_kind,
                        "permission_approval",
                        h,
                        "scope_json",
                    )?,
                    subject_json: payload_string(&payload, "subject_json")
                        .unwrap_or_else(|| "{}".to_string()),
                    status: required_field(&projection_kind, "permission_approval", e, "status")?,
                    requested_by: payload_string(&payload, "requested_by")
                        .unwrap_or_else(|| "unknown".to_string()),
                    reason: payload_string(&payload, "reason").unwrap_or_default(),
                    decision: f,
                    capability_grant_id: g,
                    updated_sequence: 0,
                },
            ))
        }
        "tool_call" => Ok(ProjectionRecord::ToolCall(ToolCallProjection {
            tool_call_id: ToolCallId::new(record_id),
            session_id: SessionId::new(required_field(
                &projection_kind,
                "tool_call",
                a,
                "session_id",
            )?),
            turn_id: b,
            tool_name: required_field(&projection_kind, "tool_call", c, "tool_name")?,
            tool_origin: required_field(&projection_kind, "tool_call", d, "tool_origin")?,
            status: required_field(&projection_kind, "tool_call", e, "status")?,
            input_artifact_id: f,
            output_artifact_id: g,
            updated_sequence: 0,
        })),
        "memory_packet" => Ok(ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
            memory_packet_id: MemoryPacketId::new(record_id),
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "memory_packet",
                a,
                "project_id",
            )?),
            task_id: optional_id(b),
            agent_id: optional_id(c),
            session_id: optional_id(d),
            run_id: optional_id(e),
            turn_id: f,
            packet_artifact_id: g,
            purpose: required_field(&projection_kind, "memory_packet", h, "purpose")?,
            updated_sequence: 0,
        })),
        "memory_record" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::MemoryRecord(Box::new(
                MemoryRecordProjection {
                    memory_record_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "memory_record",
                        a,
                        "project_id",
                    )?),
                    scope: required_field(&projection_kind, "memory_record", b, "scope")?,
                    scope_owner_ref: required_field(
                        &projection_kind,
                        "memory_record",
                        c,
                        "scope_owner_ref",
                    )?,
                    subject_ref: d,
                    sensitivity_classification: required_field(
                        &projection_kind,
                        "memory_record",
                        e,
                        "sensitivity_classification",
                    )?,
                    record_kind: required_field(
                        &projection_kind,
                        "memory_record",
                        f,
                        "record_kind",
                    )?,
                    review_state: required_field(
                        &projection_kind,
                        "memory_record",
                        g,
                        "review_state",
                    )?,
                    source_count: required_i64(
                        &projection_kind,
                        "memory_record",
                        h,
                        "source_count",
                    )?,
                    subject: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "subject",
                    )?,
                    predicate: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "predicate",
                    )?,
                    object: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "object",
                    )?,
                    body: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "body",
                    )?,
                    confidence: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "confidence",
                    )?,
                    valid_from: payload_optional_string(&payload, "valid_from"),
                    valid_until: payload_optional_string(&payload, "valid_until"),
                    supersedes_memory_record_id: payload_optional_string(
                        &payload,
                        "supersedes_memory_record_id",
                    ),
                    revoked_by_memory_record_id: payload_optional_string(
                        &payload,
                        "revoked_by_memory_record_id",
                    ),
                    redaction_state: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "redaction_state",
                    )?,
                    invalidated_at: payload_optional_string(&payload, "invalidated_at"),
                    invalidation_reason: payload_optional_string(&payload, "invalidation_reason"),
                    packet_item_ref: payload_optional_string(&payload, "packet_item_ref"),
                    updated_sequence: 0,
                },
            )))
        }
        "memory_source" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::MemorySource(MemorySourceProjection {
                memory_source_id: record_id,
                memory_record_id: required_field(
                    &projection_kind,
                    "memory_source",
                    a,
                    "memory_record_id",
                )?,
                source_kind: required_field(&projection_kind, "memory_source", b, "source_kind")?,
                source_event_id: c,
                source_artifact_id: d,
                source_path: e,
                source_anchor: f,
                source_content_hash: g,
                source_sequence: optional_i64(
                    &projection_kind,
                    "memory_source",
                    h,
                    "source_sequence",
                )?,
                quote_artifact_id: payload_optional_string(&payload, "quote_artifact_id"),
                observed_at: payload_optional_string(&payload, "observed_at"),
                updated_sequence: 0,
            }))
        }
        "evidence" => Ok(ProjectionRecord::Evidence(EvidenceProjection {
            evidence_id: EvidenceId::new(record_id),
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "evidence",
                a,
                "project_id",
            )?),
            task_id: optional_id(b),
            session_id: optional_id(c),
            run_id: optional_id(d),
            kind: required_field(&projection_kind, "evidence", e, "kind")?,
            artifact_id: f,
            confidence: required_i64(&projection_kind, "evidence", g, "confidence")?,
            updated_sequence: 0,
        })),
        "task_outcome_report" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::TaskOutcomeReport(
                TaskOutcomeReportProjection {
                    task_outcome_report_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        a,
                        "project_id",
                    )?),
                    task_id: TaskId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        b,
                        "task_id",
                    )?),
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        c,
                        "session_id",
                    )?),
                    run_id: RunId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        d,
                        "run_id",
                    )?),
                    outcome_status: required_field(
                        &projection_kind,
                        "task_outcome_report",
                        e,
                        "outcome_status",
                    )?,
                    started_sequence: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        f,
                        "started_sequence",
                    )?,
                    completed_sequence: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        g,
                        "completed_sequence",
                    )?,
                    duration_sequence_span: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        h,
                        "duration_sequence_span",
                    )?,
                    action_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "action_count",
                    )?,
                    tool_call_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "tool_call_count",
                    )?,
                    evidence_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "evidence_count",
                    )?,
                    memory_packet_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "memory_packet_count",
                    )?,
                    confidence: payload_optional_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "confidence",
                    )?,
                    blocker: payload_optional_string(&payload, "blocker"),
                    review_outcome: required_payload_string(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "review_outcome",
                    )?,
                    report_artifact_id: payload_optional_string(&payload, "report_artifact_id"),
                    updated_sequence: 0,
                },
            ))
        }
        "review_finding" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: record_id,
                project_id: ProjectId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    a,
                    "project_id",
                )?),
                task_id: TaskId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    b,
                    "task_id",
                )?),
                session_id: SessionId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    c,
                    "session_id",
                )?),
                run_id: optional_id(d),
                tool_call_id: optional_id(e),
                workpad_task_id: f,
                finding_kind: required_field(
                    &projection_kind,
                    "review_finding",
                    g,
                    "finding_kind",
                )?,
                status: required_field(&projection_kind, "review_finding", h, "status")?,
                reviewer: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "reviewer",
                )?,
                severity: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "severity",
                )?,
                summary: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "summary",
                )?,
                evidence_artifact_id: payload_optional_string(&payload, "evidence_artifact_id"),
                follow_up: payload_optional_string(&payload, "follow_up"),
                updated_sequence: 0,
            }))
        }
        "workpad_index_reset" => Ok(ProjectionRecord::WorkpadIndexReset(
            WorkpadIndexResetProjection {
                project_id: ProjectId::new(record_id),
                observed_unix: required_i64(
                    &projection_kind,
                    "workpad_index_reset",
                    a,
                    "observed_unix",
                )?,
                updated_sequence: 0,
            },
        )),
        "workpad_file" => Ok(ProjectionRecord::WorkpadFile(WorkpadFileProjection {
            path: record_id,
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "workpad_file",
                a,
                "project_id",
            )?),
            content_hash: required_field(&projection_kind, "workpad_file", b, "content_hash")?,
            headings: required_field(&projection_kind, "workpad_file", c, "headings")?,
            objective: d,
            observed_unix: required_i64(&projection_kind, "workpad_file", e, "observed_unix")?,
            updated_sequence: 0,
        })),
        "workpad_task" => Ok(ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
            workpad_task_id: record_id,
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "workpad_task",
                a,
                "project_id",
            )?),
            path: required_field(&projection_kind, "workpad_task", b, "path")?,
            source_anchor: required_field(&projection_kind, "workpad_task", c, "source_anchor")?,
            title: required_field(&projection_kind, "workpad_task", d, "title")?,
            observed_status: required_field(
                &projection_kind,
                "workpad_task",
                e,
                "observed_status",
            )?,
            capo_execution_status: required_field(
                &projection_kind,
                "workpad_task",
                f,
                "capo_execution_status",
            )?,
            observed_unix: required_i64(&projection_kind, "workpad_task", g, "observed_unix")?,
            updated_sequence: 0,
        })),
        other => Err(ProjectionDecodeError(format!(
            "unknown projection kind: {other}"
        ))),
    }
}

fn required_field(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<String, ProjectionDecodeError> {
    value.ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {field}"))
    })
}

fn parse_projection_payload(
    projection_kind: &str,
    record_id: &str,
    payload_json: &str,
) -> Result<Value, ProjectionDecodeError> {
    serde_json::from_str(payload_json).map_err(|error| {
        ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid payload_json: {error}"
        ))
    })
}

fn payload_string(payload: &Value, key: &str) -> Option<String> {
    match payload.get(key)? {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        value => Some(value.to_string()),
    }
}

fn payload_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload_string(payload, key)
}

fn required_payload_string(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<String, ProjectionDecodeError> {
    payload_string(payload, key).ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {key}"))
    })
}

fn required_payload_i64(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<i64, ProjectionDecodeError> {
    payload_optional_i64(projection_kind, record_id, payload, key)?.ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {key}"))
    })
}

fn payload_optional_i64(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<Option<i64>, ProjectionDecodeError> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_i64().map(Some).ok_or_else(|| {
            ProjectionDecodeError(format!(
                "{projection_kind}.{record_id} invalid {key}: not an i64"
            ))
        }),
        Some(Value::String(value)) => value.parse::<i64>().map(Some).map_err(|error| {
            ProjectionDecodeError(format!(
                "{projection_kind}.{record_id} invalid {key}: {error}"
            ))
        }),
        Some(_) => Err(ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid {key}: not a number"
        ))),
    }
}

fn validate_projection_json(
    kind: &'static str,
    id: &str,
    field: &'static str,
    value: &str,
) -> StateResult<()> {
    serde_json::from_str::<Value>(value)
        .map(|_| ())
        .map_err(|error| StateError::InvalidProjectionJson {
            kind,
            id: id.to_string(),
            field,
            error: error.to_string(),
        })
}

fn optional_i64(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<Option<i64>, ProjectionDecodeError> {
    value
        .map(|value| {
            value.parse::<i64>().map_err(|error| {
                ProjectionDecodeError(format!(
                    "{projection_kind}.{record_id} invalid {field}: {error}"
                ))
            })
        })
        .transpose()
}

fn required_i64(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<i64, ProjectionDecodeError> {
    let value = required_field(projection_kind, record_id, value, field)?;
    value.parse::<i64>().map_err(|error| {
        ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid {field}: {error}"
        ))
    })
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
mod tests {
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
        let task = store.task(&task_id).unwrap().expect("task");
        assert_eq!(task.latest_summary.as_deref(), Some("state scaffold"));
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
}
