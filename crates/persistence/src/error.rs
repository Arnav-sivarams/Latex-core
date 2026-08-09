//! Structured persistence failures.

use thiserror::Error;

/// A persistence configuration, database, or migration failure.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("invalid database configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("database operation failed")]
    Database(#[source] sqlx::Error),
    #[error("database migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
}
