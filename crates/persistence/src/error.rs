//! Structured persistence failures.

use core_types::WorkspaceVersion;
use thiserror::Error;

/// A persistence configuration, database, or migration failure.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("invalid database configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("persistent record not found: {entity}")]
    NotFound { entity: &'static str },
    #[error("workspace version conflict: expected {expected:?}, actual {actual:?}")]
    VersionConflict {
        expected: WorkspaceVersion,
        actual: WorkspaceVersion,
    },
    #[error("persistent integrity violation: {message}")]
    IntegrityViolation { message: String },
    #[error("database operation failed")]
    Database(#[source] sqlx::Error),
    #[error("database migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
}
