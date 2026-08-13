#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "database integration fixtures"
)]

use async_trait::async_trait;
use blob_store::{
    BlobMetadata, BlobStore, BlobStoreError, BlobStoreMaintenance, FsBlobStore, FsBlobStoreConfig,
    PutResult,
};
use bytes::Bytes;
use compiler::{CompileStatus, CompilerError};
use core_types::{
    ArtifactKind, BlobHash, CompileKey, CostClass, FileEntryV1, IdempotencyKey, JobId,
    LatexmkProfileId, LogicalPath, ShellPolicy, SnapshotId, TenantId, TexEngine, TexEnvironmentId,
    UserId, WorkerId, WorkspaceId, WorkspaceManifestV1,
};
use persistence::{
    Database, DatabaseConfig, EnqueueCompileJobV1, PostgresCompileQueue, QueueLimits,
};
use queue::{
    CompilationWorker, CompileExecutor, WorkerArtifact, WorkerConfig, WorkerExecution,
    WorkerRunOutcome, WorkerShutdown,
};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    env,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::Semaphore;

struct Fixture {
    queue: PostgresCompileQueue,
    pool: PgPool,
    store: Arc<FsBlobStore>,
    _directory: TempDir,
    tenant: TenantId,
    user: UserId,
    workspace: WorkspaceId,
    snapshot: SnapshotId,
}

async fn fixture(limits: QueueLimits) -> Fixture {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(&url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let tenant = TenantId::new();
    let user = UserId::new();
    let workspace = WorkspaceId::new();
    sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
        .bind(tenant.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user.as_uuid())
        .bind(tenant.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(tenant.as_uuid())
        .bind(user.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let directory = TempDir::new().unwrap();
    let store = Arc::new(
        FsBlobStore::open(directory.path(), FsBlobStoreConfig::development_default())
            .await
            .unwrap(),
    );
    let main = LogicalPath::parse(&format!("main-{workspace}.tex")).unwrap();
    let source = Bytes::from(format!("\\documentclass{{article}} % {workspace}"));
    let source_hash = BlobHash::digest(&source);
    store
        .put_verified(source_hash, source.clone())
        .await
        .unwrap();
    let manifest = WorkspaceManifestV1::new(
        main.clone(),
        BTreeMap::from([(
            main,
            FileEntryV1 {
                blob_hash: source_hash,
                size_bytes: u64::try_from(source.len()).unwrap(),
            },
        )]),
    )
    .unwrap();
    let bytes = manifest.canonical_json_bytes().unwrap();
    let snapshot = manifest.snapshot_id().unwrap();
    let hash = BlobHash::digest(&bytes);
    store.put_verified(hash, Bytes::from(bytes)).await.unwrap();
    sqlx::query("INSERT INTO latex_core.snapshots (snapshot_id,manifest_blob_hash) VALUES ($1,$2)")
        .bind(snapshot.to_hex())
        .bind(hash.to_hex())
        .execute(&pool)
        .await
        .unwrap();
    let queue = PostgresCompileQueue::new(database, limits);
    recover_abandoned_jobs(&queue, &pool).await;
    Fixture {
        queue,
        pool,
        store,
        _directory: directory,
        tenant,
        user,
        workspace,
        snapshot,
    }
}

async fn recover_abandoned_jobs(queue: &PostgresCompileQueue, pool: &PgPool) {
    queue.recover_expired_leases().await.unwrap();
    let abandoned: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM latex_core.compile_jobs WHERE state='queued'")
            .fetch_all(pool)
            .await
            .unwrap();
    for job_id in abandoned {
        queue
            .request_cancellation(JobId::from_uuid(job_id), Some("abandoned integration test"))
            .await
            .unwrap();
    }
}
fn limits(attempts: u32) -> QueueLimits {
    QueueLimits::new(3, 3, 6, Duration::from_secs(60), attempts).unwrap()
}
fn job(fixture: &Fixture, suffix: &str, key: CompileKey) -> EnqueueCompileJobV1 {
    EnqueueCompileJobV1 {
        job_id: JobId::new(),
        tenant_id: fixture.tenant,
        user_id: fixture.user,
        workspace_id: fixture.workspace,
        snapshot_id: fixture.snapshot,
        compile_key: key,
        idempotency_key: IdempotencyKey::parse(&format!("worker-{}-{suffix}", fixture.workspace))
            .unwrap(),
        engine: TexEngine::PdfLatex,
        tex_environment_id: TexEnvironmentId::parse("texlive-2026+full@1").unwrap(),
        latexmk_profile: LatexmkProfileId::parse("safe-v1").unwrap(),
        shell_policy: ShellPolicy::Safe,
        synctex: true,
        cost_class: CostClass::Normal,
        priority: 32_767,
    }
}
fn key(fixture: &Fixture, seed: &str) -> CompileKey {
    BlobHash::digest(format!("{}-{seed}", fixture.workspace).as_bytes())
        .to_string()
        .parse()
        .unwrap()
}
async fn state(fixture: &Fixture, id: JobId) -> String {
    sqlx::query_scalar("SELECT state FROM latex_core.compile_jobs WHERE id=$1")
        .bind(id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap()
}
async fn assert_no_running_jobs(fixture: &Fixture) {
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.compile_jobs WHERE tenant_id=$1 AND state='running'",
    )
    .bind(fixture.tenant.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(running, 0);
}

async fn cancel_queued_jobs(fixture: &Fixture) {
    let queued: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM latex_core.compile_jobs WHERE tenant_id=$1 AND state='queued'",
    )
    .bind(fixture.tenant.as_uuid())
    .fetch_all(&fixture.pool)
    .await
    .unwrap();
    for job_id in queued {
        fixture
            .queue
            .request_cancellation(JobId::from_uuid(job_id), Some("test cleanup"))
            .await
            .unwrap();
    }
}

#[derive(Copy, Clone)]
enum Mode {
    Success,
    Failed,
    TimedOut,
    Infrastructure,
    Blocked,
}
struct FakeExecutor {
    mode: Mutex<Mode>,
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
}
impl FakeExecutor {
    fn new(mode: Mode) -> Self {
        Self {
            mode: Mutex::new(mode),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            entered: Arc::new(Semaphore::new(0)),
            release: Arc::new(Semaphore::new(0)),
        }
    }
    fn execution(job: &persistence::CompileJobRecordV1, status: CompileStatus) -> WorkerExecution {
        WorkerExecution {
            status,
            compile_key: job.compile_key,
            environment_id: job.tex_environment_id.clone(),
            latexmk_profile: job.latexmk_profile.clone(),
            artifacts: vec![
                WorkerArtifact {
                    kind: ArtifactKind::Pdf,
                    logical_name: LogicalPath::parse("output.pdf").unwrap(),
                    bytes: Bytes::from_static(b"pdf"),
                },
                WorkerArtifact {
                    kind: ArtifactKind::Log,
                    logical_name: LogicalPath::parse("output.log").unwrap(),
                    bytes: Bytes::from_static(b"log"),
                },
            ],
            exit_code: Some(if status == CompileStatus::Succeeded {
                0
            } else {
                1
            }),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }
}
#[async_trait]
impl CompileExecutor for FakeExecutor {
    async fn compile_job(
        &self,
        job: &persistence::CompileJobRecordV1,
        _manifest: &WorkspaceManifestV1,
    ) -> Result<WorkerExecution, CompilerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        let mode = *self.mode.lock().unwrap();
        if matches!(mode, Mode::Blocked) {
            self.entered.add_permits(1);
            let permit = self.release.acquire().await.unwrap();
            drop(permit);
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        match mode {
            Mode::Success | Mode::Blocked => Ok(Self::execution(job, CompileStatus::Succeeded)),
            Mode::Failed => Ok(Self::execution(job, CompileStatus::Failed)),
            Mode::TimedOut => Ok(Self::execution(job, CompileStatus::TimedOut)),
            Mode::Infrastructure => Err(CompilerError::ContainerInfrastructure {
                message: "fake runtime".to_owned(),
            }),
        }
    }
}

struct FailingPutStore {
    inner: Arc<dyn BlobStore>,
    fail: AtomicBool,
}
#[async_trait]
impl BlobStore for FailingPutStore {
    async fn put(&self, bytes: Bytes) -> Result<PutResult, BlobStoreError> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(BlobStoreError::InvalidConfiguration {
                message: "injected put failure".to_owned(),
            });
        }
        self.inner.put(bytes).await
    }
    async fn put_verified(
        &self,
        expected: BlobHash,
        bytes: Bytes,
    ) -> Result<PutResult, BlobStoreError> {
        self.inner.put_verified(expected, bytes).await
    }
    async fn get(&self, hash: BlobHash) -> Result<Bytes, BlobStoreError> {
        self.inner.get(hash).await
    }
    async fn exists(&self, hash: BlobHash) -> Result<bool, BlobStoreError> {
        self.inner.exists(hash).await
    }
    async fn metadata(&self, hash: BlobHash) -> Result<BlobMetadata, BlobStoreError> {
        self.inner.metadata(hash).await
    }
}
fn worker<E: CompileExecutor>(
    fixture: &Fixture,
    executor: Arc<E>,
    blobs: Arc<dyn BlobStore>,
) -> CompilationWorker<E> {
    CompilationWorker::new(
        fixture.queue.clone(),
        blobs,
        executor,
        WorkerId::new(),
        WorkerConfig::new(2, Duration::from_millis(10)).unwrap(),
    )
}

#[tokio::test]
async fn worker_success_persists_artifacts_and_cache() {
    let fixture = fixture(limits(2)).await;
    let request = job(&fixture, "success", key(&fixture, "success"));
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Success));
    assert_eq!(
        worker(&fixture, executor, fixture.store.clone())
            .run_once()
            .await
            .unwrap(),
        WorkerRunOutcome::Compiled
    );
    assert_eq!(state(&fixture, request.job_id).await, "succeeded");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compilation_artifacts WHERE job_id=$1"
        )
        .bind(request.job_id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        2
    );
    assert!(
        fixture
            .queue
            .cache_entry(request.compile_key)
            .await
            .unwrap()
            .is_some()
    );
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_compile_failure_and_timeout_are_terminal_without_cache() {
    for (suffix, mode, expected) in [
        ("failed", Mode::Failed, "failed"),
        ("timeout", Mode::TimedOut, "timed_out"),
    ] {
        let fixture = fixture(limits(2)).await;
        let request = job(&fixture, suffix, key(&fixture, suffix));
        fixture.queue.enqueue(request.clone()).await.unwrap();
        let executor = Arc::new(FakeExecutor::new(mode));
        worker(&fixture, executor, fixture.store.clone())
            .run_once()
            .await
            .unwrap();
        assert_eq!(state(&fixture, request.job_id).await, expected);
        assert!(
            fixture
                .queue
                .cache_entry(request.compile_key)
                .await
                .unwrap()
                .is_none()
        );
        assert_no_running_jobs(&fixture).await;
    }
}

#[tokio::test]
async fn worker_infrastructure_error_retries_then_exhausts() {
    let fixture = fixture(limits(2)).await;
    let request = job(&fixture, "infra", key(&fixture, "infra"));
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Infrastructure));
    let worker = worker(&fixture, executor.clone(), fixture.store.clone());
    assert_eq!(
        worker.run_once().await.unwrap(),
        WorkerRunOutcome::RequeuedInfrastructure
    );
    assert_eq!(state(&fixture, request.job_id).await, "queued");
    assert_eq!(
        worker.run_once().await.unwrap(),
        WorkerRunOutcome::FailedInfrastructure
    );
    assert_eq!(state(&fixture, request.job_id).await, "failed");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
    assert_no_running_jobs(&fixture).await;
    cancel_queued_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_cancellation_wins_before_success_publication() {
    let fixture = fixture(limits(2)).await;
    let request = job(&fixture, "cancel", key(&fixture, "cancel"));
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Blocked));
    let worker = worker(&fixture, executor.clone(), fixture.store.clone());
    let task = tokio::spawn(async move { worker.run_once().await.unwrap() });
    let permit = executor.entered.acquire().await.unwrap();
    drop(permit);
    fixture
        .queue
        .request_cancellation(request.job_id, Some("test cancellation"))
        .await
        .unwrap();
    executor.release.add_permits(1);
    assert_eq!(task.await.unwrap(), WorkerRunOutcome::Cancelled);
    assert_eq!(state(&fixture, request.job_id).await, "cancelled");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compilation_artifacts WHERE job_id=$1"
        )
        .bind(request.job_id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_cache_hit_uses_fresh_destination_metadata() {
    let fixture = fixture(limits(2)).await;
    let compile_key = key(&fixture, "shared-cache");
    let first = job(&fixture, "cache-one", compile_key);
    fixture.queue.enqueue(first.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Success));
    let worker = worker(&fixture, executor.clone(), fixture.store.clone());
    worker.run_once().await.unwrap();
    let second = job(&fixture, "cache-two", compile_key);
    fixture.queue.enqueue(second.clone()).await.unwrap();
    assert_eq!(worker.run_once().await.unwrap(), WorkerRunOutcome::CacheHit);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let source: uuid::Uuid = sqlx::query_scalar(
        "SELECT artifact_id FROM latex_core.compilation_artifacts WHERE job_id=$1 LIMIT 1",
    )
    .bind(first.job_id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    let destination: uuid::Uuid = sqlx::query_scalar(
        "SELECT artifact_id FROM latex_core.compilation_artifacts WHERE job_id=$1 LIMIT 1",
    )
    .bind(second.job_id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_ne!(source, destination);
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_stale_cache_is_evicted_and_replaced_after_compile() {
    let fixture = fixture(limits(2)).await;
    let compile_key = key(&fixture, "stale-cache");
    let first = job(&fixture, "stale-one", compile_key);
    fixture.queue.enqueue(first.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Success));
    let worker = worker(&fixture, executor.clone(), fixture.store.clone());
    worker.run_once().await.unwrap();
    let stale = fixture
        .queue
        .cache_entry(compile_key)
        .await
        .unwrap()
        .unwrap();
    fixture
        .store
        .delete(stale.artifact_manifest_blob_hash)
        .await
        .unwrap();
    let second = job(&fixture, "stale-two", compile_key);
    fixture.queue.enqueue(second).await.unwrap();
    assert_eq!(worker.run_once().await.unwrap(), WorkerRunOutcome::Compiled);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
    assert_ne!(
        fixture
            .queue
            .cache_entry(compile_key)
            .await
            .unwrap()
            .unwrap()
            .artifact_manifest_blob_hash,
        stale.artifact_manifest_blob_hash
    );
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_blob_failure_never_marks_success_or_creates_cache() {
    let fixture = fixture(limits(2)).await;
    let request = job(&fixture, "blob-failure", key(&fixture, "blob-failure"));
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let executor = Arc::new(FakeExecutor::new(Mode::Success));
    let blobs: Arc<dyn BlobStore> = Arc::new(FailingPutStore {
        inner: fixture.store.clone(),
        fail: AtomicBool::new(true),
    });
    assert_eq!(
        worker(&fixture, executor, blobs).run_once().await.unwrap(),
        WorkerRunOutcome::RequeuedInfrastructure
    );
    assert_eq!(state(&fixture, request.job_id).await, "queued");
    assert!(
        fixture
            .queue
            .cache_entry(request.compile_key)
            .await
            .unwrap()
            .is_none()
    );
    assert_no_running_jobs(&fixture).await;
    cancel_queued_jobs(&fixture).await;
}

#[tokio::test]
async fn worker_service_never_exceeds_configured_concurrency() {
    let fixture = fixture(limits(2)).await;
    for suffix in ["concurrent-one", "concurrent-two", "concurrent-three"] {
        let request = job(&fixture, suffix, key(&fixture, suffix));
        fixture.queue.enqueue(request).await.unwrap();
    }
    let executor = Arc::new(FakeExecutor::new(Mode::Blocked));
    let worker = worker(&fixture, executor.clone(), fixture.store.clone());
    let shutdown = WorkerShutdown::new();
    let stop = shutdown.clone();
    let service = tokio::spawn(async move { worker.run_until_shutdown(stop).await.unwrap() });
    let permits = executor.entered.acquire_many(2).await.unwrap();
    drop(permits);
    assert_eq!(executor.max_active.load(Ordering::SeqCst), 2);
    shutdown.request();
    executor.release.add_permits(2);
    service.await.unwrap();
    assert!(executor.max_active.load(Ordering::SeqCst) <= 2);
    assert_no_running_jobs(&fixture).await;
    cancel_queued_jobs(&fixture).await;
}
