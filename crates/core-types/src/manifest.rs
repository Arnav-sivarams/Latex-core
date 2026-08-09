//! Immutable workspace manifest version 1.

use crate::{BlobHash, LogicalPath, SnapshotId, error::ManifestError};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct FileEntryV1 {
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
pub struct WorkspaceManifestV1 {
    schema_version: u32,
    main_file: LogicalPath,
    files: BTreeMap<LogicalPath, FileEntryV1>,
}

impl<'de> Deserialize<'de> for WorkspaceManifestV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            main_file: LogicalPath,
            files: BTreeMap<LogicalPath, FileEntryV1>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(de::Error::custom(ManifestError::UnsupportedSchema(
                wire.schema_version,
            )));
        }
        Self::new(wire.main_file, wire.files).map_err(de::Error::custom)
    }
}
impl WorkspaceManifestV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    /// Constructs a non-empty manifest whose main file is present.
    ///
    /// # Errors
    /// Returns [`ManifestError`] when the file set is empty or omits `main_file`.
    pub fn new(
        main_file: LogicalPath,
        files: BTreeMap<LogicalPath, FileEntryV1>,
    ) -> Result<Self, ManifestError> {
        if files.is_empty() {
            return Err(ManifestError::Empty);
        }
        if !files.contains_key(&main_file) {
            return Err(ManifestError::MissingMainFile);
        }
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            main_file,
            files,
        })
    }
    #[must_use]
    pub fn main_file(&self) -> &LogicalPath {
        &self.main_file
    }
    #[must_use]
    pub fn files(&self) -> &BTreeMap<LogicalPath, FileEntryV1> {
        &self.files
    }
    /// Produces compact, field-ordered JSON with lexically ordered file keys.
    ///
    /// # Errors
    /// Returns [`ManifestError`] if serialization fails.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        Ok(serde_json::to_vec(self)?)
    }
    /// Hashes the canonical JSON as the content-derived snapshot identity.
    ///
    /// # Errors
    /// Returns [`ManifestError`] if canonical serialization fails.
    pub fn snapshot_id(&self) -> Result<SnapshotId, ManifestError> {
        Ok(SnapshotId::hash_canonical(&self.canonical_json_bytes()?))
    }
}
