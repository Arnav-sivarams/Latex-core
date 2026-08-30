//! Bounded worker orchestration for PostgreSQL-authoritative compile jobs.
#![forbid(unsafe_code)]
#![allow(
    clippy::doc_markdown,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::single_match,
    clippy::single_match_else,
    clippy::too_many_lines,
    reason = "worker error boundaries deliberately preserve queue-vs-storage distinctions"
)]

use async_trait::async_trait;
use blob_store::{BlobStore, BlobStoreError};
use bytes::Bytes;
use compiler::{CompileExecution, CompileStatus, CompilerError, CompilerService, ContainerRuntime};
use core_types::{
    ArtifactId, ArtifactManifestV1, ArtifactRefV1, BlobHash, LogicalPath, WorkerId,
    WorkspaceManifestV1,
};
use persistence::{
    CompileCacheRecordV1, CompileJobRecordV1, CompletionOutcome, InfrastructureOutcome,
    PersistedArtifactV1, PostgresCompileQueue, QueueError,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::time::sleep;

/// Testable execution result independent of the M7 runtime implementation.
#[derive(Clone, Debug)]
pub struct WorkerArtifact {
    pub kind: core_types::ArtifactKind,
    pub logical_name: LogicalPath,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct WorkerExecution {
    pub status: CompileStatus,
    pub compile_key: core_types::CompileKey,
    pub environment_id: core_types::TexEnvironmentId,
    pub latexmk_profile: core_types::LatexmkProfileId,
    pub artifacts: Vec<WorkerArtifact>,
    pub exit_code: Option<i32>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}
impl From<CompileExecution> for WorkerExecution {
    fn from(value: CompileExecution) -> Self {
        Self {
            status: value.status(),
            compile_key: value.compile_key(),
            environment_id: value.environment_id().clone(),
            latexmk_profile: value.latexmk_profile().clone(),
            artifacts: value
                .artifacts()
                .iter()
                .map(|artifact| WorkerArtifact {
                    kind: artifact.kind(),
                    logical_name: artifact.logical_name().clone(),
                    bytes: artifact.bytes().clone(),
                })
                .collect(),
            exit_code: value.exit_code(),
            stdout_truncated: value.stdout_truncated(),
            stderr_truncated: value.stderr_truncated(),
        }
    }
}

/// The narrow execution seam used by the worker; it permits Docker-free worker tests.
#[async_trait]
pub trait CompileExecutor: Send + Sync + 'static {
    async fn compile_job(
        &self,
        job: &CompileJobRecordV1,
        manifest: &WorkspaceManifestV1,
    ) -> Result<WorkerExecution, CompilerError>;
}

#[async_trait]
impl<R: ContainerRuntime> CompileExecutor for CompilerService<R> {
    async fn compile_job(
        &self,
        job: &CompileJobRecordV1,
        manifest: &WorkspaceManifestV1,
    ) -> Result<WorkerExecution, CompilerError> {
        self.compile_with_execution_id(
            job.snapshot_id,
            manifest,
            job.engine,
            job.shell_policy,
            job.synctex,
            &job.id.to_string(),
        )
        .await
        .map(WorkerExecution::from)
    }
}

/// Fixed worker-pool controls. There is no unbounded task spawning.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    concurrency: usize,
    idle_backoff: Duration,
}
impl WorkerConfig {
    pub fn new(concurrency: usize, idle_backoff: Duration) -> Result<Self, WorkerError> {
        if concurrency == 0 || idle_backoff.is_zero() {
            return Err(WorkerError::InvalidConfiguration {
                message: "worker concurrency and idle backoff must be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            concurrency,
            idle_backoff,
        })
    }
    #[must_use]
    pub const fn concurrency(&self) -> usize {
        self.concurrency
    }
    #[must_use]
    pub const fn idle_backoff(&self) -> Duration {
        self.idle_backoff
    }
}

/// Explicit, caller-owned worker shutdown signal.
#[derive(Clone, Debug, Default)]
pub struct WorkerShutdown(Arc<AtomicBool>);
impl WorkerShutdown {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn request(&self) {
        self.0.store(true, Ordering::Release);
    }
    #[must_use]
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WorkerRunOutcome {
    Idle,
    CacheHit,
    Compiled,
    CompileFailed,
    TimedOut,
    RequeuedInfrastructure,
    FailedInfrastructure,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("invalid worker configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("durable queue operation failed")]
    Queue(#[from] QueueError),
    #[error("blob storage operation failed")]
    Blob(#[from] BlobStoreError),
    #[error("compiler operation failed")]
    Compiler(#[from] CompilerError),
    #[error("invalid persisted manifest: {message}")]
    Manifest { message: String },
}

/// One bounded worker service. PostgreSQL owns job state; this type owns no queue state.
pub struct CompilationWorker<E: CompileExecutor> {
    queue: PostgresCompileQueue,
    blobs: Arc<dyn BlobStore>,
    executor: Arc<E>,
    worker_id: WorkerId,
    config: WorkerConfig,
}
impl<E: CompileExecutor> Clone for CompilationWorker<E> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            blobs: Arc::clone(&self.blobs),
            executor: Arc::clone(&self.executor),
            worker_id: self.worker_id,
            config: self.config.clone(),
        }
    }
}
impl<E: CompileExecutor> std::fmt::Debug for CompilationWorker<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilationWorker")
            .field("worker_id", &self.worker_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl<E: CompileExecutor> CompilationWorker<E> {
    #[must_use]
    pub fn new(
        queue: PostgresCompileQueue,
        blobs: Arc<dyn BlobStore>,
        executor: Arc<E>,
        worker_id: WorkerId,
        config: WorkerConfig,
    ) -> Self {
        Self {
            queue,
            blobs,
            executor,
            worker_id,
            config,
        }
    }

    /// Runs one claim/execute/finalize cycle, useful for deterministic supervision and tests.
    pub async fn run_once(&self) -> Result<WorkerRunOutcome, WorkerError> {
        let Some(job) = self.queue.claim(self.worker_id).await? else {
            return Ok(WorkerRunOutcome::Idle);
        };
        if let Some(cache) = self.queue.cache_entry(job.compile_key).await? {
            match self.try_cache_hit(&job, cache).await {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(error) => return self.infrastructure_failure(job.id, error).await,
            }
        }
        let manifest_hash = match self.queue.snapshot_manifest_blob(job.snapshot_id).await {
            Ok(value) => value,
            Err(error) => {
                return self
                    .infrastructure_failure(job.id, WorkerError::Queue(error))
                    .await;
            }
        };
        let manifest_bytes = match self.blobs.get(manifest_hash).await {
            Ok(value) => value,
            Err(error) => {
                return self
                    .infrastructure_failure(job.id, WorkerError::Blob(error))
                    .await;
            }
        };
        let manifest: WorkspaceManifestV1 = match serde_json::from_slice(&manifest_bytes) {
            Ok(value) => value,
            Err(error) => {
                return self
                    .infrastructure_failure(
                        job.id,
                        WorkerError::Manifest {
                            message: error.to_string(),
                        },
                    )
                    .await;
            }
        };
        match self.compile_with_lease(&job, &manifest).await {
            Ok(execution) => self.handle_execution(job, execution).await,
            Err(WorkerError::Compiler(error)) if infrastructure_compiler_error(&error) => {
                self.infrastructure_failure(job.id, WorkerError::Compiler(error))
                    .await
            }
            Err(WorkerError::Compiler(error)) => {
                self.compile_failure(job.id, false, error.to_string()).await
            }
            Err(error) => self.infrastructure_failure(job.id, error).await,
        }
    }

    /// Starts exactly `WorkerConfig::concurrency` cooperative loops and awaits their exit.
    pub async fn run_until_shutdown(
        &self,
        shutdown: WorkerShutdown,
    ) -> Result<(), tokio::task::JoinError> {
        let mut handles = Vec::with_capacity(self.config.concurrency);
        for _ in 0..self.config.concurrency {
            let worker = self.clone();
            let shutdown = shutdown.clone();
            handles.push(tokio::spawn(async move {
                while !shutdown.requested() {
                    let result = worker.run_once().await;
                    if needs_backoff(&result) {
                        sleep(worker.config.idle_backoff).await;
                    }
                }
            }));
        }
        for handle in handles {
            handle.await?;
        }
        Ok(())
    }

    async fn try_cache_hit(
        &self,
        job: &CompileJobRecordV1,
        cache: CompileCacheRecordV1,
    ) -> Result<Option<WorkerRunOutcome>, WorkerError> {
        let bytes = match self.blobs.get(cache.artifact_manifest_blob_hash).await {
            Ok(value) => value,
            Err(_) => {
                self.queue.evict_cache_if_matches(cache).await?;
                return Ok(None);
            }
        };
        let manifest: ArtifactManifestV1 = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                self.queue.evict_cache_if_matches(cache).await?;
                return Ok(None);
            }
        };
        if manifest.compile_key() != job.compile_key {
            self.queue.evict_cache_if_matches(cache).await?;
            return Ok(None);
        }
        let mut artifacts = Vec::with_capacity(manifest.artifacts().len());
        for source in manifest.artifacts() {
            let metadata = match self.blobs.metadata(source.blob_hash).await {
                Ok(value) => value,
                Err(_) => {
                    self.queue.evict_cache_if_matches(cache).await?;
                    return Ok(None);
                }
            };
            if metadata.hash() != source.blob_hash || metadata.size_bytes() != source.size_bytes {
                self.queue.evict_cache_if_matches(cache).await?;
                return Ok(None);
            }
            artifacts.push(PersistedArtifactV1 {
                artifact_id: ArtifactId::new(),
                kind: source.kind,
                logical_name: source.logical_name.clone(),
                blob_hash: source.blob_hash,
                size_bytes: source.size_bytes,
                content_type: content_type(&source.logical_name).to_owned(),
            });
        }
        match self
            .queue
            .complete_cached_success(job.id, self.worker_id, cache, &artifacts)
            .await?
        {
            CompletionOutcome::Succeeded => Ok(Some(WorkerRunOutcome::CacheHit)),
            CompletionOutcome::Cancelled => Ok(Some(WorkerRunOutcome::Cancelled)),
        }
    }

    /// Keeps the database lease alive without holding a transaction over compilation.
    async fn compile_with_lease(
        &self,
        job: &CompileJobRecordV1,
        manifest: &WorkspaceManifestV1,
    ) -> Result<WorkerExecution, WorkerError> {
        let millis = self.queue.limits().lease_duration().as_millis() / 3;
        let renewal = Duration::from_millis(u64::try_from(millis.max(1)).unwrap_or(u64::MAX));
        let compile = self.executor.compile_job(job, manifest);
        tokio::pin!(compile);
        let mut ticker = tokio::time::interval(renewal);
        ticker.tick().await;
        loop {
            tokio::select! {
                result = &mut compile => return result.map_err(WorkerError::Compiler),
                _ = ticker.tick() => self.queue.renew_lease(job.id, self.worker_id).await?,
            }
        }
    }

    async fn handle_execution(
        &self,
        job: CompileJobRecordV1,
        execution: WorkerExecution,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        if execution.compile_key != job.compile_key
            || execution.environment_id != job.tex_environment_id
            || execution.latexmk_profile != job.latexmk_profile
        {
            return self
                .infrastructure_failure(
                    job.id,
                    WorkerError::Manifest {
                        message: "compiler result does not match durable compile identity"
                            .to_owned(),
                    },
                )
                .await;
        }
        match execution.status {
            CompileStatus::Succeeded => {
                let mut artifacts = Vec::with_capacity(execution.artifacts.len());
                for artifact in &execution.artifacts {
                    let stored = match self.blobs.put(artifact.bytes.clone()).await {
                        Ok(value) => value,
                        Err(error) => {
                            return self
                                .infrastructure_failure(job.id, WorkerError::Blob(error))
                                .await;
                        }
                    };
                    let metadata = match self.blobs.metadata(stored.hash()).await {
                        Ok(value) => value,
                        Err(error) => {
                            return self
                                .infrastructure_failure(job.id, WorkerError::Blob(error))
                                .await;
                        }
                    };
                    if metadata.hash() != stored.hash()
                        || metadata.size_bytes() != stored.size_bytes()
                    {
                        return self
                            .infrastructure_failure(
                                job.id,
                                WorkerError::Manifest {
                                    message: "blob store did not validate persisted artifact"
                                        .to_owned(),
                                },
                            )
                            .await;
                    }
                    artifacts.push(PersistedArtifactV1 {
                        artifact_id: ArtifactId::new(),
                        kind: artifact.kind,
                        logical_name: artifact.logical_name.clone(),
                        blob_hash: stored.hash(),
                        size_bytes: stored.size_bytes(),
                        content_type: content_type(&artifact.logical_name).to_owned(),
                    });
                }
                let references = artifacts
                    .iter()
                    .map(|artifact| ArtifactRefV1 {
                        artifact_id: artifact.artifact_id,
                        kind: artifact.kind,
                        logical_name: artifact.logical_name.clone(),
                        blob_hash: artifact.blob_hash,
                        size_bytes: artifact.size_bytes,
                    })
                    .collect();
                let manifest =
                    ArtifactManifestV1::new(job.compile_key, references).map_err(|error| {
                        WorkerError::Manifest {
                            message: error.to_string(),
                        }
                    })?;
                let bytes =
                    manifest
                        .canonical_json_bytes()
                        .map_err(|error| WorkerError::Manifest {
                            message: error.to_string(),
                        })?;
                let hash = BlobHash::digest(&bytes);
                let stored = match self.blobs.put_verified(hash, Bytes::from(bytes)).await {
                    Ok(value) => value,
                    Err(error) => {
                        return self
                            .infrastructure_failure(job.id, WorkerError::Blob(error))
                            .await;
                    }
                };
                if stored.hash() != hash {
                    return self
                        .infrastructure_failure(
                            job.id,
                            WorkerError::Manifest {
                                message: "artifact manifest hash changed during persistence"
                                    .to_owned(),
                            },
                        )
                        .await;
                }
                match self
                    .queue
                    .complete_success(job.id, self.worker_id, &artifacts, hash)
                    .await?
                {
                    CompletionOutcome::Succeeded => Ok(WorkerRunOutcome::Compiled),
                    CompletionOutcome::Cancelled => Ok(WorkerRunOutcome::Cancelled),
                }
            }
            CompileStatus::Failed => {
                self.compile_execution_failure(job.id, false, &execution)
                    .await
            }
            CompileStatus::TimedOut => {
                self.compile_execution_failure(job.id, true, &execution)
                    .await
            }
        }
    }

    async fn compile_execution_failure(
        &self,
        job: core_types::JobId,
        timed_out: bool,
        execution: &WorkerExecution,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        let mut artifacts = Vec::new();
        for artifact in execution
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == core_types::ArtifactKind::Log)
        {
            let stored = self.blobs.put(artifact.bytes.clone()).await?;
            artifacts.push(PersistedArtifactV1 {
                artifact_id: ArtifactId::new(),
                kind: artifact.kind,
                logical_name: artifact.logical_name.clone(),
                blob_hash: stored.hash(),
                size_bytes: stored.size_bytes(),
                content_type: content_type(&artifact.logical_name).to_owned(),
            });
        }
        let error =
            serde_json::json!({"class":"compile", "message":compile_result_error(execution)});
        match self
            .queue
            .complete_compile_failure_with_artifacts(
                job,
                self.worker_id,
                timed_out,
                error,
                &artifacts,
            )
            .await?
        {
            CompletionOutcome::Succeeded => Ok(if timed_out {
                WorkerRunOutcome::TimedOut
            } else {
                WorkerRunOutcome::CompileFailed
            }),
            CompletionOutcome::Cancelled => Ok(WorkerRunOutcome::Cancelled),
        }
    }

    async fn compile_failure(
        &self,
        job: core_types::JobId,
        timed_out: bool,
        message: String,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        let error = serde_json::json!({"class":"compile", "message":message});
        match self
            .queue
            .complete_compile_failure(job, self.worker_id, timed_out, error)
            .await?
        {
            CompletionOutcome::Succeeded => Ok(if timed_out {
                WorkerRunOutcome::TimedOut
            } else {
                WorkerRunOutcome::CompileFailed
            }),
            CompletionOutcome::Cancelled => Ok(WorkerRunOutcome::Cancelled),
        }
    }
    async fn infrastructure_failure(
        &self,
        job: core_types::JobId,
        error: WorkerError,
    ) -> Result<WorkerRunOutcome, WorkerError> {
        let value = serde_json::json!({"class":"infrastructure", "message":error.to_string()});
        match self
            .queue
            .complete_infrastructure_failure(job, self.worker_id, value)
            .await?
        {
            InfrastructureOutcome::Requeued => Ok(WorkerRunOutcome::RequeuedInfrastructure),
            InfrastructureOutcome::Failed => Ok(WorkerRunOutcome::FailedInfrastructure),
            InfrastructureOutcome::Cancelled => Ok(WorkerRunOutcome::Cancelled),
        }
    }
}

fn infrastructure_compiler_error(error: &CompilerError) -> bool {
    matches!(
        error,
        CompilerError::DockerUnavailable { .. }
            | CompilerError::DockerCommandFailed { .. }
            | CompilerError::ImageNotFound { .. }
            | CompilerError::EnvironmentMismatch { .. }
            | CompilerError::ContainerInfrastructure { .. }
            | CompilerError::BlockingTaskFailed { .. }
            | CompilerError::Io { .. }
            | CompilerError::InternalInvariant { .. }
    )
}
fn compile_result_error(execution: &WorkerExecution) -> String {
    format!(
        "compiler exited {:?}; stdout_truncated={}; stderr_truncated={}",
        execution.exit_code, execution.stdout_truncated, execution.stderr_truncated
    )
}
fn content_type(path: &LogicalPath) -> &'static str {
    match path
        .file_name()
        .rsplit_once('.')
        .map(|(_, extension)| extension)
    {
        Some("pdf") => "application/pdf",
        Some("log") => "text/plain; charset=utf-8",
        Some("synctex" | "gz") => "application/gzip",
        _ => "application/octet-stream",
    }
}

fn needs_backoff<T>(result: &Result<WorkerRunOutcome, T>) -> bool {
    matches!(result, Ok(WorkerRunOutcome::Idle) | Err(_))
}

#[cfg(test)]
mod tests {
    use super::{WorkerRunOutcome, needs_backoff};

    #[test]
    fn idle_and_repeated_errors_always_select_backoff() {
        assert!(needs_backoff(&Ok::<_, ()>(WorkerRunOutcome::Idle)));
        assert!(needs_backoff(&Err::<WorkerRunOutcome, _>(())));
        assert!(needs_backoff(&Err::<WorkerRunOutcome, _>(())));
        assert!(!needs_backoff(&Ok::<_, ()>(WorkerRunOutcome::Compiled)));
    }
}
