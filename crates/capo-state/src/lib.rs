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

/// Name of the first durable local state backend.
pub const PROTOTYPE_STATE_BACKEND: &str = "sqlite";

pub type StateResult<T> = Result<T, StateError>;

#[derive(Debug)]
pub enum StateError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    MissingRecoveryAttempt(String),
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
                        latest_summary, latest_confidence, updated_sequence
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
                        updated_sequence: row.get(9)?,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    ProjectRegistered,
    TaskDiscovered,
    AgentRegistered,
    SessionStarted,
    SessionSummaryUpdated,
    RunStarted,
    CapabilityGrantCreated,
    ToolCallRequested,
    MemoryPacketBuilt,
    EvidenceRecorded,
    RecoveryStarted,
    RecoveryCompleted,
}

impl EventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRegistered => "project.registered",
            Self::TaskDiscovered => "task.discovered",
            Self::AgentRegistered => "agent.registered",
            Self::SessionStarted => "session.started",
            Self::SessionSummaryUpdated => "session.summary_updated",
            Self::RunStarted => "run.started",
            Self::CapabilityGrantCreated => "capability.grant_created",
            Self::ToolCallRequested => "tool.call_requested",
            Self::MemoryPacketBuilt => "memory.packet_built",
            Self::EvidenceRecorded => "evidence.recorded",
            Self::RecoveryStarted => "recovery.started",
            Self::RecoveryCompleted => "recovery.completed",
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
    ToolCall(ToolCallProjection),
    MemoryPacketRef(MemoryPacketProjection),
    Evidence(EvidenceProjection),
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
        CREATE TABLE IF NOT EXISTS recovery_attempts (
            recovery_attempt_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            started_sequence INTEGER NOT NULL,
            completed_sequence INTEGER,
            notes TEXT NOT NULL
        );
        ",
    )?;
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
        "tool_calls",
        "memory_packet_refs",
        "evidence",
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
                latest_summary, latest_confidence, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(session_id) DO UPDATE SET
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                agent_id = excluded.agent_id,
                title = excluded.title,
                status = excluded.status,
                current_goal = excluded.current_goal,
                latest_summary = excluded.latest_summary,
                latest_confidence = excluded.latest_confidence,
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
        ProjectionRecord::CapabilityGrant(grant) => transaction.execute(
            "INSERT INTO capability_grants(
                capability_grant_id, capability_profile_id, scope_json, effect,
                subject_json, updated_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(capability_grant_id) DO UPDATE SET
                capability_profile_id = excluded.capability_profile_id,
                scope_json = excluded.scope_json,
                effect = excluded.effect,
                subject_json = excluded.subject_json,
                updated_sequence = excluded.updated_sequence",
            params![
                grant.capability_grant_id,
                grant.capability_profile_id,
                grant.scope_json,
                grant.effect,
                grant.subject_json,
                sequence,
            ],
        )?,
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
            e: None,
            f: None,
            g: None,
            h: None,
            payload_json: "{}".to_string(),
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
    _payload_json: String,
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
                updated_sequence: 0,
            },
        )),
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
}
