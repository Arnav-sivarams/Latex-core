//! S4 immutable paper versions and exact-state scheduling metadata.

use crate::{V2Error, V2Repository};
use core_types::{
    BlobHash, CompileKey, LatexmkProfileId, ShellPolicy, SnapshotId, TenantId, TexEngine,
    TexEnvironmentId, UserId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct V2BuildRequest {
    pub paper_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub document_epoch: u64,
    pub source_sequence: u64,
    pub snapshot_id: SnapshotId,
    pub manifest: Value,
    pub state_hash: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub trigger_type: String,
    pub compile_key: CompileKey,
    pub engine: TexEngine,
    pub tex_environment_id: TexEnvironmentId,
    pub latexmk_profile: LatexmkProfileId,
    pub shell_policy: ShellPolicy,
    pub synctex: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct V2PaperVersion {
    pub id: Uuid,
    pub paper_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub document_epoch: u64,
    pub version_number: u64,
    pub version_type: String,
    pub name: Option<String>,
    pub created_by_user_id: UserId,
    pub author_email: String,
    pub workspace_version: u64,
    pub snapshot_id: String,
    pub manifest: Value,
    pub state_hash: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct V2BuildSubmission {
    pub build_id: Option<Uuid>,
    pub active_build_id: Option<Uuid>,
    pub state_hash: String,
    pub status: String,
    pub reused: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct V2BuildView {
    pub desired_state_hash: Option<String>,
    pub source_sequence: Option<u64>,
    pub active_build_id: Option<Uuid>,
    pub active_status: Option<String>,
    pub current_build_id: Option<Uuid>,
    pub current_source_sequence: Option<u64>,
    pub current_job_id: Option<Uuid>,
    pub latest_status: Option<String>,
    pub latest_error: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct V2ArtifactRecord {
    pub artifact_id: Uuid,
    pub job_id: Uuid,
    pub logical_name: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub content_type: String,
}

impl V2Repository {
    pub async fn paper_document_epoch(&self, workspace_id: WorkspaceId) -> Result<u64, V2Error> {
        let value: i64 = sqlx::query_scalar(
            "INSERT INTO latex_core.paper_collaboration_state (workspace_id) VALUES ($1) \
             ON CONFLICT (workspace_id) DO UPDATE SET workspace_id=EXCLUDED.workspace_id \
             RETURNING document_epoch",
        )
        .bind(workspace_id.as_uuid())
        .fetch_one(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        u64::try_from(value).map_err(|_| V2Error::Integrity {
            message: "negative collaboration document epoch".to_owned(),
        })
    }

    pub async fn collaboration_cutoffs(
        &self,
        workspace_id: WorkspaceId,
        document_epoch: u64,
    ) -> Result<Vec<(Uuid, u64)>, V2Error> {
        let epoch = i64::try_from(document_epoch).map_err(|_| V2Error::Integrity {
            message: "collaboration epoch exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        let rows = sqlx::query(
            "SELECT f.file_id,COALESCE(max(u.id),0) AS cutoff FROM latex_core.paper_files f \
             LEFT JOIN latex_core.collaboration_updates u ON u.workspace_id=f.workspace_id \
               AND u.file_id=f.file_id AND u.document_epoch=$2 \
             WHERE f.workspace_id=$1 AND NOT f.tombstoned GROUP BY f.file_id ORDER BY f.file_id",
        )
        .bind(workspace_id.as_uuid())
        .bind(epoch)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter()
            .map(|row| {
                let cutoff: i64 = row.try_get("cutoff").map_err(V2Error::Database)?;
                Ok((
                    row.try_get("file_id").map_err(V2Error::Database)?,
                    u64::try_from(cutoff).map_err(|_| V2Error::Integrity {
                        message: "negative collaboration cutoff".to_owned(),
                    })?,
                ))
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_manual_version(
        &self,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        document_epoch: u64,
        workspace_version: u64,
        snapshot_id: SnapshotId,
        manifest: Value,
        state_hash: &str,
        actor: UserId,
        name: &str,
    ) -> Result<V2PaperVersion, V2Error> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err(V2Error::InvalidName);
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_writer_access(&mut tx, actor, paper_id, workspace_id, true).await?;
        lock_scheduler(&mut tx, paper_id, workspace_id).await?;
        let number = next_version_number(&mut tx, workspace_id).await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO latex_core.paper_versions \
             (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) \
             VALUES ($1,$2,$3,$4,$5,'manual_checkpoint',$6,$7,$8,$9,$10,$11)",
        )
        .bind(id)
        .bind(paper_id)
        .bind(workspace_id.as_uuid())
        .bind(to_i64(document_epoch, "document epoch")?)
        .bind(to_i64(number, "version number")?)
        .bind(name)
        .bind(actor.as_uuid())
        .bind(to_i64(workspace_version, "workspace version")?)
        .bind(snapshot_id.to_hex())
        .bind(&manifest)
        .bind(state_hash)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        self.paper_version(actor, paper_id, id).await
    }

    pub async fn paper_versions(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<Vec<V2PaperVersion>, V2Error> {
        let workspace_id = participant_workspace(self.database.pool(), actor, paper_id).await?;
        let rows = sqlx::query(
            "SELECT v.*,v.created_at::text AS created_at_text,c.email AS author_email FROM latex_core.paper_versions v \
             JOIN latex_core.user_credentials c ON c.user_id=v.created_by_user_id \
             WHERE v.workspace_id=$1 ORDER BY v.version_number DESC",
        )
        .bind(workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_version).collect()
    }

    pub async fn paper_version(
        &self,
        actor: UserId,
        paper_id: Uuid,
        version_id: Uuid,
    ) -> Result<V2PaperVersion, V2Error> {
        let workspace_id = participant_workspace(self.database.pool(), actor, paper_id).await?;
        let row = sqlx::query(
            "SELECT v.*,v.created_at::text AS created_at_text,c.email AS author_email FROM latex_core.paper_versions v \
             JOIN latex_core.user_credentials c ON c.user_id=v.created_by_user_id \
             WHERE v.workspace_id=$1 AND v.id=$2",
        )
        .bind(workspace_id.as_uuid())
        .bind(version_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "paper version" })?;
        decode_version(row)
    }

    pub async fn submit_v2_build(
        &self,
        request: &V2BuildRequest,
    ) -> Result<V2BuildSubmission, V2Error> {
        validate_build_request(request)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_build_access(
            &mut tx,
            request.user_id,
            request.paper_id,
            request.workspace_id,
            true,
        )
        .await?;
        lock_scheduler(&mut tx, request.paper_id, request.workspace_id).await?;
        sqlx::query(
            "UPDATE latex_core.v2_paper_build_state SET desired_state_hash=$2,desired_source_sequence=$3,updated_at=statement_timestamp() WHERE workspace_id=$1",
        )
        .bind(request.workspace_id.as_uuid())
        .bind(&request.state_hash)
        .bind(to_i64(request.source_sequence, "source sequence")?)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;

        if let Some(row) = sqlx::query(
            "SELECT b.id FROM latex_core.v2_paper_builds b \
             WHERE b.workspace_id=$1 AND b.state_hash=$2 AND b.status='succeeded' \
               AND EXISTS (SELECT 1 FROM latex_core.compilation_artifacts a WHERE a.job_id=b.compile_job_id AND a.kind='pdf') \
               AND EXISTS (SELECT 1 FROM latex_core.compilation_artifacts a WHERE a.job_id=b.compile_job_id AND a.kind='log') \
               AND EXISTS (SELECT 1 FROM latex_core.compilation_artifacts a WHERE a.job_id=b.compile_job_id AND a.kind='synctex' AND a.size_bytes>0) \
             ORDER BY b.created_at DESC LIMIT 1",
        )
        .bind(request.workspace_id.as_uuid())
        .bind(&request.state_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        {
            let build_id: Uuid = row.try_get("id").map_err(V2Error::Database)?;
            clear_pending_and_promote(&mut tx, request.workspace_id, build_id).await?;
            tx.commit().await.map_err(V2Error::Database)?;
            return Ok(V2BuildSubmission {
                build_id: Some(build_id),
                active_build_id: None,
                state_hash: request.state_hash.clone(),
                status: "succeeded".to_owned(),
                reused: true,
            });
        }

        let active = sqlx::query(
            "SELECT b.id,b.state_hash,j.state FROM latex_core.v2_paper_build_state s \
             JOIN latex_core.v2_paper_builds b ON b.id=s.active_build_id \
             JOIN latex_core.compile_jobs j ON j.id=b.compile_job_id WHERE s.workspace_id=$1",
        )
        .bind(request.workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if let Some(active) = active {
            let active_id: Uuid = active.try_get("id").map_err(V2Error::Database)?;
            let active_hash: String = active.try_get("state_hash").map_err(V2Error::Database)?;
            let active_status: String = active.try_get("state").map_err(V2Error::Database)?;
            if active_hash == request.state_hash {
                clear_pending(&mut tx, request.workspace_id).await?;
                tx.commit().await.map_err(V2Error::Database)?;
                return Ok(V2BuildSubmission {
                    build_id: Some(active_id),
                    active_build_id: Some(active_id),
                    state_hash: request.state_hash.clone(),
                    status: active_status,
                    reused: true,
                });
            }
            set_pending(&mut tx, request).await?;
            tx.commit().await.map_err(V2Error::Database)?;
            return Ok(V2BuildSubmission {
                build_id: None,
                active_build_id: Some(active_id),
                state_hash: request.state_hash.clone(),
                status: "pending".to_owned(),
                reused: false,
            });
        }

        let build_id = enqueue_build(&mut tx, request).await?;
        sqlx::query(
            "UPDATE latex_core.v2_paper_build_state SET active_build_id=$2,updated_at=statement_timestamp() WHERE workspace_id=$1",
        )
        .bind(request.workspace_id.as_uuid())
        .bind(build_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(V2BuildSubmission {
            build_id: Some(build_id),
            active_build_id: Some(build_id),
            state_hash: request.state_hash.clone(),
            status: "queued".to_owned(),
            reused: false,
        })
    }

    pub async fn v2_build_view(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<V2BuildView, V2Error> {
        let workspace_id = participant_workspace(self.database.pool(), actor, paper_id).await?;
        let row = sqlx::query(
            "SELECT s.desired_state_hash,s.desired_source_sequence,s.active_build_id,aj.state AS active_status, \
                    s.current_build_id,cb.source_sequence AS current_source_sequence,cb.compile_job_id AS current_job_id, \
                    lb.status AS latest_status,lj.last_error AS latest_error \
             FROM latex_core.v2_paper_build_state s \
             LEFT JOIN latex_core.v2_paper_builds ab ON ab.id=s.active_build_id \
             LEFT JOIN latex_core.compile_jobs aj ON aj.id=ab.compile_job_id \
             LEFT JOIN latex_core.v2_paper_builds cb ON cb.id=s.current_build_id \
             LEFT JOIN LATERAL (SELECT b.id,b.status,b.compile_job_id FROM latex_core.v2_paper_builds b WHERE b.workspace_id=s.workspace_id ORDER BY b.created_at DESC LIMIT 1) lb ON TRUE \
             LEFT JOIN latex_core.compile_jobs lj ON lj.id=lb.compile_job_id \
             WHERE s.workspace_id=$1",
        )
        .bind(workspace_id.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        let Some(row) = row else {
            return Ok(V2BuildView {
                desired_state_hash: None,
                source_sequence: None,
                active_build_id: None,
                active_status: None,
                current_build_id: None,
                current_source_sequence: None,
                current_job_id: None,
                latest_status: None,
                latest_error: None,
            });
        };
        Ok(V2BuildView {
            desired_state_hash: row
                .try_get("desired_state_hash")
                .map_err(V2Error::Database)?,
            source_sequence: optional_u64(&row, "desired_source_sequence")?,
            active_build_id: row.try_get("active_build_id").map_err(V2Error::Database)?,
            active_status: row.try_get("active_status").map_err(V2Error::Database)?,
            current_build_id: row.try_get("current_build_id").map_err(V2Error::Database)?,
            current_source_sequence: optional_u64(&row, "current_source_sequence")?,
            current_job_id: row.try_get("current_job_id").map_err(V2Error::Database)?,
            latest_status: row.try_get("latest_status").map_err(V2Error::Database)?,
            latest_error: row.try_get("latest_error").map_err(V2Error::Database)?,
        })
    }

    pub async fn current_v2_artifact(
        &self,
        actor: UserId,
        paper_id: Uuid,
        kind: &str,
    ) -> Result<V2ArtifactRecord, V2Error> {
        if !matches!(kind, "pdf" | "log" | "synctex") {
            return Err(V2Error::NotFound { entity: "artifact" });
        }
        let workspace_id = participant_workspace(self.database.pool(), actor, paper_id).await?;
        let row = sqlx::query(
            "SELECT a.artifact_id,a.job_id,a.logical_name,a.blob_hash,a.size_bytes,a.content_type \
             FROM latex_core.v2_paper_build_state s \
             JOIN latex_core.v2_paper_builds b ON b.id=s.current_build_id \
             JOIN latex_core.compilation_artifacts a ON a.job_id=b.compile_job_id \
             WHERE s.workspace_id=$1 AND a.kind=$2 ORDER BY a.logical_name LIMIT 1",
        )
        .bind(workspace_id.as_uuid())
        .bind(kind)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "artifact" })?;
        decode_artifact(row)
    }

    pub async fn v2_artifact_for_build(
        &self,
        actor: UserId,
        paper_id: Uuid,
        build_id: Uuid,
        kind: &str,
    ) -> Result<V2ArtifactRecord, V2Error> {
        if !matches!(kind, "pdf" | "log" | "synctex") {
            return Err(V2Error::NotFound { entity: "artifact" });
        }
        let workspace_id = participant_workspace(self.database.pool(), actor, paper_id).await?;
        let row = sqlx::query(
            "SELECT a.artifact_id,a.job_id,a.logical_name,a.blob_hash,a.size_bytes,a.content_type \
             FROM latex_core.v2_paper_builds b \
             JOIN latex_core.compilation_artifacts a ON a.job_id=b.compile_job_id \
             WHERE b.workspace_id=$1 AND b.id=$2 AND b.status='succeeded' AND a.kind=$3 \
             ORDER BY a.logical_name LIMIT 1",
        )
        .bind(workspace_id.as_uuid())
        .bind(build_id)
        .bind(kind)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "artifact" })?;
        decode_artifact(row)
    }
}

async fn participant_workspace(
    pool: &sqlx::PgPool,
    actor: UserId,
    paper_id: Uuid,
) -> Result<WorkspaceId, V2Error> {
    let row = sqlx::query_scalar::<_, Uuid>(
        "SELECT p.workspace_id FROM latex_core.personal_papers p \
         JOIN latex_core.global_user_roles r ON r.user_id=$2 AND r.role='writer' \
         WHERE p.id=$1 AND p.owner_user_id=$2 \
         UNION ALL \
         SELECT t.workspace_id FROM latex_core.paper_teams t \
         JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id AND m.user_id=$2 \
         JOIN latex_core.global_user_roles r ON r.user_id=$2 AND r.role IN ('writer','mentor') WHERE t.id=$1",
    )
    .bind(paper_id)
    .bind(actor.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper" })?;
    Ok(WorkspaceId::from_uuid(row))
}

async fn require_writer_access(
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    paper_id: Uuid,
    workspace_id: WorkspaceId,
    active: bool,
) -> Result<(), V2Error> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT p.status FROM latex_core.personal_papers p \
         JOIN latex_core.global_user_roles r ON r.user_id=$3 AND r.role='writer' \
         WHERE p.id=$1 AND p.workspace_id=$2 AND p.owner_user_id=$3 \
         UNION ALL \
         SELECT t.status FROM latex_core.paper_teams t \
         JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id AND m.user_id=$3 \
         JOIN latex_core.global_user_roles r ON r.user_id=$3 AND r.role='writer' \
         WHERE t.id=$1 AND t.workspace_id=$2",
    )
    .bind(paper_id)
    .bind(workspace_id.as_uuid())
    .bind(actor.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper" })?;
    if active && status != "active" {
        return Err(V2Error::Conflict {
            entity: "read-only paper",
        });
    }
    Ok(())
}

async fn require_build_access(
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    paper_id: Uuid,
    workspace_id: WorkspaceId,
    active: bool,
) -> Result<(), V2Error> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT p.status FROM latex_core.personal_papers p \
         JOIN latex_core.global_user_roles r ON r.user_id=$3 AND r.role='writer' \
         WHERE p.id=$1 AND p.workspace_id=$2 AND p.owner_user_id=$3 \
         UNION ALL \
         SELECT t.status FROM latex_core.paper_teams t \
         JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id AND m.user_id=$3 \
         JOIN latex_core.global_user_roles r ON r.user_id=$3 AND r.role IN ('writer','mentor') \
         WHERE t.id=$1 AND t.workspace_id=$2",
    )
    .bind(paper_id)
    .bind(workspace_id.as_uuid())
    .bind(actor.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper" })?;
    if active && status != "active" {
        return Err(V2Error::Conflict {
            entity: "read-only paper",
        });
    }
    Ok(())
}

async fn lock_scheduler(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
    workspace_id: WorkspaceId,
) -> Result<(), V2Error> {
    sqlx::query(
        "INSERT INTO latex_core.v2_paper_build_state (workspace_id,paper_id) VALUES ($1,$2) \
         ON CONFLICT (workspace_id) DO NOTHING",
    )
    .bind(workspace_id.as_uuid())
    .bind(paper_id)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    let stored: Uuid = sqlx::query_scalar(
        "SELECT paper_id FROM latex_core.v2_paper_build_state WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    if stored != paper_id {
        return Err(V2Error::Integrity {
            message: "scheduler paper identity mismatch".to_owned(),
        });
    }
    Ok(())
}

async fn next_version_number(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<u64, V2Error> {
    let value: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    u64::try_from(value).map_err(|_| V2Error::Integrity {
        message: "invalid version number".to_owned(),
    })
}

async fn enqueue_build(
    tx: &mut Transaction<'_, Postgres>,
    request: &V2BuildRequest,
) -> Result<Uuid, V2Error> {
    let build_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let number = next_version_number(tx, request.workspace_id).await?;
    sqlx::query(
        "INSERT INTO latex_core.compile_jobs \
         (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,priority,state) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'normal',$13,'queued')",
    )
    .bind(build_id)
    .bind(request.tenant_id.as_uuid())
    .bind(request.user_id.as_uuid())
    .bind(request.workspace_id.as_uuid())
    .bind(request.snapshot_id.to_hex())
    .bind(request.compile_key.to_hex())
    .bind(format!("v2:{build_id}"))
    .bind(request.engine.to_string())
    .bind(request.tex_environment_id.as_str())
    .bind(request.latexmk_profile.as_str())
    .bind(request.shell_policy.to_string())
    .bind(request.synctex)
    .bind(if request.trigger_type == "manual" { 10_i16 } else { 0_i16 })
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.paper_versions \
         (id,paper_id,workspace_id,document_epoch,version_number,version_type,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) \
         VALUES ($1,$2,$3,$4,$5,'compile_checkpoint',$6,$7,$8,$9,$10)",
    )
    .bind(version_id)
    .bind(request.paper_id)
    .bind(request.workspace_id.as_uuid())
    .bind(to_i64(request.document_epoch, "document epoch")?)
    .bind(to_i64(number, "version number")?)
    .bind(request.user_id.as_uuid())
    .bind(to_i64(request.source_sequence, "source sequence")?)
    .bind(request.snapshot_id.to_hex())
    .bind(&request.manifest)
    .bind(&request.state_hash)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.v2_paper_builds \
         (id,paper_id,workspace_id,compile_job_id,version_id,document_epoch,source_sequence,state_hash,trigger_type,status) \
         VALUES ($1,$2,$3,$1,$4,$5,$6,$7,$8,'queued')",
    )
    .bind(build_id)
    .bind(request.paper_id)
    .bind(request.workspace_id.as_uuid())
    .bind(version_id)
    .bind(to_i64(request.document_epoch, "document epoch")?)
    .bind(to_i64(request.source_sequence, "source sequence")?)
    .bind(&request.state_hash)
    .bind(&request.trigger_type)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(build_id)
}

async fn set_pending(
    tx: &mut Transaction<'_, Postgres>,
    request: &V2BuildRequest,
) -> Result<(), V2Error> {
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET \
         pending_snapshot_id=$2,pending_manifest=$3,pending_state_hash=$4,pending_source_sequence=$5, \
         pending_document_epoch=$6,pending_tenant_id=$7,pending_user_id=$8,pending_trigger_type=$9, \
         pending_compile_key=$10,pending_engine=$11,pending_tex_environment_id=$12,pending_latexmk_profile=$13, \
         pending_shell_policy=$14,pending_synctex=$15,updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(request.workspace_id.as_uuid())
    .bind(request.snapshot_id.to_hex())
    .bind(&request.manifest)
    .bind(&request.state_hash)
    .bind(to_i64(request.source_sequence, "source sequence")?)
    .bind(to_i64(request.document_epoch, "document epoch")?)
    .bind(request.tenant_id.as_uuid())
    .bind(request.user_id.as_uuid())
    .bind(&request.trigger_type)
    .bind(request.compile_key.to_hex())
    .bind(request.engine.to_string())
    .bind(request.tex_environment_id.as_str())
    .bind(request.latexmk_profile.as_str())
    .bind(request.shell_policy.to_string())
    .bind(request.synctex)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(())
}

async fn clear_pending(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<(), V2Error> {
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET pending_snapshot_id=NULL,pending_manifest=NULL, \
         pending_state_hash=NULL,pending_source_sequence=NULL,pending_document_epoch=NULL,pending_tenant_id=NULL, \
         pending_user_id=NULL,pending_trigger_type=NULL,pending_compile_key=NULL,pending_engine=NULL, \
         pending_tex_environment_id=NULL,pending_latexmk_profile=NULL,pending_shell_policy=NULL,pending_synctex=NULL, \
         updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(())
}

async fn clear_pending_and_promote(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    build_id: Uuid,
) -> Result<(), V2Error> {
    clear_pending(tx, workspace_id).await?;
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET active_build_id=NULL,current_build_id=$2,updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .bind(build_id)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query("UPDATE latex_core.v2_paper_builds SET promoted_at=COALESCE(promoted_at,statement_timestamp()),updated_at=statement_timestamp() WHERE id=$1")
        .bind(build_id).execute(&mut **tx).await.map_err(V2Error::Database)?;
    Ok(())
}

fn validate_build_request(request: &V2BuildRequest) -> Result<(), V2Error> {
    if !matches!(request.trigger_type.as_str(), "auto" | "manual") {
        return Err(V2Error::Integrity {
            message: "invalid V2 build trigger".to_owned(),
        });
    }
    if request.state_hash != request.snapshot_id.to_hex() {
        return Err(V2Error::Integrity {
            message: "state hash must equal canonical snapshot identity".to_owned(),
        });
    }
    if !request.synctex {
        return Err(V2Error::Integrity {
            message: "V2 builds require SyncTeX".to_owned(),
        });
    }
    Ok(())
}

fn decode_version(row: PgRow) -> Result<V2PaperVersion, V2Error> {
    Ok(V2PaperVersion {
        id: row.try_get("id").map_err(V2Error::Database)?,
        paper_id: row.try_get("paper_id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        document_epoch: required_u64(&row, "document_epoch")?,
        version_number: required_u64(&row, "version_number")?,
        version_type: row.try_get("version_type").map_err(V2Error::Database)?,
        name: row.try_get("name").map_err(V2Error::Database)?,
        created_by_user_id: UserId::from_uuid(
            row.try_get("created_by_user_id")
                .map_err(V2Error::Database)?,
        ),
        author_email: row.try_get("author_email").map_err(V2Error::Database)?,
        workspace_version: required_u64(&row, "workspace_version")?,
        snapshot_id: row.try_get("snapshot_id").map_err(V2Error::Database)?,
        manifest: row.try_get("manifest").map_err(V2Error::Database)?,
        state_hash: row.try_get("state_hash").map_err(V2Error::Database)?,
        created_at: row.try_get("created_at_text").map_err(V2Error::Database)?,
    })
}

fn decode_artifact(row: PgRow) -> Result<V2ArtifactRecord, V2Error> {
    let size: i64 = row.try_get("size_bytes").map_err(V2Error::Database)?;
    let blob: String = row.try_get("blob_hash").map_err(V2Error::Database)?;
    Ok(V2ArtifactRecord {
        artifact_id: row.try_get("artifact_id").map_err(V2Error::Database)?,
        job_id: row.try_get("job_id").map_err(V2Error::Database)?,
        logical_name: row.try_get("logical_name").map_err(V2Error::Database)?,
        blob_hash: BlobHash::from_str(&blob).map_err(|error| V2Error::Integrity {
            message: error.to_string(),
        })?,
        size_bytes: u64::try_from(size).map_err(|_| V2Error::Integrity {
            message: "negative artifact size".to_owned(),
        })?,
        content_type: row.try_get("content_type").map_err(V2Error::Database)?,
    })
}

fn required_u64(row: &PgRow, column: &str) -> Result<u64, V2Error> {
    let value: i64 = row.try_get(column).map_err(V2Error::Database)?;
    u64::try_from(value).map_err(|_| V2Error::Integrity {
        message: format!("negative {column}"),
    })
}

fn optional_u64(row: &PgRow, column: &str) -> Result<Option<u64>, V2Error> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(V2Error::Database)?
        .map(|value| {
            u64::try_from(value).map_err(|_| V2Error::Integrity {
                message: format!("negative {column}"),
            })
        })
        .transpose()
}

fn to_i64(value: u64, name: &str) -> Result<i64, V2Error> {
    i64::try_from(value).map_err(|_| V2Error::Integrity {
        message: format!("{name} exceeds PostgreSQL BIGINT"),
    })
}
