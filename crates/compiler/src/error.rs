use core_types::{BlobHash, ShellPolicy, SnapshotId};
use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompilerError {
    #[error("invalid compiler configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("snapshot mismatch: requested {requested}, manifest {actual}")]
    SnapshotMismatch {
        requested: SnapshotId,
        actual: SnapshotId,
    },
    #[error("missing source blob: {hash}")]
    MissingBlob { hash: BlobHash },
    #[error("source blob integrity mismatch: expected {expected}, actual {actual}")]
    BlobIntegrityMismatch {
        expected: BlobHash,
        actual: BlobHash,
    },
    #[error("materialized blob size mismatch: expected {expected}, actual {actual}")]
    Materialization { expected: u64, actual: u64 },
    #[error("Docker is unavailable: {message}")]
    DockerUnavailable { message: String },
    #[error("Docker command failed: {message}")]
    DockerCommandFailed { message: String },
    #[error("compiler image not found: {image}")]
    ImageNotFound { image: String },
    #[error("image reference is not immutable: {image}")]
    InvalidImageReference { image: String },
    #[error("compiler image environment mismatch: {message}")]
    EnvironmentMismatch { message: String },
    #[error("shell policy is unsupported in milestone 7: {policy}")]
    UnsupportedShellPolicy { policy: ShellPolicy },
    #[error("invalid output entry: {path:?}")]
    InvalidOutputEntry { path: PathBuf },
    #[error("artifact exceeds limit: {path:?}, {size} bytes")]
    ArtifactTooLarge { path: PathBuf, size: u64 },
    #[error("total artifacts exceed limit: {size} bytes")]
    TotalArtifactsTooLarge { size: u64 },
    #[error("compiler output contains more than {limit} entries")]
    OutputEntriesExceeded { limit: usize },
    #[error("compiler output contains more than {limit} artifact files")]
    ArtifactFilesExceeded { limit: usize },
    #[error("successful compilation produced no PDF artifact")]
    MissingPdfArtifact,
    #[error("container infrastructure failure: {message}")]
    ContainerInfrastructure { message: String },
    #[error("blocking compiler runtime task failed")]
    BlockingTaskFailed {
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("I/O operation `{operation}` failed")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("internal invariant failed: {message}")]
    InternalInvariant { message: String },
}
