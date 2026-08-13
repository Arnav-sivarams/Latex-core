//! Manual compilation domain contracts and deterministic cache keys.

use crate::{
    CompileKey, IdempotencyKey, LatexmkProfileId, SnapshotId, TexEnvironmentId, WorkspaceId,
    error::CompileDomainError,
};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, str::FromStr};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl fmt::Display for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { let value = match self { $(Self::$variant => $value),+ }; f.write_str(value) } }
    };
}
string_enum!(TexEngine { Latex => "latex", PdfLatex => "pdflatex", LuaLatex => "lualatex", XeLatex => "xelatex" });
impl FromStr for TexEngine {
    type Err = CompileDomainError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "latex" => Ok(Self::Latex),
            "pdflatex" => Ok(Self::PdfLatex),
            "lualatex" => Ok(Self::LuaLatex),
            "xelatex" => Ok(Self::XeLatex),
            _ => Err(CompileDomainError::UnknownEngine(value.to_owned())),
        }
    }
}
string_enum!(ShellPolicy { Safe => "safe", Restricted => "restricted", Compatibility => "compatibility" });
string_enum!(CostClass { Small => "small", Normal => "normal", Heavy => "heavy" });
string_enum!(JobState { Queued => "queued", Claimed => "claimed", Running => "running", Succeeded => "succeeded", Failed => "failed", Cancelled => "cancelled" });

#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
pub struct CompileRequestV1 {
    schema_version: u32,
    workspace_id: WorkspaceId,
    snapshot_id: SnapshotId,
    engine: TexEngine,
    shell_policy: ShellPolicy,
    synctex: bool,
    idempotency_key: IdempotencyKey,
}

impl<'de> Deserialize<'de> for CompileRequestV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            workspace_id: WorkspaceId,
            snapshot_id: SnapshotId,
            engine: TexEngine,
            shell_policy: ShellPolicy,
            synctex: bool,
            idempotency_key: IdempotencyKey,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(de::Error::custom(CompileDomainError::UnsupportedSchema(
                wire.schema_version,
            )));
        }

        Ok(Self::new(
            wire.workspace_id,
            wire.snapshot_id,
            wire.engine,
            wire.shell_policy,
            wire.synctex,
            wire.idempotency_key,
        ))
    }
}

impl CompileRequestV1 {
    pub const SCHEMA_VERSION: u32 = 1;

    #[must_use]
    pub fn new(
        workspace_id: WorkspaceId,
        snapshot_id: SnapshotId,
        engine: TexEngine,
        shell_policy: ShellPolicy,
        synctex: bool,
        idempotency_key: IdempotencyKey,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            workspace_id,
            snapshot_id,
            engine,
            shell_policy,
            synctex,
            idempotency_key,
        }
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    #[must_use]
    pub const fn snapshot_id(&self) -> SnapshotId {
        self.snapshot_id
    }

    #[must_use]
    pub const fn engine(&self) -> TexEngine {
        self.engine
    }

    #[must_use]
    pub const fn shell_policy(&self) -> ShellPolicy {
        self.shell_policy
    }

    #[must_use]
    pub const fn synctex(&self) -> bool {
        self.synctex
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
pub struct CompileKeyMaterialV1 {
    schema_version: u32,
    snapshot_id: SnapshotId,
    engine: TexEngine,
    tex_environment_id: TexEnvironmentId,
    latexmk_profile: LatexmkProfileId,
    shell_policy: ShellPolicy,
    synctex: bool,
}

impl<'de> Deserialize<'de> for CompileKeyMaterialV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            snapshot_id: SnapshotId,
            engine: TexEngine,
            tex_environment_id: TexEnvironmentId,
            latexmk_profile: LatexmkProfileId,
            shell_policy: ShellPolicy,
            synctex: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(de::Error::custom(CompileDomainError::UnsupportedSchema(
                wire.schema_version,
            )));
        }
        Ok(Self::new(
            wire.snapshot_id,
            wire.engine,
            wire.tex_environment_id,
            wire.latexmk_profile,
            wire.shell_policy,
            wire.synctex,
        ))
    }
}
impl CompileKeyMaterialV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    #[must_use]
    pub fn new(
        snapshot_id: SnapshotId,
        engine: TexEngine,
        tex_environment_id: TexEnvironmentId,
        latexmk_profile: LatexmkProfileId,
        shell_policy: ShellPolicy,
        synctex: bool,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            snapshot_id,
            engine,
            tex_environment_id,
            latexmk_profile,
            shell_policy,
            synctex,
        }
    }
    /// Produces compact, fixed-field-order compile-key material.
    ///
    /// # Errors
    /// Returns [`CompileDomainError`] if serialization fails.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, CompileDomainError> {
        serde_json::to_vec(self)
            .map_err(|error| CompileDomainError::Serialization(error.to_string()))
    }
    /// Hashes canonical material into the user-independent compilation cache key.
    ///
    /// # Errors
    /// Returns [`CompileDomainError`] if canonical serialization fails.
    pub fn compile_key(&self) -> Result<CompileKey, CompileDomainError> {
        Ok(CompileKey::hash_canonical(&self.canonical_json_bytes()?))
    }
}
