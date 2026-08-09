//! Shared domain types for the LaTeX core platform.
#![forbid(unsafe_code)]

pub mod artifact;
pub mod compile;
pub mod digest;
pub mod error;
pub mod ids;
pub mod logical_path;
pub mod manifest;

pub use artifact::{ArtifactKind, ArtifactManifestV1, ArtifactRefV1};
pub use compile::{
    CompileKeyMaterialV1, CompileRequestV1, CostClass, JobState, ShellPolicy, TexEngine,
};
pub use digest::{BlobHash, CompileKey, SnapshotId};
pub use error::{
    ArtifactManifestError, CompileDomainError, DigestParseError, IdentifierError, LogicalPathError,
    ManifestError, VersionOverflowError,
};
pub use ids::{
    ArtifactId, ExtensionId, IdempotencyKey, JobId, LatexmkProfileId, TenantId, TexEnvironmentId,
    UserId, WorkerId, WorkspaceId, WorkspaceVersion,
};
pub use logical_path::LogicalPath;
pub use manifest::{FileEntryV1, WorkspaceManifestV1};
