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
