//! PostgreSQL-authoritative compilation job state machine.

use crate::{AppError, AppRepository, Database, Permission};
use core_types::{
    ArtifactId, ArtifactKind, BlobHash, CompileKey, CostClass, IdempotencyKey, JobId,
    LatexmkProfileId, ShellPolicy, SnapshotId, TenantId, TexEngine, TexEnvironmentId, UserId,
    WorkerId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::{str::FromStr, time::Duration};
use thiserror::Error;

/// Limits enforced inside queue admission and claiming transactions.
#[derive(Clone, Debug)]
pub struct QueueLimits {
    global_running: u32,
    per_user_running: u32,
    per_user_outstanding: u32,
    lease_duration: Duration,
    max_attempts: u32,
}
impl QueueLimits {
    pub fn new(
        global_running: u32,
        per_user_running: u32,
        per_user_outstanding: u32,
        lease_duration: Duration,
        max_attempts: u32,
    ) -> Result<Self, QueueError> {
        if global_running == 0
            || per_user_running == 0
            || per_user_outstanding == 0
            || lease_duration.is_zero()
            || max_attempts == 0
        {
            return Err(QueueError::InvalidConfiguration {
                message: "queue limits must all be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            global_running,
            per_user_running,
            per_user_outstanding,
            lease_duration,
            max_attempts,
        })
    }
    #[must_use]
    pub const fn global_running(&self) -> u32 {
        self.global_running
    }
    #[must_use]
    pub const fn per_user_running(&self) -> u32 {
        self.per_user_running
    }
    #[must_use]
    pub const fn per_user_outstanding(&self) -> u32 {
        self.per_user_outstanding
    }
    #[must_use]
    pub const fn lease_duration(&self) -> Duration {
        self.lease_duration
    }
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
    fn lease_millis(&self) -> Result<i64, QueueError> {
        i64::try_from(self.lease_duration.as_millis()).map_err(|_| {
            QueueError::InvalidConfiguration {
                message: "lease duration is too large".to_owned(),
            }
        })
    }
}

/// Immutable values captured with a durable manual compile submission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnqueueCompileJobV1 {
    pub job_id: JobId,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub workspace_id: WorkspaceId,
    pub snapshot_id: SnapshotId,
    pub compile_key: CompileKey,
    pub idempotency_key: IdempotencyKey,
    pub engine: TexEngine,
    pub tex_environment_id: TexEnvironmentId,
    pub latexmk_profile: LatexmkProfileId,
    pub shell_policy: ShellPolicy,
    pub synctex: bool,
    pub cost_class: CostClass,
    pub priority: i16,
}

/// Durable artifact metadata. Blob bytes are intentionally excluded.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedArtifactV1 {
    pub artifact_id: ArtifactId,
    pub kind: ArtifactKind,
    pub logical_name: core_types::LogicalPath,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub content_type: String,
}

/// A job returned to a worker after an atomic claim.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileJobRecordV1 {
    pub id: JobId,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub workspace_id: WorkspaceId,
    pub snapshot_id: SnapshotId,
    pub compile_key: CompileKey,
    pub engine: TexEngine,
    pub tex_environment_id: TexEnvironmentId,
    pub latexmk_profile: LatexmkProfileId,
    pub shell_policy: ShellPolicy,
    pub synctex: bool,
    pub attempt_count: u32,
}

/// Cache metadata; the manifest is read from immutable blob storage by the caller.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileCacheRecordV1 {
    pub compile_key: CompileKey,
    pub source_job_id: JobId,
    pub artifact_manifest_blob_hash: BlobHash,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CompletionOutcome {
    Succeeded,
    Cancelled,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InfrastructureOutcome {
    Requeued,
    Failed,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("invalid queue configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("compile queue admission rejected: {scope} limit {limit} reached")]
    AdmissionRejected { scope: &'static str, limit: u32 },
    #[error("idempotency key was already used for different compile inputs")]
    IdempotencyConflict,
    #[error("compile job not found")]
    NotFound,
    #[error("compile job lease is lost or owned by another worker")]
    LeaseLost,
    #[error("invalid compile job state transition from {state}")]
    InvalidState { state: String },
    #[error("persistent queue integrity violation: {message}")]
    Integrity { message: String },
    #[error("database operation failed")]
    Database(#[source] sqlx::Error),
}

/// Repository for all durable compile-job state transitions.
#[derive(Clone, Debug)]
pub struct PostgresCompileQueue {
    database: Database,
    limits: QueueLimits,
}

impl PostgresCompileQueue {
    #[must_use]
    pub fn new(database: Database, limits: QueueLimits) -> Self {
        Self { database, limits }
    }
    #[must_use]
    pub const fn limits(&self) -> &QueueLimits {
        &self.limits
    }

    pub async fn enqueue(&self, request: EnqueueCompileJobV1) -> Result<JobId, QueueError> {
        let permissions = AppRepository::new(self.database.clone())
            .effective_permissions(request.user_id, request.workspace_id)
            .await
            .map_err(map_permission_error)?;
        if !permissions.allows(Permission::CompileSubmit) {
            return Err(QueueError::NotFound);
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        lock_user(&mut tx, request.user_id).await?;
        if let Some(row) = sqlx::query("SELECT id,snapshot_id,compile_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex FROM latex_core.compile_jobs WHERE user_id=$1 AND idempotency_key=$2")
            .bind(request.user_id.as_uuid()).bind(request.idempotency_key.as_str()).fetch_optional(&mut *tx).await.map_err(QueueError::Database)? {
            let same = row.try_get::<String,_>("snapshot_id").map_err(QueueError::Database)? == request.snapshot_id.to_hex()
                && row.try_get::<String,_>("compile_key").map_err(QueueError::Database)? == request.compile_key.to_hex()
                && row.try_get::<String,_>("engine").map_err(QueueError::Database)? == request.engine.to_string()
                && row.try_get::<String,_>("tex_environment_id").map_err(QueueError::Database)? == request.tex_environment_id.as_str()
                && row.try_get::<String,_>("latexmk_profile").map_err(QueueError::Database)? == request.latexmk_profile.as_str()
                && row.try_get::<String,_>("shell_policy").map_err(QueueError::Database)? == request.shell_policy.to_string()
                && row.try_get::<bool,_>("synctex").map_err(QueueError::Database)? == request.synctex;
            if !same { return Err(QueueError::IdempotencyConflict); }
            let id = JobId::from_uuid(row.try_get("id").map_err(QueueError::Database)?);
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(id);
        }
        ensure_ownership(&mut tx, &request).await?;
        let outstanding: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs WHERE user_id=$1 AND state IN ('queued','claimed','running')")
            .bind(request.user_id.as_uuid()).fetch_one(&mut *tx).await.map_err(QueueError::Database)?;
        if outstanding >= i64::from(self.limits.per_user_outstanding) {
            return Err(QueueError::AdmissionRejected {
                scope: "per-user outstanding",
                limit: self.limits.per_user_outstanding,
            });
        }
        sqlx::query("INSERT INTO latex_core.compile_jobs (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,priority,state) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'queued')")
            .bind(request.job_id.as_uuid()).bind(request.tenant_id.as_uuid()).bind(request.user_id.as_uuid()).bind(request.workspace_id.as_uuid())
            .bind(request.snapshot_id.to_hex()).bind(request.compile_key.to_hex()).bind(request.idempotency_key.as_str()).bind(request.engine.to_string())
            .bind(request.tex_environment_id.as_str()).bind(request.latexmk_profile.as_str()).bind(request.shell_policy.to_string()).bind(request.synctex).bind(request.cost_class.to_string()).bind(request.priority)
            .execute(&mut *tx).await.map_err(QueueError::Database)?;
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(request.job_id)
    }

    /// Atomically claims one eligible job using PostgreSQL ordering and `SKIP LOCKED`.
    pub async fn claim(
        &self,
        worker_id: WorkerId,
    ) -> Result<Option<CompileJobRecordV1>, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        lock_global(&mut tx).await?;
        let running: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.compile_jobs WHERE state IN ('claimed','running')",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(QueueError::Database)?;
        if running >= i64::from(self.limits.global_running) {
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(None);
        }
        let row = sqlx::query("WITH candidate AS (SELECT j.id FROM latex_core.compile_jobs j WHERE j.state='queued' AND (SELECT count(*) FROM latex_core.compile_jobs active WHERE active.user_id=j.user_id AND active.state IN ('claimed','running')) < $1 ORDER BY j.priority DESC,j.created_at ASC,j.id ASC FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE latex_core.compile_jobs j SET state='running',worker_id=$2,lease_until=statement_timestamp() + ($3::bigint * interval '1 millisecond'),claimed_at=COALESCE(j.claimed_at,statement_timestamp()),started_at=COALESCE(j.started_at,statement_timestamp()),attempt_count=j.attempt_count+1,updated_at=statement_timestamp() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.tenant_id,j.user_id,j.workspace_id,j.snapshot_id,j.compile_key,j.engine,j.tex_environment_id,j.latexmk_profile,j.shell_policy,j.synctex,j.attempt_count")
            .bind(i64::from(self.limits.per_user_running)).bind(worker_id.as_uuid()).bind(self.limits.lease_millis()?)
            .fetch_optional(&mut *tx).await.map_err(QueueError::Database)?;
        if let Some(ref claimed) = row {
            let id: uuid::Uuid = claimed.try_get("id").map_err(QueueError::Database)?;
            sqlx::query("UPDATE latex_core.v2_paper_builds SET status='running',updated_at=statement_timestamp() WHERE compile_job_id=$1")
                .bind(id).execute(&mut *tx).await.map_err(QueueError::Database)?;
        }
        tx.commit().await.map_err(QueueError::Database)?;
        row.map(decode_job).transpose()
    }

    pub async fn renew_lease(&self, job_id: JobId, worker_id: WorkerId) -> Result<(), QueueError> {
        let affected = sqlx::query("UPDATE latex_core.compile_jobs SET lease_until=statement_timestamp() + ($3::bigint * interval '1 millisecond'),updated_at=statement_timestamp() WHERE id=$1 AND state='running' AND worker_id=$2 AND lease_until>statement_timestamp()")
            .bind(job_id.as_uuid()).bind(worker_id.as_uuid()).bind(self.limits.lease_millis()?).execute(self.database.pool()).await.map_err(QueueError::Database)?.rows_affected();
        if affected == 1 {
            Ok(())
        } else {
            Err(QueueError::LeaseLost)
        }
    }

    /// Cancelling queued work is terminal immediately; running work gets a durable request.
    pub async fn request_cancellation(
        &self,
        job_id: JobId,
        reason: Option<&str>,
    ) -> Result<(), QueueError> {
        if reason.is_some_and(|value| value.is_empty() || value.len() > 512) {
            return Err(QueueError::Integrity {
                message: "cancellation reason must be 1..=512 bytes".to_owned(),
            });
        }
        let result = sqlx::query("UPDATE latex_core.compile_jobs SET state=CASE WHEN state='queued' THEN 'cancelled' ELSE state END,cancellation_requested_at=COALESCE(cancellation_requested_at,statement_timestamp()),cancellation_reason=COALESCE(cancellation_reason,$2),finished_at=CASE WHEN state='queued' THEN statement_timestamp() ELSE finished_at END,worker_id=CASE WHEN state='queued' THEN NULL ELSE worker_id END,lease_until=CASE WHEN state='queued' THEN NULL ELSE lease_until END,updated_at=statement_timestamp() WHERE id=$1 AND state IN ('queued','claimed','running')")
            .bind(job_id.as_uuid()).bind(reason).execute(self.database.pool()).await.map_err(QueueError::Database)?.rows_affected();
        if result == 1 {
            Ok(())
        } else {
            self.existing_or_not_found(job_id).await
        }
    }

    /// Requeues expired infrastructure work, or fails it once retry budget is exhausted.
    pub async fn recover_expired_leases(&self) -> Result<u64, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        let result = sqlx::query("UPDATE latex_core.compile_jobs SET state=CASE WHEN cancellation_requested_at IS NOT NULL THEN 'cancelled' WHEN attempt_count >= $1 THEN 'failed' ELSE 'queued' END,worker_id=NULL,lease_until=NULL,finished_at=CASE WHEN cancellation_requested_at IS NOT NULL OR attempt_count >= $1 THEN statement_timestamp() ELSE NULL END,error_class=CASE WHEN cancellation_requested_at IS NOT NULL THEN 'cancelled' WHEN attempt_count >= $1 THEN 'infrastructure' ELSE error_class END,last_error=CASE WHEN cancellation_requested_at IS NOT NULL THEN last_error WHEN attempt_count >= $1 THEN jsonb_build_object('class','infrastructure','message','worker lease expired; retry budget exhausted') ELSE jsonb_build_object('class','infrastructure','message','worker lease expired; requeued') END,updated_at=statement_timestamp() WHERE state IN ('claimed','running') AND lease_until<=statement_timestamp()")
            .bind(i32::try_from(self.limits.max_attempts).map_err(|_| QueueError::InvalidConfiguration { message: "max attempts too large".to_owned() })?).execute(&mut *tx).await.map_err(QueueError::Database)?;
        let terminal_v2 = sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT j.id FROM latex_core.compile_jobs j JOIN latex_core.v2_paper_builds b ON b.compile_job_id=j.id WHERE j.state='failed' AND b.status IN ('queued','running')",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(QueueError::Database)?;
        for job in terminal_v2 {
            finalize_v2_build(&mut tx, JobId::from_uuid(job), false).await?;
        }
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(result.rows_affected())
    }

    pub async fn complete_success(
        &self,
        job_id: JobId,
        worker_id: WorkerId,
        artifacts: &[PersistedArtifactV1],
        manifest_hash: BlobHash,
    ) -> Result<CompletionOutcome, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        let key = self.assert_live_lease(&mut tx, job_id, worker_id).await?;
        if key.is_none() {
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(CompletionOutcome::Cancelled);
        }
        let key = key.ok_or_else(|| QueueError::Integrity {
            message: "missing claimed compile key".to_owned(),
        })?;
        insert_artifacts(&mut tx, job_id, key, artifacts).await?;
        sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ($1,$2,$3) ON CONFLICT (compile_key) DO UPDATE SET last_accessed_at=statement_timestamp()")
            .bind(key.to_hex()).bind(job_id.as_uuid()).bind(manifest_hash.to_hex()).execute(&mut *tx).await.map_err(QueueError::Database)?;
        finish(&mut tx, job_id, worker_id, "succeeded", None, None, None).await?;
        finalize_v2_build(&mut tx, job_id, true).await?;
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(CompletionOutcome::Succeeded)
    }

    pub async fn complete_cached_success(
        &self,
        job_id: JobId,
        worker_id: WorkerId,
        cache: CompileCacheRecordV1,
        artifacts: &[PersistedArtifactV1],
    ) -> Result<CompletionOutcome, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        let key = self.assert_live_lease(&mut tx, job_id, worker_id).await?;
        if key.is_none() {
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(CompletionOutcome::Cancelled);
        }
        let key = key.ok_or_else(|| QueueError::Integrity {
            message: "missing claimed compile key".to_owned(),
        })?;
        if key != cache.compile_key {
            return Err(QueueError::Integrity {
                message: "cache key does not match job".to_owned(),
            });
        }
        insert_artifacts(&mut tx, job_id, key, artifacts).await?;
        sqlx::query("UPDATE latex_core.compile_cache SET last_accessed_at=statement_timestamp() WHERE compile_key=$1 AND source_job_id=$2 AND artifact_manifest_blob_hash=$3")
            .bind(key.to_hex()).bind(cache.source_job_id.as_uuid()).bind(cache.artifact_manifest_blob_hash.to_hex()).execute(&mut *tx).await.map_err(QueueError::Database)?;
        finish(
            &mut tx,
            job_id,
            worker_id,
            "succeeded",
            None,
            None,
            Some(cache.source_job_id),
        )
        .await?;
        finalize_v2_build(&mut tx, job_id, true).await?;
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(CompletionOutcome::Succeeded)
    }

    pub async fn complete_compile_failure(
        &self,
        job_id: JobId,
        worker_id: WorkerId,
        timed_out: bool,
        error: Value,
    ) -> Result<CompletionOutcome, QueueError> {
        self.complete_compile_failure_with_artifacts(job_id, worker_id, timed_out, error, &[])
            .await
    }

    pub async fn complete_compile_failure_with_artifacts(
        &self,
        job_id: JobId,
        worker_id: WorkerId,
        timed_out: bool,
        error: Value,
        artifacts: &[PersistedArtifactV1],
    ) -> Result<CompletionOutcome, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        let key = self.assert_live_lease(&mut tx, job_id, worker_id).await?;
        if key.is_none() {
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(CompletionOutcome::Cancelled);
        }
        insert_artifacts(
            &mut tx,
            job_id,
            key.ok_or_else(|| integrity("missing claimed compile key"))?,
            artifacts,
        )
        .await?;
        finish(
            &mut tx,
            job_id,
            worker_id,
            if timed_out { "timed_out" } else { "failed" },
            Some("compile"),
            Some(error),
            None,
        )
        .await?;
        finalize_v2_build(&mut tx, job_id, false).await?;
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(CompletionOutcome::Succeeded)
    }

    pub async fn complete_infrastructure_failure(
        &self,
        job_id: JobId,
        worker_id: WorkerId,
        error: Value,
    ) -> Result<InfrastructureOutcome, QueueError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(QueueError::Database)?;
        let key = self.assert_live_lease(&mut tx, job_id, worker_id).await?;
        if key.is_none() {
            tx.commit().await.map_err(QueueError::Database)?;
            return Ok(InfrastructureOutcome::Cancelled);
        }
        let attempts: i32 = sqlx::query_scalar(
            "SELECT attempt_count FROM latex_core.compile_jobs WHERE id=$1 FOR UPDATE",
        )
        .bind(job_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(QueueError::Database)?;
        let exhausted = u32::try_from(attempts).map_err(|_| QueueError::Integrity {
            message: "negative attempt count".to_owned(),
        })? >= self.limits.max_attempts;
        if exhausted {
            finish(
                &mut tx,
                job_id,
                worker_id,
                "failed",
                Some("infrastructure"),
                Some(error),
                None,
            )
            .await?;
            finalize_v2_build(&mut tx, job_id, false).await?;
        } else {
            sqlx::query("UPDATE latex_core.compile_jobs SET state='queued',worker_id=NULL,lease_until=NULL,updated_at=statement_timestamp(),error_class='infrastructure',last_error=$2 WHERE id=$1 AND worker_id=$3 AND state='running'")
                .bind(job_id.as_uuid()).bind(error).bind(worker_id.as_uuid()).execute(&mut *tx).await.map_err(QueueError::Database)?;
        }
        tx.commit().await.map_err(QueueError::Database)?;
        Ok(if exhausted {
            InfrastructureOutcome::Failed
        } else {
            InfrastructureOutcome::Requeued
        })
    }

    pub async fn cache_entry(
        &self,
        key: CompileKey,
    ) -> Result<Option<CompileCacheRecordV1>, QueueError> {
        let row = sqlx::query("SELECT compile_key,source_job_id,artifact_manifest_blob_hash FROM latex_core.compile_cache WHERE compile_key=$1")
            .bind(key.to_hex()).fetch_optional(self.database.pool()).await.map_err(QueueError::Database)?;
        row.map(|row| {
            Ok(CompileCacheRecordV1 {
                compile_key: parse_digest(
                    row.try_get("compile_key").map_err(QueueError::Database)?,
                )?,
                source_job_id: JobId::from_uuid(
                    row.try_get("source_job_id").map_err(QueueError::Database)?,
                ),
                artifact_manifest_blob_hash: parse_blob(
                    row.try_get("artifact_manifest_blob_hash")
                        .map_err(QueueError::Database)?,
                )?,
            })
        })
        .transpose()
    }
    pub async fn evict_cache_if_matches(
        &self,
        cache: CompileCacheRecordV1,
    ) -> Result<(), QueueError> {
        sqlx::query("DELETE FROM latex_core.compile_cache WHERE compile_key=$1 AND source_job_id=$2 AND artifact_manifest_blob_hash=$3")
            .bind(cache.compile_key.to_hex()).bind(cache.source_job_id.as_uuid()).bind(cache.artifact_manifest_blob_hash.to_hex()).execute(self.database.pool()).await.map_err(QueueError::Database)?;
        Ok(())
    }
    pub async fn snapshot_manifest_blob(
        &self,
        snapshot_id: SnapshotId,
    ) -> Result<BlobHash, QueueError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT manifest_blob_hash FROM latex_core.snapshots WHERE snapshot_id=$1",
        )
        .bind(snapshot_id.to_hex())
        .fetch_optional(self.database.pool())
        .await
        .map_err(QueueError::Database)?;
        parse_blob(value.ok_or(QueueError::NotFound)?)
    }
    async fn existing_or_not_found(&self, job_id: JobId) -> Result<(), QueueError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.compile_jobs WHERE id=$1)")
                .bind(job_id.as_uuid())
                .fetch_one(self.database.pool())
                .await
                .map_err(QueueError::Database)?;
        if exists {
            Err(QueueError::InvalidState {
                state: "terminal".to_owned(),
            })
        } else {
            Err(QueueError::NotFound)
        }
    }
    async fn assert_live_lease(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        job_id: JobId,
        worker_id: WorkerId,
    ) -> Result<Option<CompileKey>, QueueError> {
        let row = sqlx::query("SELECT state,worker_id,lease_until>statement_timestamp() AS live,cancellation_requested_at IS NOT NULL AS cancelled,compile_key FROM latex_core.compile_jobs WHERE id=$1 FOR UPDATE")
            .bind(job_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(QueueError::Database)?.ok_or(QueueError::NotFound)?;
        let state: String = row.try_get("state").map_err(QueueError::Database)?;
        if state != "running" {
            return Err(QueueError::InvalidState { state });
        }
        let owner: Option<uuid::Uuid> = row.try_get("worker_id").map_err(QueueError::Database)?;
        let live: bool = row.try_get("live").map_err(QueueError::Database)?;
        if owner != Some(*worker_id.as_uuid()) || !live {
            return Err(QueueError::LeaseLost);
        }
        if row
            .try_get::<bool, _>("cancelled")
            .map_err(QueueError::Database)?
        {
            finish(
                tx,
                job_id,
                worker_id,
                "cancelled",
                Some("cancelled"),
                None,
                None,
            )
            .await?;
            return Ok(None);
        }
        parse_digest(row.try_get("compile_key").map_err(QueueError::Database)?).map(Some)
    }
}

fn map_permission_error(error: AppError) -> QueueError {
    match error {
        AppError::NotFound | AppError::Forbidden => QueueError::NotFound,
        AppError::Conflict | AppError::DraftConflict => QueueError::Integrity {
            message: error.to_string(),
        },
        AppError::Integrity { message } => QueueError::Integrity { message },
        AppError::Mail(error) => QueueError::Integrity {
            message: error.to_string(),
        },
        AppError::Database(error) => QueueError::Database(error),
    }
}

async fn ensure_ownership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &EnqueueCompileJobV1,
) -> Result<(), QueueError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.workspaces w LEFT JOIN latex_core.team_projects tp ON tp.workspace_id=w.id LEFT JOIN latex_core.team_project_members pm ON pm.team_project_id=tp.id AND pm.user_id=$3 WHERE w.id=$1 AND ((w.tenant_id=$2 AND w.owner_user_id=$3) OR pm.writer OR pm.mentor))")
        .bind(request.workspace_id.as_uuid()).bind(request.tenant_id.as_uuid()).bind(request.user_id.as_uuid()).fetch_one(&mut **tx).await.map_err(QueueError::Database)?;
    if !exists {
        return Err(QueueError::NotFound);
    }
    let snapshot: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.snapshots WHERE snapshot_id=$1)",
    )
    .bind(request.snapshot_id.to_hex())
    .fetch_one(&mut **tx)
    .await
    .map_err(QueueError::Database)?;
    if !snapshot {
        return Err(QueueError::NotFound);
    }
    Ok(())
}
async fn lock_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: UserId,
) -> Result<(), QueueError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(user.to_string())
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)
        .map(|_| ())
}
async fn lock_global(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), QueueError> {
    sqlx::query("SELECT pg_advisory_xact_lock(4815162342::bigint)")
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)
        .map(|_| ())
}
async fn insert_artifacts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: JobId,
    key: CompileKey,
    artifacts: &[PersistedArtifactV1],
) -> Result<(), QueueError> {
    for artifact in artifacts {
        let size = i64::try_from(artifact.size_bytes).map_err(|_| QueueError::Integrity {
            message: "artifact size exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        sqlx::query("INSERT INTO latex_core.compilation_artifacts (artifact_id,job_id,compile_key,kind,logical_name,blob_hash,size_bytes,content_type) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(artifact.artifact_id.as_uuid()).bind(job.as_uuid()).bind(key.to_hex()).bind(artifact_kind_text(artifact.kind)).bind(artifact.logical_name.as_str()).bind(artifact.blob_hash.to_hex()).bind(size).bind(&artifact.content_type).execute(&mut **tx).await.map_err(QueueError::Database)?;
    }
    Ok(())
}

/// Finalizes optional S4 metadata in the same transaction as the legacy queue
/// transition, then promotes only an exact desired-state match and activates
/// at most the single newest pending snapshot.
#[allow(
    clippy::too_many_lines,
    reason = "promotion and newest-pending activation must remain one auditable PostgreSQL transaction"
)]
async fn finalize_v2_build(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: JobId,
    succeeded: bool,
) -> Result<(), QueueError> {
    let build = sqlx::query(
        "SELECT id,workspace_id,state_hash FROM latex_core.v2_paper_builds WHERE compile_job_id=$1",
    )
    .bind(job.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(QueueError::Database)?;
    let Some(build) = build else {
        return Ok(());
    };
    let build_id: uuid::Uuid = build.try_get("id").map_err(QueueError::Database)?;
    let workspace_id: uuid::Uuid = build
        .try_get("workspace_id")
        .map_err(QueueError::Database)?;
    let state_hash: String = build.try_get("state_hash").map_err(QueueError::Database)?;
    let scheduler = sqlx::query(
        "SELECT * FROM latex_core.v2_paper_build_state WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(QueueError::Database)?;
    sqlx::query(
        "UPDATE latex_core.v2_paper_builds SET status=$2,updated_at=statement_timestamp() WHERE id=$1",
    )
    .bind(build_id)
    .bind(if succeeded { "succeeded" } else { "failed" })
    .execute(&mut **tx)
    .await
    .map_err(QueueError::Database)?;
    let desired: Option<String> = scheduler
        .try_get("desired_state_hash")
        .map_err(QueueError::Database)?;
    if succeeded && desired.as_deref() == Some(state_hash.as_str()) {
        sqlx::query(
            "UPDATE latex_core.v2_paper_build_state SET current_build_id=$2,updated_at=statement_timestamp() WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .bind(build_id)
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
        sqlx::query(
            "UPDATE latex_core.v2_paper_builds SET promoted_at=statement_timestamp(),updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(build_id)
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
    }

    let pending_hash: Option<String> = scheduler
        .try_get("pending_state_hash")
        .map_err(QueueError::Database)?;
    let next_build = if let Some(pending_hash) = pending_hash {
        let next_build = uuid::Uuid::new_v4();
        let next_version = uuid::Uuid::new_v4();
        let paper_id: uuid::Uuid = scheduler
            .try_get("paper_id")
            .map_err(QueueError::Database)?;
        let snapshot: String = required_pending(&scheduler, "pending_snapshot_id")?;
        let manifest: Value = required_pending(&scheduler, "pending_manifest")?;
        let sequence: i64 = required_pending(&scheduler, "pending_source_sequence")?;
        let epoch: i64 = required_pending(&scheduler, "pending_document_epoch")?;
        let tenant: uuid::Uuid = required_pending(&scheduler, "pending_tenant_id")?;
        let user: uuid::Uuid = required_pending(&scheduler, "pending_user_id")?;
        let trigger: String = required_pending(&scheduler, "pending_trigger_type")?;
        let compile_key: String = required_pending(&scheduler, "pending_compile_key")?;
        let engine: String = required_pending(&scheduler, "pending_engine")?;
        let environment: String = required_pending(&scheduler, "pending_tex_environment_id")?;
        let profile: String = required_pending(&scheduler, "pending_latexmk_profile")?;
        let shell: String = required_pending(&scheduler, "pending_shell_policy")?;
        let synctex: bool = required_pending(&scheduler, "pending_synctex")?;
        let number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.compile_jobs \
             (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,priority,state) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'normal',$13,'queued')",
        )
        .bind(next_build)
        .bind(tenant)
        .bind(user)
        .bind(workspace_id)
        .bind(&snapshot)
        .bind(&compile_key)
        .bind(format!("v2:{next_build}"))
        .bind(&engine)
        .bind(&environment)
        .bind(&profile)
        .bind(&shell)
        .bind(synctex)
        .bind(if trigger == "manual" { 10_i16 } else { 0_i16 })
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.paper_versions \
             (id,paper_id,workspace_id,document_epoch,version_number,version_type,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) \
             VALUES ($1,$2,$3,$4,$5,'compile_checkpoint',$6,$7,$8,$9,$10)",
        )
        .bind(next_version)
        .bind(paper_id)
        .bind(workspace_id)
        .bind(epoch)
        .bind(number)
        .bind(user)
        .bind(sequence)
        .bind(&snapshot)
        .bind(manifest)
        .bind(&pending_hash)
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.v2_paper_builds \
             (id,paper_id,workspace_id,compile_job_id,version_id,document_epoch,source_sequence,state_hash,trigger_type,status) \
             VALUES ($1,$2,$3,$1,$4,$5,$6,$7,$8,'queued')",
        )
        .bind(next_build)
        .bind(paper_id)
        .bind(workspace_id)
        .bind(next_version)
        .bind(epoch)
        .bind(sequence)
        .bind(pending_hash)
        .bind(trigger)
        .execute(&mut **tx)
        .await
        .map_err(QueueError::Database)?;
        Some(next_build)
    } else {
        None
    };
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET active_build_id=$2, \
         pending_snapshot_id=NULL,pending_manifest=NULL,pending_state_hash=NULL,pending_source_sequence=NULL, \
         pending_document_epoch=NULL,pending_tenant_id=NULL,pending_user_id=NULL,pending_trigger_type=NULL, \
         pending_compile_key=NULL,pending_engine=NULL,pending_tex_environment_id=NULL,pending_latexmk_profile=NULL, \
         pending_shell_policy=NULL,pending_synctex=NULL,updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id)
    .bind(next_build)
    .execute(&mut **tx)
    .await
    .map_err(QueueError::Database)?;
    Ok(())
}

fn required_pending<T>(row: &sqlx::postgres::PgRow, column: &str) -> Result<T, QueueError>
where
    for<'r> T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get::<Option<T>, _>(column)
        .map_err(QueueError::Database)?
        .ok_or_else(|| integrity(format!("incomplete pending V2 build: {column}")))
}
async fn finish(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: JobId,
    worker: WorkerId,
    state: &str,
    error_class: Option<&str>,
    error: Option<Value>,
    cache_source: Option<JobId>,
) -> Result<(), QueueError> {
    let affected = sqlx::query("UPDATE latex_core.compile_jobs SET state=$3,worker_id=NULL,lease_until=NULL,finished_at=statement_timestamp(),updated_at=statement_timestamp(),error_class=$4,last_error=$5,cache_source_job_id=$6 WHERE id=$1 AND state='running' AND worker_id=$2 AND lease_until>statement_timestamp()")
        .bind(job.as_uuid()).bind(worker.as_uuid()).bind(state).bind(error_class).bind(error).bind(cache_source.map(|id| *id.as_uuid())).execute(&mut **tx).await.map_err(QueueError::Database)?.rows_affected();
    if affected == 1 {
        Ok(())
    } else {
        Err(QueueError::LeaseLost)
    }
}
fn decode_job(row: sqlx::postgres::PgRow) -> Result<CompileJobRecordV1, QueueError> {
    Ok(CompileJobRecordV1 {
        id: JobId::from_uuid(row.try_get("id").map_err(QueueError::Database)?),
        tenant_id: TenantId::from_uuid(row.try_get("tenant_id").map_err(QueueError::Database)?),
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(QueueError::Database)?),
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(QueueError::Database)?,
        ),
        snapshot_id: parse_snapshot(row.try_get("snapshot_id").map_err(QueueError::Database)?)?,
        compile_key: parse_digest(row.try_get("compile_key").map_err(QueueError::Database)?)?,
        engine: TexEngine::from_str(
            &row.try_get::<String, _>("engine")
                .map_err(QueueError::Database)?,
        )
        .map_err(|error| integrity(error.to_string()))?,
        tex_environment_id: TexEnvironmentId::parse(
            &row.try_get::<String, _>("tex_environment_id")
                .map_err(QueueError::Database)?,
        )
        .map_err(|error| integrity(error.to_string()))?,
        latexmk_profile: LatexmkProfileId::parse(
            &row.try_get::<String, _>("latexmk_profile")
                .map_err(QueueError::Database)?,
        )
        .map_err(|error| integrity(error.to_string()))?,
        shell_policy: parse_shell(
            &row.try_get::<String, _>("shell_policy")
                .map_err(QueueError::Database)?,
        )?,
        synctex: row.try_get("synctex").map_err(QueueError::Database)?,
        attempt_count: u32::try_from(
            row.try_get::<i32, _>("attempt_count")
                .map_err(QueueError::Database)?,
        )
        .map_err(|_| integrity("negative attempt count"))?,
    })
}
fn parse_snapshot(value: String) -> Result<SnapshotId, QueueError> {
    SnapshotId::from_str(&value).map_err(|error| integrity(error.to_string()))
}
fn parse_digest(value: String) -> Result<CompileKey, QueueError> {
    CompileKey::from_str(&value).map_err(|error| integrity(error.to_string()))
}
fn parse_blob(value: String) -> Result<BlobHash, QueueError> {
    BlobHash::from_str(&value).map_err(|error| integrity(error.to_string()))
}
fn parse_shell(value: &str) -> Result<ShellPolicy, QueueError> {
    match value {
        "safe" => Ok(ShellPolicy::Safe),
        "restricted" => Ok(ShellPolicy::Restricted),
        "compatibility" => Ok(ShellPolicy::Compatibility),
        _ => Err(integrity("invalid persisted shell policy")),
    }
}
fn artifact_kind_text(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Pdf => "pdf",
        ArtifactKind::Log => "log",
        ArtifactKind::Synctex => "synctex",
        ArtifactKind::Fls => "fls",
        ArtifactKind::Aux => "aux",
        ArtifactKind::Bcf => "bcf",
        ArtifactKind::Toc => "toc",
        ArtifactKind::Other => "other",
    }
}
fn integrity(message: impl Into<String>) -> QueueError {
    QueueError::Integrity {
        message: message.into(),
    }
}
