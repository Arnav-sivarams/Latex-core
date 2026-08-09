//! Structured persistence boundaries.
#![forbid(unsafe_code)]

mod config;
mod database;
mod error;
mod workspace;

pub use config::DatabaseConfig;
pub use database::Database;
pub use error::PersistenceError;
pub use workspace::{
    PostgresWorkspaceRepository, WorkspaceEventRecord, WorkspaceHeadRecord, WorkspaceSnapshotRecord,
};
