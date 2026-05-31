use crate::RedactionState;

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
    /// AI2: a Codex-bound chat turn failed (fail-closed gate, or spawn/parse
    /// failure). Carried as a typed error so the chat surface never fabricates a
    /// fake summary in place of a real Codex result.
    CodexLiveChat(String),
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
