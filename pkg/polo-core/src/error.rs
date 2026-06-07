use crate::fact::{BranchName, FactId, Namespace, TxId};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("fact {0} not found")]
    FactNotFound(FactId),

    #[error("transaction {0} not found")]
    TxNotFound(TxId),

    #[error("namespace '{0}' not found")]
    NamespaceNotFound(Namespace),

    #[error("namespace '{0}' already exists")]
    NamespaceExists(Namespace),

    #[error("branch '{0}' not found in namespace '{1}'")]
    BranchNotFound(BranchName, Namespace),

    #[error("branch '{0}' already exists in namespace '{1}'")]
    BranchExists(BranchName, Namespace),

    #[error("cannot delete branch '{0}': it is the root branch of namespace '{1}'")]
    CannotDeleteRoot(BranchName, Namespace),

    #[error("merge conflict: {0}")]
    Conflict(String),

    #[error("schema validation failed for attr '{attr}': {reason}")]
    SchemaViolation { attr: String, reason: String },

    #[error("invalid valid-time range: valid_from must be before valid_to")]
    InvalidTimeRange,

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("clock error: {0}")]
    Clock(#[from] crate::clock::ClockError),

    #[error("query error: {0}")]
    Query(String),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(String),

    #[error("serialization: {0}")]
    Serde(String),

    #[error("task join: {0}")]
    Join(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }

    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Error::FactNotFound(_)
                | Error::TxNotFound(_)
                | Error::NamespaceNotFound(_)
                | Error::BranchNotFound(_, _)
        )
    }
}
