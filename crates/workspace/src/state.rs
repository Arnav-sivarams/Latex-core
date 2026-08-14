//! Deterministic replayed workspace state.

use crate::{WorkspaceError, WorkspaceMutationV1, WorkspaceOperationV1};
use core_types::{
    BlobHash, FileEntryV1, LogicalPath, WorkspaceId, WorkspaceManifestV1, WorkspaceVersion,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceFile {
    blob_hash: BlobHash,
    size_bytes: u64,
}
impl WorkspaceFile {
    #[must_use]
    pub const fn blob_hash(&self) -> BlobHash {
        self.blob_hash
    }
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceState {
    workspace_id: WorkspaceId,
    version: WorkspaceVersion,
    main_file: Option<LogicalPath>,
    files: BTreeMap<LogicalPath, WorkspaceFile>,
}

#[allow(clippy::missing_errors_doc)]
impl WorkspaceState {
    #[must_use]
    pub fn empty(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            version: WorkspaceVersion::initial(),
            main_file: None,
            files: BTreeMap::new(),
        }
    }
    pub fn from_manifest(
        workspace_id: WorkspaceId,
        version: WorkspaceVersion,
        manifest: &WorkspaceManifestV1,
    ) -> Result<Self, WorkspaceError> {
        let main_file = manifest.main_file().clone();
        let files = manifest
            .files()
            .iter()
            .map(|(path, entry)| {
                (
                    path.clone(),
                    WorkspaceFile {
                        blob_hash: entry.blob_hash,
                        size_bytes: entry.size_bytes,
                    },
                )
            })
            .collect();
        let state = Self {
            workspace_id,
            version,
            main_file: Some(main_file),
            files,
        };
        state.validate()?;
        Ok(state)
    }
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    #[must_use]
    pub const fn version(&self) -> WorkspaceVersion {
        self.version
    }
    #[must_use]
    pub const fn main_file(&self) -> Option<&LogicalPath> {
        self.main_file.as_ref()
    }
    #[must_use]
    pub const fn files(&self) -> &BTreeMap<LogicalPath, WorkspaceFile> {
        &self.files
    }
    #[must_use]
    pub fn file(&self, path: &LogicalPath) -> Option<&WorkspaceFile> {
        self.files.get(path)
    }
    #[must_use]
    pub fn contains_file(&self, path: &LogicalPath) -> bool {
        self.files.contains_key(path)
    }
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn apply(
        &mut self,
        mutation: &WorkspaceMutationV1,
        expected: WorkspaceVersion,
    ) -> Result<(), WorkspaceError> {
        if self.version != expected {
            return Err(WorkspaceError::VersionConflict {
                expected,
                actual: self.version,
            });
        }
        let mut candidate = self.clone();
        for operation in mutation.operations() {
            candidate.apply_operation(operation)?;
        }
        candidate.validate()?;
        candidate.version = candidate
            .version
            .checked_next()
            .map_err(|_| WorkspaceError::VersionOverflow)?;
        *self = candidate;
        Ok(())
    }

    pub fn to_manifest(&self) -> Result<WorkspaceManifestV1, WorkspaceError> {
        self.validate()?;
        let main = self
            .main_file
            .clone()
            .ok_or_else(|| invalid("empty workspace cannot be snapshotted"))?;
        let files = self
            .files
            .iter()
            .map(|(path, file)| {
                (
                    path.clone(),
                    FileEntryV1 {
                        blob_hash: file.blob_hash,
                        size_bytes: file.size_bytes,
                    },
                )
            })
            .collect();
        WorkspaceManifestV1::new(main, files).map_err(|error| WorkspaceError::Manifest {
            message: error.to_string(),
        })
    }

    fn apply_operation(&mut self, operation: &WorkspaceOperationV1) -> Result<(), WorkspaceError> {
        match operation {
            WorkspaceOperationV1::PutFile {
                path,
                blob_hash,
                size_bytes,
            } => {
                self.files.insert(
                    path.clone(),
                    WorkspaceFile {
                        blob_hash: *blob_hash,
                        size_bytes: *size_bytes,
                    },
                );
            }
            WorkspaceOperationV1::DeleteFile { path } => {
                if self.files.remove(path).is_none() {
                    return Err(WorkspaceError::FileNotFound { path: path.clone() });
                }
                if self.main_file.as_ref() == Some(path) {
                    self.main_file = None;
                }
            }
            WorkspaceOperationV1::RenameFile { from, to } => {
                if from == to {
                    if !self.files.contains_key(from) {
                        return Err(WorkspaceError::FileNotFound { path: from.clone() });
                    }
                    return Ok(());
                }
                if self.files.contains_key(to) {
                    return Err(WorkspaceError::FileAlreadyExists { path: to.clone() });
                }
                let file = self
                    .files
                    .remove(from)
                    .ok_or_else(|| WorkspaceError::FileNotFound { path: from.clone() })?;
                self.files.insert(to.clone(), file);
                if self.main_file.as_ref() == Some(from) {
                    self.main_file = Some(to.clone());
                }
            }
            WorkspaceOperationV1::SetMainFile { path } => {
                if !self.files.contains_key(path) {
                    return Err(WorkspaceError::FileNotFound { path: path.clone() });
                }
                self.main_file = Some(path.clone());
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), WorkspaceError> {
        match (&self.main_file, self.files.is_empty()) {
            // Imported projects may intentionally have no main file when several
            // root documents are plausible. They remain editable but cannot be
            // snapshotted for compilation until the owner chooses one.
            (None, _) => Ok(()),
            (Some(main), false) if self.files.contains_key(main) => Ok(()),
            (Some(_), true) => Err(invalid("empty workspace has a main file")),
            (Some(_), false) => Err(invalid("main file is absent from workspace files")),
        }
    }
}

fn invalid(message: &str) -> WorkspaceError {
    WorkspaceError::InvalidWorkspaceState {
        message: message.to_owned(),
    }
}
