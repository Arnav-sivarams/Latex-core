//! Structured persistence boundaries.
#![forbid(unsafe_code)]

mod app;
#[allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    reason = "repository methods share the crate's typed error contract; decoding consumes SQLx rows"
)]
mod compile_queue;
mod config;
mod database;
mod error;
mod workspace;

pub use app::{
    AppArtifactRecord, AppError, AppJobRecord, AppProjectRecord, AppRepository, AppSessionRecord,
    AppTemplateFileRecord, AppTemplateRecord, AppUserRecord,
};
pub use compile_queue::{
    CompileCacheRecordV1, CompileJobRecordV1, CompletionOutcome, EnqueueCompileJobV1,
    InfrastructureOutcome, PersistedArtifactV1, PostgresCompileQueue, QueueError, QueueLimits,
};
pub use config::DatabaseConfig;
pub use database::Database;
pub use error::PersistenceError;
pub use workspace::{
    PostgresWorkspaceRepository, WorkspaceEventRecord, WorkspaceHeadRecord, WorkspaceSnapshotRecord,
};
