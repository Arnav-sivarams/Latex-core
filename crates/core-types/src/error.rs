//! Structured domain errors.

use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdentifierError {
    #[error("identifier length must be between {min} and {max} bytes, got {actual}")]
    InvalidLength {
        min: usize,
        max: usize,
        actual: usize,
    },
    #[error("identifier must contain ASCII characters only")]
    NonAscii,
    #[error("identifier contains an invalid character")]
    InvalidCharacter,
    #[error("extension identifier must contain exactly one dot-separated publisher and name")]
    InvalidExtensionFormat,
    #[error("extension identifier sections must begin and end with a letter or digit")]
    InvalidExtensionBoundary,
    #[error("invalid UUID: {0}")]
    InvalidUuid(String),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DigestParseError {
    #[error("SHA-256 digest must contain exactly 64 hexadecimal characters, got {0}")]
    InvalidLength(usize),
    #[error("SHA-256 digest must use lowercase hexadecimal characters only")]
    InvalidHex,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LogicalPathError {
    #[error("logical path must not be empty")]
    Empty,
    #[error("logical path exceeds 1024 UTF-8 bytes")]
    TooLong,
    #[error("logical path must be relative and use no leading slash")]
    Absolute,
    #[error("logical path must not end with a slash")]
    TrailingSlash,
    #[error("logical path contains an empty segment")]
    EmptySegment,
    #[error("logical path contains a forbidden dot segment")]
    DotSegment,
    #[error("logical path contains a forbidden character")]
    ForbiddenCharacter,
    #[error("logical path segment exceeds 255 UTF-8 bytes")]
    SegmentTooLong,
}

#[derive(Copy, Clone, Debug, Eq, Error, PartialEq)]
#[error("workspace version cannot advance beyond u64::MAX")]
pub struct VersionOverflowError;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("workspace manifest must contain at least one file")]
    Empty,
    #[error("workspace manifest main file is absent from files")]
    MissingMainFile,
    #[error("workspace manifest schema version must be 1, got {0}")]
    UnsupportedSchema(u32),
    #[error("failed to serialize canonical workspace manifest: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CompileDomainError {
    #[error("unknown TeX engine: {0}")]
    UnknownEngine(String),
    #[error("compile domain schema version must be 1, got {0}")]
    UnsupportedSchema(u32),
    #[error("failed to serialize canonical compile material: {0}")]
    Serialization(String),
}

#[derive(Debug, Error)]
pub enum ArtifactManifestError {
    #[error("artifact manifest contains duplicate kind and logical-name pair")]
    DuplicateArtifact,
    #[error("artifact manifest schema version must be 1, got {0}")]
    UnsupportedSchema(u32),
    #[error("failed to serialize canonical artifact manifest: {0}")]
    Serialization(#[from] serde_json::Error),
}
