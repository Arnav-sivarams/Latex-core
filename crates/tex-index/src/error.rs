use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TexIndexError {
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("missing executable: {0}")]
    MissingExecutable(String),
    #[error("command failed: {program} (status {status:?}): {stderr}")]
    CommandFailed {
        program: String,
        status: Option<i32>,
        stderr: String,
    },
    #[error("command timed out: {program}")]
    CommandTimedOut { program: String },
    #[error("invalid command output from {program}: {message}")]
    InvalidCommandOutput { program: String, message: String },
    #[error("required TeX Live {required}, found {actual}")]
    WrongTexLiveRelease { required: u16, actual: u16 },
    #[error("TEXMFROOT is missing or invalid: {0}")]
    MissingTexmfRoot(PathBuf),
    #[error("local TeX Live package database is missing: {0}")]
    MissingTlpdb(PathBuf),
    #[error("invalid TLPDB: {0}")]
    InvalidTlpdb(String),
    #[error("invalid package path: {0}")]
    InvalidPackagePath(String),
    #[error("package entry escapes TEXMFROOT: {0}")]
    StorageEntryEscape(String),
    #[error("runtime file is missing: {0}")]
    MissingRuntimeFile(String),
    #[error("runtime entry is not a regular file: {0}")]
    InvalidRuntimeFile(String),
    #[error("hashing failed for {path}: {source}")]
    HashFailure {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("unsupported schema version: {0}")]
    UnsupportedSchema(u32),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("internal invariant failed: {0}")]
    InternalInvariant(String),
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
