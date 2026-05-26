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

mod apply;
mod codec;
mod codec_encode;
mod error;
mod event;
mod projections;
mod queries;
mod schema;

pub use error::{StateError, StateResult};
pub use event::{
    ArtifactRecord, EventKind, EventRecord, NewEvent, RecoveryAttempt, RedactionState,
};
pub use projections::*;

use apply::{apply_projection_record, update_watermark};
use codec::projection_record_from_row;
use codec_encode::projection_record_to_row;
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
