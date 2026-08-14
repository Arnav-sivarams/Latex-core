//! Structured workspace failures.

use blob_store::BlobStoreError;
use core_types::{LogicalPath, SnapshotId, WorkspaceVersion};
use persistence::PersistenceError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace mutation must contain at least one operation")]
    EmptyMutation,
    #[error("workspace file not found: {path}")]
    FileNotFound { path: LogicalPath },
    #[error("workspace file already exists: {path}")]
    FileAlreadyExists { path: LogicalPath },
    #[error("invalid workspace state: {message}")]
    InvalidWorkspaceState { message: String },
    #[error("workspace version conflict: expected {expected:?}, actual {actual:?}")]
    VersionConflict {
        expected: WorkspaceVersion,
        actual: WorkspaceVersion,
    },
    #[error("workspace version overflow")]
    VersionOverflow,
    #[error("unsupported workspace event type: {event_type}")]
    UnsupportedEventType { event_type: String },
    #[error("unsupported workspace event schema: {version}")]
    UnsupportedEventSchema { version: u32 },
    #[error("corrupt persistent workspace event: {message}")]
    CorruptPersistentEvent { message: String },
    #[error("snapshot identity mismatch: expected {expected}, actual {actual}")]
    SnapshotMismatch {
        expected: SnapshotId,
        actual: SnapshotId,
    },
    #[error("blob size mismatch: expected {expected}, actual {actual}")]
    BlobSizeMismatch { expected: u64, actual: u64 },
    #[error("blob storage failure")]
    Blob(#[from] BlobStoreError),
    #[error("persistence failure")]
    Persistence(#[from] PersistenceError),
    #[error("workspace serialization failure")]
    Serialization(#[from] serde_json::Error),
    #[error("workspace manifest failure: {message}")]
    Manifest { message: String },
}

impl WorkspaceError {
    pub(crate) fn map_persistence(error: PersistenceError) -> Self {
        match error {
            PersistenceError::VersionConflict { expected, actual } => {
                Self::VersionConflict { expected, actual }
            }
            other => Self::Persistence(other),
        }
    }
}
