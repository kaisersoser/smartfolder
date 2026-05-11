use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, SmartfolderError>;

#[derive(Debug, Error)]
pub enum SmartfolderError {
    #[error("failed to resolve app-local data directory")]
    AppDataDirectoryUnavailable,

    #[error("scan root is not a directory: {path}")]
    ScanRootNotDirectory { path: PathBuf },

    #[error("scan was cancelled")]
    ScanCancelled,

    #[error("plan has no selected operations to apply")]
    NoSelectedOperations,

    #[error("transaction journal path cannot be resolved for transaction: {transaction_id}")]
    InvalidTransactionId { transaction_id: String },

    #[error("destination path is empty")]
    EmptyDestination,

    #[error("destination path must not contain a Windows prefix: {path}")]
    DestinationHasPrefix { path: PathBuf },

    #[error("destination path must stay inside the selected root: {path}")]
    DestinationEscapesRoot { path: PathBuf },

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML rule profile error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("invalid rule profile: {message}")]
    InvalidRuleProfile { message: String },
}

impl SmartfolderError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
