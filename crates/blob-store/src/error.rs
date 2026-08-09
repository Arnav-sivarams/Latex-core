//! Blob storage errors.

use core_types::BlobHash;
use std::{io, path::PathBuf};
use thiserror::Error;

/// A structured failure from blob storage.
#[derive(Debug, Error)]
pub enum BlobStoreError {
    /// Store configuration is invalid.
    #[error("invalid blob store configuration: {message}")]
    InvalidConfiguration { message: String },
    /// The requested blob does not exist.
    #[error("blob not found: {hash}")]
    NotFound { hash: BlobHash },
    /// Supplied bytes do not match the caller's expected identity.
    #[error("blob hash mismatch: expected {expected}, actual {actual}")]
    HashMismatch {
        expected: BlobHash,
        actual: BlobHash,
    },
    /// Published storage content does not match its immutable identity.
    #[error("corrupt blob: expected {expected}, actual {actual}")]
    CorruptBlob {
        expected: BlobHash,
        actual: BlobHash,
    },
    /// A final object path contains something other than a regular file.
    #[error("invalid storage entry at {path:?}: {kind}")]
    InvalidStorageEntry { path: PathBuf, kind: &'static str },
    /// A filesystem operation failed.
    #[error("blob filesystem operation `{operation}` failed for {path:?}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A blocking filesystem task could not complete.
    #[error("blocking blob filesystem task failed")]
    BlockingTaskFailed {
        #[source]
        source: tokio::task::JoinError,
    },
}
