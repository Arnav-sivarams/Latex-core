//! Deterministic compilation artifact manifests.

use crate::{ArtifactId, BlobHash, CompileKey, LogicalPath, error::ArtifactManifestError};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::collections::BTreeSet;

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    Pdf,
    Log,
    Synctex,
    Fls,
    Aux,
    Bcf,
    Toc,
    Other,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct ArtifactRefV1 {
    pub artifact_id: ArtifactId,
    pub kind: ArtifactKind,
    pub logical_name: LogicalPath,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
pub struct ArtifactManifestV1 {
    schema_version: u32,
    compile_key: CompileKey,
    artifacts: Vec<ArtifactRefV1>,
}
impl<'de> Deserialize<'de> for ArtifactManifestV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            compile_key: CompileKey,
            artifacts: Vec<ArtifactRefV1>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(de::Error::custom(ArtifactManifestError::UnsupportedSchema(
                wire.schema_version,
            )));
        }
        Self::new(wire.compile_key, wire.artifacts).map_err(de::Error::custom)
    }
}
impl ArtifactManifestV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    /// Sorts artifacts deterministically and validates pair uniqueness.
    ///
    /// # Errors
    /// Returns [`ArtifactManifestError`] for a duplicate `(kind, logical_name)` pair.
    pub fn new(
        compile_key: CompileKey,
        mut artifacts: Vec<ArtifactRefV1>,
    ) -> Result<Self, ArtifactManifestError> {
        artifacts.sort_by(|left, right| {
            (left.kind, &left.logical_name).cmp(&(right.kind, &right.logical_name))
        });
        let mut seen = BTreeSet::new();
        if artifacts
            .iter()
            .any(|artifact| !seen.insert((artifact.kind, artifact.logical_name.clone())))
        {
            return Err(ArtifactManifestError::DuplicateArtifact);
        }
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            compile_key,
            artifacts,
        })
    }
    #[must_use]
    pub const fn compile_key(&self) -> CompileKey {
        self.compile_key
    }
    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactRefV1] {
        &self.artifacts
    }
    /// Produces compact JSON using the validated deterministic artifact order.
    ///
    /// # Errors
    /// Returns [`ArtifactManifestError`] if serialization fails.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, ArtifactManifestError> {
        Ok(serde_json::to_vec(self)?)
    }
}
