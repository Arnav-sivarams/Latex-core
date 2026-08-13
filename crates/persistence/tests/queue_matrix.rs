#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "database integration fixtures"
)]

use core_types::{
    ArtifactId, ArtifactKind, BlobHash, CompileKey, CostClass, IdempotencyKey, JobId,
    LatexmkProfileId, LogicalPath, ShellPolicy, SnapshotId, TenantId, TexEngine, TexEnvironmentId,
    UserId, WorkerId, WorkspaceId,
};
use persistence::{
    CompletionOutcome, EnqueueCompileJobV1, InfrastructureOutcome, PersistedArtifactV1,
    PostgresCompileQueue, QueueError, QueueLimits,
};
use serde_json::json;
use sqlx::PgPool;
use std::{env, time::Duration};
use tokio::sync::Barrier;
use uuid::Uuid;

struct Fixture {
    queue: PostgresCompileQueue,
    pool: PgPool,
    tenant: TenantId,
    user: UserId,
    workspace: WorkspaceId,
    snapshot: SnapshotId,
}

async fn fixture(limits: QueueLimits) -> Fixture {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database =
        persistence::Database::connect(persistence::DatabaseConfig::development(&url).unwrap())
            .await
            .unwrap();
    database.migrate().await.unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let tenant = TenantId::new();
    let user = UserId::new();
    let workspace = WorkspaceId::new();
    let snapshot: SnapshotId = hash(format!("fixture-snapshot-{workspace}").as_bytes())
        .parse()
        .unwrap();
    insert_identity(&pool, tenant, user, workspace).await;
    sqlx::query("INSERT INTO latex_core.snapshots (snapshot_id,manifest_blob_hash) VALUES ($1,$2)")
        .bind(snapshot.to_hex())
        .bind(hash(format!("fixture-manifest-{workspace}").as_bytes()))
        .execute(&pool)
        .await
        .unwrap();
    let queue = PostgresCompileQueue::new(database, limits);
    recover_abandoned_jobs(&queue, &pool).await;
    Fixture {
        queue,
        pool,
        tenant,
        user,
        workspace,
        snapshot,
    }
}

async fn recover_abandoned_jobs(queue: &PostgresCompileQueue, pool: &PgPool) {
    queue.recover_expired_leases().await.unwrap();
    let abandoned: Vec<Uuid> =
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

async fn insert_identity(pool: &PgPool, tenant: TenantId, user: UserId, workspace: WorkspaceId) {
    sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user.as_uuid())
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(tenant.as_uuid())
        .bind(user.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

async fn insert_user_workspace(
    pool: &PgPool,
    tenant: TenantId,
    user: UserId,
    workspace: WorkspaceId,
) {
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user.as_uuid())
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(tenant.as_uuid())
        .bind(user.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

fn limits(global: u32, user_running: u32, outstanding: u32, attempts: u32) -> QueueLimits {
    QueueLimits::new(
        global,
        user_running,
        outstanding,
        Duration::from_secs(60),
        attempts,
    )
    .unwrap()
}
fn hash(bytes: &[u8]) -> String {
    BlobHash::digest(bytes).to_hex()
}
fn request(fixture: &Fixture, suffix: &str, priority: i16) -> EnqueueCompileJobV1 {
    EnqueueCompileJobV1 {
        job_id: JobId::new(),
        tenant_id: fixture.tenant,
        user_id: fixture.user,
        workspace_id: fixture.workspace,
        snapshot_id: fixture.snapshot,
        compile_key: hash(format!("key-{}-{suffix}", fixture.workspace).as_bytes())
            .parse::<CompileKey>()
            .unwrap(),
        idempotency_key: IdempotencyKey::parse(&format!(
            "idempotency-{}-{suffix}",
            fixture.workspace
        ))
        .unwrap(),
        engine: TexEngine::PdfLatex,
        tex_environment_id: TexEnvironmentId::parse("texlive-2026+full@1").unwrap(),
        latexmk_profile: LatexmkProfileId::parse("safe-v1").unwrap(),
        shell_policy: ShellPolicy::Safe,
        synctex: true,
        cost_class: CostClass::Normal,
        priority,
    }
}
fn artifact(suffix: &str) -> PersistedArtifactV1 {
    PersistedArtifactV1 {
        artifact_id: ArtifactId::new(),
        kind: ArtifactKind::Pdf,
        logical_name: LogicalPath::parse(&format!("{suffix}.pdf")).unwrap(),
        blob_hash: hash(format!("blob-{suffix}").as_bytes()).parse().unwrap(),
        size_bytes: 3,
        content_type: "application/pdf".to_owned(),
    }
}
async fn state(pool: &PgPool, job: JobId) -> String {
    sqlx::query_scalar("SELECT state FROM latex_core.compile_jobs WHERE id=$1")
        .bind(job.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn finish_test_claim(fixture: &Fixture, job: JobId, worker: WorkerId) {
    fixture
        .queue
        .complete_compile_failure(job, worker, false, json!({"class":"test-cleanup"}))
        .await
        .unwrap();
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
    let queued: Vec<Uuid> = sqlx::query_scalar(
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

#[tokio::test]
async fn enqueue_persists_queued_state_and_reuses_matching_idempotency_key() {
    let fixture = fixture(limits(3, 2, 3, 2)).await;
    let request = request(&fixture, "enqueue", 7);
    assert_eq!(
        fixture.queue.enqueue(request.clone()).await.unwrap(),
        request.job_id
    );
    assert_eq!(
        fixture.queue.enqueue(request.clone()).await.unwrap(),
        request.job_id
    );
    let row: (String, i16, i32) = sqlx::query_as(
        "SELECT state,priority,attempt_count FROM latex_core.compile_jobs WHERE id=$1",
    )
    .bind(request.job_id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(row, ("queued".to_owned(), 7, 0));
    let mut conflicting = request;
    conflicting.compile_key = hash(b"different").parse().unwrap();
    assert!(matches!(
        fixture.queue.enqueue(conflicting).await,
        Err(QueueError::IdempotencyConflict)
    ));
    cancel_queued_jobs(&fixture).await;
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn claim_orders_priority_fifo_and_stable_id() {
    let fixture = fixture(limits(4, 4, 4, 2)).await;
    let low = request(&fixture, "low", 32_766);
    let first = request(&fixture, "first", 32_767);
    let second = request(&fixture, "second", 32_767);
    for item in [&low, &first, &second] {
        fixture.queue.enqueue((*item).clone()).await.unwrap();
    }
    sqlx::query("UPDATE latex_core.compile_jobs SET created_at=TIMESTAMPTZ '2020-01-01T00:00:00Z' WHERE id IN ($1,$2)").bind(first.job_id.as_uuid()).bind(second.job_id.as_uuid()).execute(&fixture.pool).await.unwrap();
    let expected = if first.job_id < second.job_id {
        first.job_id
    } else {
        second.job_id
    };
    let first_worker = WorkerId::new();
    let first_claim = fixture.queue.claim(first_worker).await.unwrap().unwrap();
    assert_eq!(first_claim.id, expected);
    let next_worker = WorkerId::new();
    let next_claim = fixture.queue.claim(next_worker).await.unwrap().unwrap();
    let next = next_claim.id;
    assert_eq!(
        next,
        if expected == first.job_id {
            second.job_id
        } else {
            first.job_id
        }
    );
    let low_worker = WorkerId::new();
    let low_claim = fixture.queue.claim(low_worker).await.unwrap().unwrap();
    assert_eq!(low_claim.id, low.job_id);
    finish_test_claim(&fixture, first_claim.id, first_worker).await;
    finish_test_claim(&fixture, next_claim.id, next_worker).await;
    finish_test_claim(&fixture, low_claim.id, low_worker).await;
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn concurrent_claim_has_exactly_one_winner() {
    let fixture = fixture(limits(2, 2, 2, 2)).await;
    let request = request(&fixture, "concurrent", 32_767);
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let barrier = std::sync::Arc::new(Barrier::new(3));
    let left = fixture.queue.clone();
    let left_barrier = barrier.clone();
    let right = fixture.queue.clone();
    let right_barrier = barrier.clone();
    let left_worker = WorkerId::new();
    let a = tokio::spawn(async move {
        left_barrier.wait().await;
        (left_worker, left.claim(left_worker).await.unwrap())
    });
    let right_worker = WorkerId::new();
    let b = tokio::spawn(async move {
        right_barrier.wait().await;
        (right_worker, right.claim(right_worker).await.unwrap())
    });
    barrier.wait().await;
    let claimed = [a.await.unwrap(), b.await.unwrap()]
        .into_iter()
        .filter_map(|(worker, job)| job.map(|job| (worker, job)))
        .collect::<Vec<_>>();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].1.id, request.job_id);
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.compile_jobs WHERE id=$1 AND state='running'",
    )
    .bind(request.job_id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(active, 1);
    finish_test_claim(&fixture, claimed[0].1.id, claimed[0].0).await;
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn global_and_per_user_caps_are_enforced_with_different_users() {
    let fixture = fixture(limits(2, 1, 2, 2)).await;
    let first = request(&fixture, "cap-first", 32_767);
    fixture.queue.enqueue(first.clone()).await.unwrap();
    let first_worker = WorkerId::new();
    let first_claim = fixture.queue.claim(first_worker).await.unwrap().unwrap();
    assert_eq!(first_claim.id, first.job_id);
    let same_user = request(&fixture, "cap-same-user", 32_767);
    fixture.queue.enqueue(same_user.clone()).await.unwrap();
    assert!(matches!(
        fixture
            .queue
            .enqueue(request(&fixture, "cap-overflow", 32_767))
            .await,
        Err(QueueError::AdmissionRejected { .. })
    ));
    let other_user = UserId::new();
    let other_workspace = WorkspaceId::new();
    insert_user_workspace(&fixture.pool, fixture.tenant, other_user, other_workspace).await;
    let mut other = request(&fixture, "cap-other", 32_767);
    other.user_id = other_user;
    other.workspace_id = other_workspace;
    fixture.queue.enqueue(other.clone()).await.unwrap();
    let other_worker = WorkerId::new();
    let other_claim = fixture.queue.claim(other_worker).await.unwrap().unwrap();
    assert_eq!(
        other_claim.id, other.job_id,
        "per-user running cap skips the first user's second job"
    );
    let third_user = UserId::new();
    let third_workspace = WorkspaceId::new();
    insert_user_workspace(&fixture.pool, fixture.tenant, third_user, third_workspace).await;
    let mut blocked_by_global = request(&fixture, "cap-global", 32_767);
    blocked_by_global.user_id = third_user;
    blocked_by_global.workspace_id = third_workspace;
    fixture
        .queue
        .enqueue(blocked_by_global.clone())
        .await
        .unwrap();
    assert!(
        fixture
            .queue
            .claim(WorkerId::new())
            .await
            .unwrap()
            .is_none(),
        "global cap blocks another user"
    );
    finish_test_claim(&fixture, first_claim.id, first_worker).await;
    finish_test_claim(&fixture, other_claim.id, other_worker).await;
    cancel_queued_jobs(&fixture).await;
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn lease_renewal_stale_completion_and_expiry_recovery_are_safe() {
    let fixture = fixture(limits(2, 2, 3, 2)).await;
    let request = request(&fixture, "lease", 32_767);
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let owner = WorkerId::new();
    let claimed = fixture.queue.claim(owner).await.unwrap().unwrap();
    fixture.queue.renew_lease(claimed.id, owner).await.unwrap();
    assert!(matches!(
        fixture.queue.renew_lease(claimed.id, WorkerId::new()).await,
        Err(QueueError::LeaseLost)
    ));
    assert!(matches!(
        fixture
            .queue
            .complete_success(
                claimed.id,
                WorkerId::new(),
                &[artifact("stale")],
                hash(b"manifest").parse().unwrap()
            )
            .await,
        Err(QueueError::LeaseLost)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compilation_artifacts WHERE job_id=$1"
        )
        .bind(claimed.id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("UPDATE latex_core.compile_jobs SET lease_until=statement_timestamp() - interval '1 millisecond' WHERE id=$1").bind(claimed.id.as_uuid()).execute(&fixture.pool).await.unwrap();
    assert_eq!(fixture.queue.recover_expired_leases().await.unwrap(), 1);
    assert_eq!(state(&fixture.pool, claimed.id).await, "queued");
    let retry = fixture.queue.claim(owner).await.unwrap().unwrap();
    assert_eq!(retry.attempt_count, 2);
    sqlx::query("UPDATE latex_core.compile_jobs SET lease_until=statement_timestamp() - interval '1 millisecond' WHERE id=$1").bind(retry.id.as_uuid()).execute(&fixture.pool).await.unwrap();
    fixture.queue.recover_expired_leases().await.unwrap();
    assert_eq!(state(&fixture.pool, retry.id).await, "failed");
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn cancellation_and_terminal_transitions_have_coherent_winners() {
    let fixture = fixture(limits(3, 3, 3, 2)).await;
    let queued = request(&fixture, "queued-cancel", 32_767);
    fixture.queue.enqueue(queued.clone()).await.unwrap();
    fixture
        .queue
        .request_cancellation(queued.job_id, Some("cancel"))
        .await
        .unwrap();
    assert_eq!(state(&fixture.pool, queued.job_id).await, "cancelled");
    let running = request(&fixture, "running-cancel", 32_767);
    fixture.queue.enqueue(running.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    fixture
        .queue
        .request_cancellation(claim.id, Some("cancel first"))
        .await
        .unwrap();
    let requested: bool = sqlx::query_scalar(
        "SELECT cancellation_requested_at IS NOT NULL FROM latex_core.compile_jobs WHERE id=$1",
    )
    .bind(claim.id.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert!(requested);
    assert_eq!(
        fixture
            .queue
            .complete_success(
                claim.id,
                worker,
                &[artifact("not-published")],
                hash(b"not-published").parse().unwrap()
            )
            .await
            .unwrap(),
        CompletionOutcome::Cancelled
    );
    assert_eq!(state(&fixture.pool, claim.id).await, "cancelled");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compilation_artifacts WHERE job_id=$1"
        )
        .bind(claim.id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    let failed = request(&fixture, "failure", 32_767);
    fixture.queue.enqueue(failed.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    fixture
        .queue
        .complete_compile_failure(claim.id, worker, false, json!({"class":"compile"}))
        .await
        .unwrap();
    assert_eq!(state(&fixture.pool, claim.id).await, "failed");
    assert!(matches!(
        fixture
            .queue
            .complete_compile_failure(claim.id, worker, false, json!({}))
            .await,
        Err(QueueError::InvalidState { .. })
    ));
    let timeout = request(&fixture, "timeout", 32_767);
    fixture.queue.enqueue(timeout.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    fixture
        .queue
        .complete_compile_failure(claim.id, worker, true, json!({}))
        .await
        .unwrap();
    assert_eq!(state(&fixture.pool, claim.id).await, "timed_out");
}

#[tokio::test]
async fn cancellation_vs_claim_race_has_one_durable_winner() {
    let fixture = fixture(limits(2, 2, 2, 2)).await;
    let request = request(&fixture, "cancel-claim-race", 32_767);
    fixture.queue.enqueue(request.clone()).await.unwrap();
    let barrier = std::sync::Arc::new(Barrier::new(3));
    let claim_queue = fixture.queue.clone();
    let claim_barrier = barrier.clone();
    let cancel_queue = fixture.queue.clone();
    let cancel_barrier = barrier.clone();
    let worker = WorkerId::new();
    let claim = tokio::spawn(async move {
        claim_barrier.wait().await;
        (worker, claim_queue.claim(worker).await.unwrap())
    });
    let cancel = tokio::spawn(async move {
        cancel_barrier.wait().await;
        cancel_queue
            .request_cancellation(request.job_id, Some("race"))
            .await
    });
    barrier.wait().await;
    let (worker, claimed) = claim.await.unwrap();
    let cancelled = cancel.await.unwrap();
    let terminal = state(&fixture.pool, request.job_id).await;
    match (claimed, cancelled, terminal.as_str()) {
        (None, Ok(()), "cancelled") => {}
        (Some(job), Ok(()), "cancelled") if job.id != request.job_id => {
            finish_test_claim(&fixture, job.id, worker).await;
        }
        (Some(job), Ok(()), "running") => {
            assert_eq!(job.id, request.job_id);
            assert!(sqlx::query_scalar::<_, bool>("SELECT cancellation_requested_at IS NOT NULL FROM latex_core.compile_jobs WHERE id=$1").bind(request.job_id.as_uuid()).fetch_one(&fixture.pool).await.unwrap());
            assert_eq!(
                fixture
                    .queue
                    .complete_success(job.id, worker, &[], hash(b"race-cleanup").parse().unwrap())
                    .await
                    .unwrap(),
                CompletionOutcome::Cancelled
            );
        }
        other => panic!("incoherent cancellation/claim race result: {other:?}"),
    }
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn success_artifacts_cache_and_cache_reuse_are_job_scoped() {
    let fixture = fixture(limits(3, 3, 3, 2)).await;
    let source = request(&fixture, "cache-source", 32_767);
    fixture.queue.enqueue(source.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    let source_artifact = artifact("source");
    let manifest: BlobHash = hash(b"cache-manifest").parse().unwrap();
    fixture
        .queue
        .complete_success(claim.id, worker, &[source_artifact.clone()], manifest)
        .await
        .unwrap();
    assert_eq!(state(&fixture.pool, source.job_id).await, "succeeded");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.compilation_artifacts WHERE job_id=$1"
        )
        .bind(source.job_id.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    assert!(
        fixture
            .queue
            .cache_entry(source.compile_key)
            .await
            .unwrap()
            .is_some()
    );
    let mut destination = request(&fixture, "cache-destination", 32_767);
    destination.compile_key = source.compile_key;
    fixture.queue.enqueue(destination.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    let cache = fixture
        .queue
        .cache_entry(source.compile_key)
        .await
        .unwrap()
        .unwrap();
    let copied = PersistedArtifactV1 {
        artifact_id: ArtifactId::new(),
        ..source_artifact
    };
    fixture
        .queue
        .complete_cached_success(claim.id, worker, cache, &[copied.clone()])
        .await
        .unwrap();
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT artifact_id FROM latex_core.compilation_artifacts WHERE job_id IN ($1,$2) ORDER BY job_id,artifact_id").bind(source.job_id.as_uuid()).bind(destination.job_id.as_uuid()).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    fixture.queue.evict_cache_if_matches(cache).await.unwrap();
    assert!(
        fixture
            .queue
            .cache_entry(source.compile_key)
            .await
            .unwrap()
            .is_none()
    );
    assert_no_running_jobs(&fixture).await;
}

#[tokio::test]
async fn infrastructure_retries_but_compile_failures_and_timeouts_do_not() {
    let fixture = fixture(limits(3, 3, 3, 2)).await;
    let infra = request(&fixture, "infra", 32_767);
    fixture.queue.enqueue(infra.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    assert_eq!(
        fixture
            .queue
            .complete_infrastructure_failure(claim.id, worker, json!({}))
            .await
            .unwrap(),
        InfrastructureOutcome::Requeued
    );
    let retry = fixture.queue.claim(worker).await.unwrap().unwrap();
    assert_eq!(retry.attempt_count, 2);
    assert_eq!(
        fixture
            .queue
            .complete_infrastructure_failure(retry.id, worker, json!({}))
            .await
            .unwrap(),
        InfrastructureOutcome::Failed
    );
    let compile = request(&fixture, "compile", 32_767);
    fixture.queue.enqueue(compile.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claim = fixture.queue.claim(worker).await.unwrap().unwrap();
    fixture
        .queue
        .complete_compile_failure(claim.id, worker, false, json!({}))
        .await
        .unwrap();
    assert_no_running_jobs(&fixture).await;
}
