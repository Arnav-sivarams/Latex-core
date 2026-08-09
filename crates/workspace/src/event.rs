//! Versioned whole-file workspace mutations.

use crate::WorkspaceError;
use core_types::{BlobHash, LogicalPath};
use serde::{Deserialize, Deserializer, Serialize, de};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkspaceOperationV1 {
    PutFile {
        path: LogicalPath,
        blob_hash: BlobHash,
        size_bytes: u64,
    },
    DeleteFile {
        path: LogicalPath,
    },
    RenameFile {
        from: LogicalPath,
        to: LogicalPath,
    },
    SetMainFile {
        path: LogicalPath,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceMutationV1 {
    schema_version: u32,
    operations: Vec<WorkspaceOperationV1>,
}

impl<'de> Deserialize<'de> for WorkspaceMutationV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            operations: Vec<WorkspaceOperationV1>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.schema_version != Self::SCHEMA_VERSION {
            return Err(de::Error::custom(format!(
                "unsupported workspace mutation schema {}",
                wire.schema_version
            )));
        }
        Self::new(wire.operations).map_err(de::Error::custom)
    }
}

#[allow(clippy::missing_errors_doc)]
impl WorkspaceMutationV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    pub fn new(operations: Vec<WorkspaceOperationV1>) -> Result<Self, WorkspaceError> {
        if operations.is_empty() {
            return Err(WorkspaceError::EmptyMutation);
        }
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            operations,
        })
    }
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    #[must_use]
    pub fn operations(&self) -> &[WorkspaceOperationV1] {
        &self.operations
    }
}
