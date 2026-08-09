//! PostgreSQL-backed durable workspace event and snapshot records.

use crate::{Database, PersistenceError};
use core_types::{BlobHash, SnapshotId, TenantId, UserId, WorkspaceId, WorkspaceVersion};
use serde_json::Value;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSnapshotRecord {
    workspace_version: WorkspaceVersion,
    snapshot_id: SnapshotId,
    manifest_blob_hash: BlobHash,
}

impl WorkspaceSnapshotRecord {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceHeadRecord {
    workspace_id: WorkspaceId,
    durable_version: WorkspaceVersion,
    latest_snapshot: Option<WorkspaceSnapshotRecord>,
}

impl WorkspaceHeadRecord {
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    #[must_use]
    pub const fn durable_version(&self) -> WorkspaceVersion {
        self.durable_version
    }
    #[must_use]
    pub const fn latest_snapshot(&self) -> Option<&WorkspaceSnapshotRecord> {
        self.latest_snapshot.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceEventRecord {
    sequence: WorkspaceVersion,
    base_version: WorkspaceVersion,
    event_type: String,
    event_schema_version: u32,
    payload: Value,
}

impl WorkspaceEventRecord {
    #[must_use]
    pub fn new(
        sequence: WorkspaceVersion,
        base_version: WorkspaceVersion,
        event_type: String,
        event_schema_version: u32,
        payload: Value,
    ) -> Self {
        Self {
            sequence,
            base_version,
            event_type,
            event_schema_version,
            payload,
        }
    }
    #[must_use]
    pub const fn sequence(&self) -> WorkspaceVersion {
        self.sequence
    }
    #[must_use]
    pub const fn base_version(&self) -> WorkspaceVersion {
        self.base_version
    }
    #[must_use]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }
    #[must_use]
    pub const fn event_schema_version(&self) -> u32 {
        self.event_schema_version
    }
    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }
}

#[derive(Clone, Debug)]
pub struct PostgresWorkspaceRepository {
    database: Database,
}

#[allow(clippy::missing_errors_doc)]
impl PostgresWorkspaceRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    #[cfg(feature = "database-tests")]
    pub async fn create_test_identity(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> Result<(), PersistenceError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(PersistenceError::Database)?;
        sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
            .bind(tenant_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::Database)?;
        sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
            .bind(user_id.as_uuid())
            .bind(tenant_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::Database)?;
        tx.commit().await.map_err(PersistenceError::Database)
    }

    pub async fn create_workspace_with_initial_event(
        &self,
        tenant_id: TenantId,
        owner: UserId,
        workspace_id: WorkspaceId,
        event_type: &str,
        event_schema_version: u32,
        payload: Value,
    ) -> Result<WorkspaceVersion, PersistenceError> {
        let schema = i32::try_from(event_schema_version)
            .map_err(|_| integrity("event schema version exceeds PostgreSQL INTEGER"))?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(PersistenceError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)",
        )
        .bind(workspace_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .bind(owner.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.workspace_heads (workspace_id,durable_version) VALUES ($1,0)",
        )
        .bind(workspace_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::Database)?;
        sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,1,$2,0,$3,$4,$5,$6)")
            .bind(workspace_id.as_uuid()).bind(Uuid::new_v4()).bind(event_type).bind(schema).bind(payload).bind(owner.as_uuid()).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=1,updated_at=now() WHERE workspace_id=$1")
            .bind(workspace_id.as_uuid()).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        tx.commit().await.map_err(PersistenceError::Database)?;
        Ok(WorkspaceVersion::new(1))
    }

    pub async fn append_event(
        &self,
        workspace_id: WorkspaceId,
        created_by: UserId,
        expected: WorkspaceVersion,
        event_type: &str,
        event_schema_version: u32,
        payload: Value,
    ) -> Result<WorkspaceVersion, PersistenceError> {
        let expected_db = version_to_db(expected)?;
        let schema = i32::try_from(event_schema_version)
            .map_err(|_| integrity("event schema version exceeds PostgreSQL INTEGER"))?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(PersistenceError::Database)?;
        let actual_db: Option<i64> = sqlx::query_scalar("SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE")
            .bind(workspace_id.as_uuid()).fetch_optional(&mut *tx).await.map_err(PersistenceError::Database)?;
        let actual = db_to_version(actual_db.ok_or(PersistenceError::NotFound {
            entity: "workspace",
        })?)?;
        if actual != expected {
            return Err(PersistenceError::VersionConflict { expected, actual });
        }
        let next = expected
            .checked_next()
            .map_err(|_| integrity("workspace version overflow"))?;
        let next_db = version_to_db(next)?;
        sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(workspace_id.as_uuid()).bind(next_db).bind(Uuid::new_v4()).bind(expected_db).bind(event_type).bind(schema).bind(payload).bind(created_by.as_uuid()).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=now() WHERE workspace_id=$1")
            .bind(workspace_id.as_uuid()).bind(next_db).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        tx.commit().await.map_err(PersistenceError::Database)?;
        Ok(next)
    }

    pub async fn load_head(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceHeadRecord, PersistenceError> {
        let row = sqlx::query("SELECT h.durable_version,h.latest_snapshot_version,ws.snapshot_id,s.manifest_blob_hash FROM latex_core.workspace_heads h LEFT JOIN latex_core.workspace_snapshots ws ON ws.workspace_id=h.workspace_id AND ws.workspace_version=h.latest_snapshot_version LEFT JOIN latex_core.snapshots s ON s.snapshot_id=ws.snapshot_id WHERE h.workspace_id=$1")
            .bind(workspace_id.as_uuid()).fetch_optional(self.database.pool()).await.map_err(PersistenceError::Database)?
            .ok_or(PersistenceError::NotFound { entity: "workspace" })?;
        let durable_version = db_to_version(
            row.try_get("durable_version")
                .map_err(PersistenceError::Database)?,
        )?;
        let snapshot_version: Option<i64> = row
            .try_get("latest_snapshot_version")
            .map_err(PersistenceError::Database)?;
        let latest_snapshot = match snapshot_version {
            None => None,
            Some(version) => {
                let snapshot: Option<String> = row
                    .try_get("snapshot_id")
                    .map_err(PersistenceError::Database)?;
                let manifest: Option<String> = row
                    .try_get("manifest_blob_hash")
                    .map_err(PersistenceError::Database)?;
                let snapshot =
                    snapshot.ok_or_else(|| integrity("snapshot head has no snapshot mapping"))?;
                let manifest =
                    manifest.ok_or_else(|| integrity("snapshot mapping has no global snapshot"))?;
                Some(WorkspaceSnapshotRecord {
                    workspace_version: db_to_version(version)?,
                    snapshot_id: parse_snapshot(&snapshot)?,
                    manifest_blob_hash: parse_blob(&manifest)?,
                })
            }
        };
        Ok(WorkspaceHeadRecord {
            workspace_id,
            durable_version,
            latest_snapshot,
        })
    }

    pub async fn load_events(
        &self,
        workspace_id: WorkspaceId,
        after: WorkspaceVersion,
        through: WorkspaceVersion,
    ) -> Result<Vec<WorkspaceEventRecord>, PersistenceError> {
        if after > through {
            return Err(integrity("event range start exceeds end"));
        }
        let rows = sqlx::query("SELECT sequence,base_version,event_type,event_schema_version,payload FROM latex_core.workspace_events WHERE workspace_id=$1 AND sequence>$2 AND sequence<=$3 ORDER BY sequence ASC")
            .bind(workspace_id.as_uuid()).bind(version_to_db(after)?).bind(version_to_db(through)?).fetch_all(self.database.pool()).await.map_err(PersistenceError::Database)?;
        rows.into_iter()
            .map(|row| {
                let schema: i32 = row
                    .try_get("event_schema_version")
                    .map_err(PersistenceError::Database)?;
                Ok(WorkspaceEventRecord {
                    sequence: db_to_version(
                        row.try_get("sequence")
                            .map_err(PersistenceError::Database)?,
                    )?,
                    base_version: db_to_version(
                        row.try_get("base_version")
                            .map_err(PersistenceError::Database)?,
                    )?,
                    event_type: row
                        .try_get("event_type")
                        .map_err(PersistenceError::Database)?,
                    event_schema_version: u32::try_from(schema)
                        .map_err(|_| integrity("negative event schema version"))?,
                    payload: row.try_get("payload").map_err(PersistenceError::Database)?,
                })
            })
            .collect()
    }

    pub async fn record_snapshot(
        &self,
        workspace_id: WorkspaceId,
        version: WorkspaceVersion,
        snapshot_id: SnapshotId,
        manifest_hash: BlobHash,
    ) -> Result<(), PersistenceError> {
        let version_db = version_to_db(version)?;
        let snapshot_text = snapshot_id.to_hex();
        let manifest_text = manifest_hash.to_hex();
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(PersistenceError::Database)?;
        let durable_db: Option<i64> = sqlx::query_scalar("SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE").bind(workspace_id.as_uuid()).fetch_optional(&mut *tx).await.map_err(PersistenceError::Database)?;
        let durable = db_to_version(durable_db.ok_or(PersistenceError::NotFound {
            entity: "workspace",
        })?)?;
        if version > durable {
            return Err(integrity("snapshot version exceeds durable version"));
        }
        sqlx::query("INSERT INTO latex_core.snapshots (snapshot_id,manifest_blob_hash) VALUES ($1,$2) ON CONFLICT (snapshot_id) DO NOTHING")
            .bind(&snapshot_text).bind(&manifest_text).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        let stored_manifest: String = sqlx::query_scalar(
            "SELECT manifest_blob_hash FROM latex_core.snapshots WHERE snapshot_id=$1",
        )
        .bind(&snapshot_text)
        .fetch_one(&mut *tx)
        .await
        .map_err(PersistenceError::Database)?;
        if stored_manifest != manifest_text {
            return Err(integrity(
                "snapshot identity maps to a different manifest blob",
            ));
        }
        sqlx::query("INSERT INTO latex_core.workspace_snapshots (workspace_id,workspace_version,snapshot_id) VALUES ($1,$2,$3) ON CONFLICT (workspace_id,workspace_version) DO NOTHING")
            .bind(workspace_id.as_uuid()).bind(version_db).bind(&snapshot_text).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        let stored_snapshot: String = sqlx::query_scalar("SELECT snapshot_id FROM latex_core.workspace_snapshots WHERE workspace_id=$1 AND workspace_version=$2")
            .bind(workspace_id.as_uuid()).bind(version_db).fetch_one(&mut *tx).await.map_err(PersistenceError::Database)?;
        if stored_snapshot != snapshot_text {
            return Err(integrity("workspace version maps to a different snapshot"));
        }
        sqlx::query("UPDATE latex_core.workspace_heads SET latest_snapshot_version=$2,updated_at=now() WHERE workspace_id=$1 AND (latest_snapshot_version IS NULL OR latest_snapshot_version<$2)")
            .bind(workspace_id.as_uuid()).bind(version_db).execute(&mut *tx).await.map_err(PersistenceError::Database)?;
        tx.commit().await.map_err(PersistenceError::Database)
    }
}

fn version_to_db(version: WorkspaceVersion) -> Result<i64, PersistenceError> {
    i64::try_from(version.get())
        .map_err(|_| integrity("workspace version exceeds PostgreSQL BIGINT"))
}
fn db_to_version(value: i64) -> Result<WorkspaceVersion, PersistenceError> {
    u64::try_from(value)
        .map(WorkspaceVersion::new)
        .map_err(|_| integrity("negative persisted workspace version"))
}
fn parse_snapshot(value: &str) -> Result<SnapshotId, PersistenceError> {
    SnapshotId::from_str(value)
        .map_err(|error| integrity(&format!("invalid persisted snapshot digest: {error}")))
}
fn parse_blob(value: &str) -> Result<BlobHash, PersistenceError> {
    BlobHash::from_str(value)
        .map_err(|error| integrity(&format!("invalid persisted blob digest: {error}")))
}
fn integrity(message: &str) -> PersistenceError {
    PersistenceError::IntegrityViolation {
        message: message.to_owned(),
    }
}
