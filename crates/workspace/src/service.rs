//! Durable blob-first workspace orchestration.

use crate::{WorkspaceError, WorkspaceMutationV1, WorkspaceOperationV1, WorkspaceState};
use blob_store::BlobStore;
use bytes::Bytes;
use core_types::{
    BlobHash, LogicalPath, SnapshotId, TenantId, UserId, WorkspaceId, WorkspaceManifestV1,
    WorkspaceVersion,
};
use persistence::{PostgresWorkspaceRepository, WorkspaceEventRecord};
use std::sync::Arc;

const EVENT_TYPE: &str = "workspace.mutation";
const EVENT_SCHEMA: u32 = 1;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCheckpoint {
    workspace_version: WorkspaceVersion,
    snapshot_id: SnapshotId,
    manifest_blob_hash: BlobHash,
}
impl WorkspaceCheckpoint {
    #[must_use]
    pub const fn workspace_version(&self) -> WorkspaceVersion {
        self.workspace_version
    }
    #[must_use]
    pub const fn snapshot_id(&self) -> SnapshotId {
        self.snapshot_id
    }
    #[must_use]
    pub const fn manifest_blob_hash(&self) -> BlobHash {
        self.manifest_blob_hash
    }
}

#[derive(Clone)]
pub struct WorkspaceService {
    repository: PostgresWorkspaceRepository,
    blobs: Arc<dyn BlobStore>,
}

impl std::fmt::Debug for WorkspaceService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceService")
            .field("repository", &self.repository)
            .finish_non_exhaustive()
    }
}

#[allow(clippy::missing_errors_doc)]
impl WorkspaceService {
    #[must_use]
    pub fn new(repository: PostgresWorkspaceRepository, blobs: Arc<dyn BlobStore>) -> Self {
        Self { repository, blobs }
    }

    pub async fn create_workspace(
        &self,
        tenant: TenantId,
        owner: UserId,
        workspace_id: WorkspaceId,
        main: LogicalPath,
        bytes: Bytes,
    ) -> Result<WorkspaceState, WorkspaceError> {
        let stored = self.blobs.put(bytes).await?;
        let mutation = WorkspaceMutationV1::new(vec![
            WorkspaceOperationV1::PutFile {
                path: main.clone(),
                blob_hash: stored.hash(),
                size_bytes: stored.size_bytes(),
            },
            WorkspaceOperationV1::SetMainFile { path: main },
        ])?;
        let mut state = WorkspaceState::empty(workspace_id);
        state.apply(&mutation, WorkspaceVersion::initial())?;
        let payload = serde_json::to_value(&mutation)?;
        self.repository
            .create_workspace_with_initial_event(
                tenant,
                owner,
                workspace_id,
                EVENT_TYPE,
                EVENT_SCHEMA,
                payload,
            )
            .await
            .map_err(WorkspaceError::map_persistence)?;
        Ok(state)
    }

    pub async fn put_file(
        &self,
        workspace_id: WorkspaceId,
        user: UserId,
        expected: WorkspaceVersion,
        path: LogicalPath,
        bytes: Bytes,
    ) -> Result<WorkspaceVersion, WorkspaceError> {
        let state = self.restore(workspace_id).await?;
        let stored = self.blobs.put(bytes).await?;
        if state.version() != expected {
            return Err(WorkspaceError::VersionConflict {
                expected,
                actual: state.version(),
            });
        }
        let mutation = WorkspaceMutationV1::new(vec![WorkspaceOperationV1::PutFile {
            path,
            blob_hash: stored.hash(),
            size_bytes: stored.size_bytes(),
        }])?;
        self.validate_and_append(state, user, expected, mutation)
            .await
    }

    pub async fn delete_file(
        &self,
        workspace_id: WorkspaceId,
        user: UserId,
        expected: WorkspaceVersion,
        path: LogicalPath,
    ) -> Result<WorkspaceVersion, WorkspaceError> {
        let state = self.restore_expected(workspace_id, expected).await?;
        let mutation = WorkspaceMutationV1::new(vec![WorkspaceOperationV1::DeleteFile { path }])?;
        self.validate_and_append(state, user, expected, mutation)
            .await
    }

    pub async fn rename_file(
        &self,
        workspace_id: WorkspaceId,
        user: UserId,
        expected: WorkspaceVersion,
        from: LogicalPath,
        to: LogicalPath,
    ) -> Result<WorkspaceVersion, WorkspaceError> {
        let state = self.restore_expected(workspace_id, expected).await?;
        let mutation =
            WorkspaceMutationV1::new(vec![WorkspaceOperationV1::RenameFile { from, to }])?;
        self.validate_and_append(state, user, expected, mutation)
            .await
    }

    pub async fn set_main_file(
        &self,
        workspace_id: WorkspaceId,
        user: UserId,
        expected: WorkspaceVersion,
        path: LogicalPath,
    ) -> Result<WorkspaceVersion, WorkspaceError> {
        let state = self.restore_expected(workspace_id, expected).await?;
        let mutation = WorkspaceMutationV1::new(vec![WorkspaceOperationV1::SetMainFile { path }])?;
        self.validate_and_append(state, user, expected, mutation)
            .await
    }

    pub async fn restore(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceState, WorkspaceError> {
        let head = self
            .repository
            .load_head(workspace_id)
            .await
            .map_err(WorkspaceError::map_persistence)?;
        let target = head.durable_version();
        let mut state = if let Some(snapshot) = head.latest_snapshot() {
            let bytes = self.blobs.get(snapshot.manifest_blob_hash()).await?;
            let actual_blob = BlobHash::digest(&bytes);
            if actual_blob != snapshot.manifest_blob_hash() {
                return Err(WorkspaceError::CorruptPersistentEvent {
                    message: "snapshot manifest blob hash mismatch".to_owned(),
                });
            }
            let manifest: WorkspaceManifestV1 = serde_json::from_slice(&bytes)?;
            let actual_snapshot =
                manifest
                    .snapshot_id()
                    .map_err(|error| WorkspaceError::Manifest {
                        message: error.to_string(),
                    })?;
            if actual_snapshot != snapshot.snapshot_id() {
                return Err(WorkspaceError::SnapshotMismatch {
                    expected: snapshot.snapshot_id(),
                    actual: actual_snapshot,
                });
            }
            WorkspaceState::from_manifest(workspace_id, snapshot.workspace_version(), &manifest)?
        } else {
            WorkspaceState::empty(workspace_id)
        };
        let events = self
            .repository
            .load_events(workspace_id, state.version(), target)
            .await
            .map_err(WorkspaceError::map_persistence)?;
        for event in &events {
            Self::replay_event(&mut state, event)?;
        }
        if state.version() != target {
            return Err(WorkspaceError::CorruptPersistentEvent {
                message: format!(
                    "replay ended at {:?}, expected {:?}",
                    state.version(),
                    target
                ),
            });
        }
        Ok(state)
    }

    pub async fn read_file(
        &self,
        workspace_id: WorkspaceId,
        path: &LogicalPath,
    ) -> Result<Bytes, WorkspaceError> {
        let state = self.restore(workspace_id).await?;
        let file = state
            .file(path)
            .ok_or_else(|| WorkspaceError::FileNotFound { path: path.clone() })?;
        let bytes = self.blobs.get(file.blob_hash()).await?;
        let actual =
            u64::try_from(bytes.len()).map_err(|_| WorkspaceError::InvalidWorkspaceState {
                message: "blob length exceeds u64".to_owned(),
            })?;
        if actual != file.size_bytes() {
            return Err(WorkspaceError::BlobSizeMismatch {
                expected: file.size_bytes(),
                actual,
            });
        }
        Ok(bytes)
    }

    pub async fn force_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        let state = self.restore(workspace_id).await?;
        let manifest = state.to_manifest()?;
        let canonical =
            manifest
                .canonical_json_bytes()
                .map_err(|error| WorkspaceError::Manifest {
                    message: error.to_string(),
                })?;
        let snapshot_id = manifest
            .snapshot_id()
            .map_err(|error| WorkspaceError::Manifest {
                message: error.to_string(),
            })?;
        let manifest_blob_hash = BlobHash::digest(&canonical);
        let stored = self
            .blobs
            .put_verified(manifest_blob_hash, Bytes::from(canonical))
            .await?;
        if stored.hash() != manifest_blob_hash {
            return Err(WorkspaceError::CorruptPersistentEvent {
                message: "blob store returned a different manifest hash".to_owned(),
            });
        }
        self.repository
            .record_snapshot(
                workspace_id,
                state.version(),
                snapshot_id,
                manifest_blob_hash,
            )
            .await
            .map_err(WorkspaceError::map_persistence)?;
        Ok(WorkspaceCheckpoint {
            workspace_version: state.version(),
            snapshot_id,
            manifest_blob_hash,
        })
    }

    pub fn replay_event(
        state: &mut WorkspaceState,
        event: &WorkspaceEventRecord,
    ) -> Result<(), WorkspaceError> {
        let next = state
            .version()
            .checked_next()
            .map_err(|_| WorkspaceError::VersionOverflow)?;
        if event.sequence() != next || event.base_version() != state.version() {
            return Err(WorkspaceError::CorruptPersistentEvent {
                message: "workspace event sequence or base version is discontinuous".to_owned(),
            });
        }
        if event.event_type() != EVENT_TYPE {
            return Err(WorkspaceError::UnsupportedEventType {
                event_type: event.event_type().to_owned(),
            });
        }
        if event.event_schema_version() != EVENT_SCHEMA {
            return Err(WorkspaceError::UnsupportedEventSchema {
                version: event.event_schema_version(),
            });
        }
        let payload_schema = event
            .payload()
            .get("schema_version")
            .and_then(serde_json::Value::as_u64);
        if payload_schema != Some(u64::from(EVENT_SCHEMA)) {
            let version = payload_schema
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0);
            return Err(WorkspaceError::UnsupportedEventSchema { version });
        }
        let mutation: WorkspaceMutationV1 = serde_json::from_value(event.payload().clone())
            .map_err(|error| WorkspaceError::CorruptPersistentEvent {
                message: error.to_string(),
            })?;
        state.apply(&mutation, event.base_version())
    }

    async fn restore_expected(
        &self,
        workspace_id: WorkspaceId,
        expected: WorkspaceVersion,
    ) -> Result<WorkspaceState, WorkspaceError> {
        let state = self.restore(workspace_id).await?;
        if state.version() != expected {
            return Err(WorkspaceError::VersionConflict {
                expected,
                actual: state.version(),
            });
        }
        Ok(state)
    }

    async fn validate_and_append(
        &self,
        mut state: WorkspaceState,
        user: UserId,
        expected: WorkspaceVersion,
        mutation: WorkspaceMutationV1,
    ) -> Result<WorkspaceVersion, WorkspaceError> {
        state.apply(&mutation, expected)?;
        let payload = serde_json::to_value(mutation)?;
        self.repository
            .append_event(
                state.workspace_id(),
                user,
                expected,
                EVENT_TYPE,
                EVENT_SCHEMA,
                payload,
            )
            .await
            .map_err(WorkspaceError::map_persistence)
    }
}
