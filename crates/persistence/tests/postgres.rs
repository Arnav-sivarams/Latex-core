#![cfg(feature = "database-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test fixtures"
)]

use persistence::{Database, DatabaseConfig};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::{env, time::Duration};
use uuid::Uuid;

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MANIFEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const COMPILE: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const BLOB: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[tokio::test]
async fn production_schema_enforces_relational_contract() {
    let url = env::var("TEST_DATABASE_URL")
        .expect("database-tests requires TEST_DATABASE_URL; use ./scripts/test-db.sh");
    let config = DatabaseConfig::new(&url, 1, 4, Duration::from_secs(5)).unwrap();
    assert_eq!(config.min_connections(), 1);
    assert_eq!(config.max_connections(), 4);
    let database = Database::connect(config).await.unwrap();
    database.health_check().await.unwrap();
    database.migrate().await.unwrap();
    database.migrate().await.unwrap();
    database.health_check().await.unwrap();

    let pool = PgPool::connect(&url).await.unwrap();
    verify_session_initialization(&url).await;
    verify_schema(&pool).await;
    verify_empty(&pool).await;

    let tenant = Uuid::new_v4();
    let user = Uuid::new_v4();
    let workspace_a = Uuid::new_v4();
    insert_base(&pool, tenant, user, workspace_a).await;
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM latex_core.workspaces WHERE id = $1")
            .bind(workspace_a)
            .fetch_one(&pool)
            .await
            .unwrap(),
        workspace_a
    );

    let bad_workspace = sqlx::query(
        "INSERT INTO latex_core.workspaces (id, tenant_id, owner_user_id) VALUES ($1, $2, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;
    assert_constraint(bad_workspace);

    verify_events(&pool, workspace_a, user).await;
    let workspace_b = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO latex_core.workspaces (id, tenant_id, owner_user_id) VALUES ($1, $2, $3)",
    )
    .bind(workspace_b)
    .bind(tenant)
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace_b)
        .execute(&pool)
        .await
        .unwrap();
    verify_snapshots_and_heads(&pool, workspace_a, workspace_b).await;
    verify_jobs(&pool, tenant, user, workspace_a).await;
    verify_queue_indexes(&pool).await;
    verify_artifacts_and_cache(&pool, tenant, user, workspace_a).await;

    pool.close().await;
    database.close().await;
}

async fn verify_session_initialization(url: &str) {
    let config = DatabaseConfig::new(url, 1, 1, Duration::from_secs(5)).unwrap();
    let database = Database::connect(config).await.unwrap();
    database.health_check().await.unwrap();
    database.close().await;
}

async fn verify_schema(pool: &PgPool) {
    let schema: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = 'latex_core')")
        .fetch_one(pool).await.unwrap();
    assert!(schema);
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM information_schema.tables WHERE table_schema = 'latex_core' ORDER BY table_name")
        .fetch_all(pool).await.unwrap();
    let mut expected = vec![
        "compile_cache",
        "compile_jobs",
        "compilation_artifacts",
        "snapshots",
        "tenants",
        "users",
        "workspace_events",
        "workspace_heads",
        "workspace_snapshots",
        "workspaces",
    ];
    expected.sort_unstable();
    assert_eq!(tables, expected);
}

async fn verify_empty(pool: &PgPool) {
    for table in ["tenants", "users", "workspaces"] {
        let sql = format!("SELECT count(*) FROM latex_core.{table}");
        let count: i64 = sqlx::query_scalar(&sql).fetch_one(pool).await.unwrap();
        assert_eq!(count, 0, "migration inserted rows into {table}");
    }
}

async fn insert_base(pool: &PgPool, tenant: Uuid, user: Uuid, workspace: Uuid) {
    sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users (id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO latex_core.workspaces (id, tenant_id, owner_user_id) VALUES ($1, $2, $3)",
    )
    .bind(workspace)
    .bind(tenant)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace)
        .execute(pool)
        .await
        .unwrap();
    let row =
        sqlx::query("SELECT tenant_id, owner_user_id FROM latex_core.workspaces WHERE id = $1")
            .bind(workspace)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(row.get::<Uuid, _>("tenant_id"), tenant);
    assert_eq!(row.get::<Uuid, _>("owner_user_id"), user);
}

async fn verify_events(pool: &PgPool, workspace: Uuid, user: Uuid) {
    let insert = "INSERT INTO latex_core.workspace_events (workspace_id, sequence, event_id, base_version, event_type, event_schema_version, payload, created_by_user_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)";
    sqlx::query(insert)
        .bind(workspace)
        .bind(1_i64)
        .bind(Uuid::new_v4())
        .bind(0_i64)
        .bind("file_changed")
        .bind(1_i32)
        .bind(json!({"blob_hash": BLOB}))
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    assert_constraint(
        sqlx::query(insert)
            .bind(workspace)
            .bind(2_i64)
            .bind(Uuid::new_v4())
            .bind(0_i64)
            .bind("bad")
            .bind(1_i32)
            .bind(json!({}))
            .bind(user)
            .execute(pool)
            .await,
    );
    assert_constraint(
        sqlx::query(insert)
            .bind(workspace)
            .bind(2_i64)
            .bind(Uuid::new_v4())
            .bind(1_i64)
            .bind("bad")
            .bind(1_i32)
            .bind(json!([]))
            .bind(user)
            .execute(pool)
            .await,
    );
    assert_constraint(
        sqlx::query(insert)
            .bind(workspace)
            .bind(1_i64)
            .bind(Uuid::new_v4())
            .bind(0_i64)
            .bind("duplicate")
            .bind(1_i32)
            .bind(json!({}))
            .bind(user)
            .execute(pool)
            .await,
    );
}

async fn verify_snapshots_and_heads(pool: &PgPool, a: Uuid, b: Uuid) {
    sqlx::query(
        "INSERT INTO latex_core.snapshots (snapshot_id, manifest_blob_hash) VALUES ($1,$2)",
    )
    .bind(SNAPSHOT)
    .bind(MANIFEST)
    .execute(pool)
    .await
    .unwrap();
    for (workspace, version) in [(a, 1_i64), (b, 1_i64), (a, 2_i64)] {
        sqlx::query("INSERT INTO latex_core.workspace_snapshots (workspace_id, workspace_version, snapshot_id) VALUES ($1,$2,$3)").bind(workspace).bind(version).bind(SNAPSHOT).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=2, latest_snapshot_version=2 WHERE workspace_id=$1").bind(a).execute(pool).await.unwrap();
    assert_constraint(sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=1, latest_snapshot_version=2 WHERE workspace_id=$1").bind(a).execute(pool).await);
    assert_constraint(sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=3, latest_snapshot_version=3 WHERE workspace_id=$1").bind(a).execute(pool).await);
}

async fn insert_job(
    pool: &PgPool,
    tenant: Uuid,
    user: Uuid,
    workspace: Uuid,
    idempotency: &str,
    state: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let active = matches!(state, "claimed" | "running");
    let sql = if active {
        "INSERT INTO latex_core.compile_jobs (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,state,worker_id,lease_until) VALUES ($1,$2,$3,$4,$5,$6,$7,'pdflatex','texlive-2026+full@1','default-v1','safe',true,'normal',$8,$9,now())"
    } else {
        "INSERT INTO latex_core.compile_jobs (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,state,worker_id,lease_until) VALUES ($1,$2,$3,$4,$5,$6,$7,'pdflatex','texlive-2026+full@1','default-v1','safe',true,'normal',$8,$9,NULL)"
    };
    sqlx::query(sql)
        .bind(id)
        .bind(tenant)
        .bind(user)
        .bind(workspace)
        .bind(SNAPSHOT)
        .bind(COMPILE)
        .bind(idempotency)
        .bind(state)
        .bind(active.then(Uuid::new_v4))
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn verify_jobs(pool: &PgPool, tenant: Uuid, user: Uuid, workspace: Uuid) {
    insert_job(pool, tenant, user, workspace, "valid:queued", "queued").await;
    let invalid_sqls = [
        "engine='unknown'",
        "shell_policy='unknown'",
        "cost_class='unknown'",
        "state='unknown'",
        "compile_key='bad'",
        "snapshot_id='bad'",
        "idempotency_key='bad key'",
        "attempt_count=-1",
        "worker_id=gen_random_uuid()",
        "state='claimed', lease_until=now()",
        "state='claimed', worker_id=gen_random_uuid()",
    ];
    for (index, assignment) in invalid_sqls.into_iter().enumerate() {
        let id = insert_job(
            pool,
            tenant,
            user,
            workspace,
            &format!("mutation:{index}"),
            "queued",
        )
        .await;
        let expression = if assignment.contains("gen_random_uuid") {
            assignment.replace("gen_random_uuid()", &format!("'{}'::uuid", Uuid::new_v4()))
        } else {
            assignment.to_owned()
        };
        let result = sqlx::query(&format!(
            "UPDATE latex_core.compile_jobs SET {expression} WHERE id=$1"
        ))
        .bind(id)
        .execute(pool)
        .await;
        assert_constraint(result);
    }
    let first = insert_job(pool, tenant, user, workspace, "request:abc", "queued").await;
    assert_ne!(first, Uuid::nil());
    let duplicate = insert_job_result(pool, tenant, user, workspace, "request:abc").await;
    assert_constraint(duplicate);
    let user2 = Uuid::new_v4();
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user2)
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    insert_job(pool, tenant, user2, workspace, "request:abc", "queued").await;
}

async fn insert_job_result(
    pool: &PgPool,
    tenant: Uuid,
    user: Uuid,
    workspace: Uuid,
    key: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("INSERT INTO latex_core.compile_jobs (id,tenant_id,user_id,workspace_id,snapshot_id,compile_key,idempotency_key,engine,tex_environment_id,latexmk_profile,shell_policy,synctex,cost_class,state) VALUES ($1,$2,$3,$4,$5,$6,$7,'pdflatex','texlive-2026','default','safe',true,'normal','queued')")
        .bind(Uuid::new_v4()).bind(tenant).bind(user).bind(workspace).bind(SNAPSHOT).bind(COMPILE).bind(key).execute(pool).await
}

async fn verify_queue_indexes(pool: &PgPool) {
    let indexes: Vec<String> = sqlx::query_scalar("SELECT indexname FROM pg_indexes WHERE schemaname='latex_core' AND tablename='compile_jobs'").fetch_all(pool).await.unwrap();
    for expected in [
        "compile_jobs_queue_idx",
        "compile_jobs_expired_lease_idx",
        "compile_jobs_user_state_idx",
        "compile_jobs_workspace_idx",
        "compile_jobs_compile_key_idx",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
}

async fn verify_artifacts_and_cache(pool: &PgPool, tenant: Uuid, user: Uuid, workspace: Uuid) {
    let job = insert_job(pool, tenant, user, workspace, "artifact-job", "running").await;
    let artifact_sql = "INSERT INTO latex_core.compilation_artifacts (artifact_id,job_id,compile_key,kind,logical_name,blob_hash,size_bytes) VALUES ($1,$2,$3,$4,$5,$6,$7)";
    for (kind, name) in [("pdf", "output.pdf"), ("log", "output.log")] {
        sqlx::query(artifact_sql)
            .bind(Uuid::new_v4())
            .bind(job)
            .bind(COMPILE)
            .bind(kind)
            .bind(name)
            .bind(BLOB)
            .bind(10_i64)
            .execute(pool)
            .await
            .unwrap();
    }
    assert_constraint(
        sqlx::query(artifact_sql)
            .bind(Uuid::new_v4())
            .bind(job)
            .bind(COMPILE)
            .bind("pdf")
            .bind("output.pdf")
            .bind(BLOB)
            .bind(1_i64)
            .execute(pool)
            .await,
    );
    assert_constraint(
        sqlx::query(artifact_sql)
            .bind(Uuid::new_v4())
            .bind(job)
            .bind(COMPILE)
            .bind("other")
            .bind("bad")
            .bind(BLOB)
            .bind(-1_i64)
            .execute(pool)
            .await,
    );
    let successful = insert_job(pool, tenant, user, workspace, "cache-job", "succeeded").await;
    sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ($1,$2,$3)").bind("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").bind(successful).bind(MANIFEST).execute(pool).await.unwrap();
    assert_constraint(sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ('bad',$1,$2)").bind(successful).bind(MANIFEST).execute(pool).await);
    assert_constraint(sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ($1,$2,$3)").bind("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").bind(Uuid::new_v4()).bind(MANIFEST).execute(pool).await);
}

fn assert_constraint<T>(result: Result<T, sqlx::Error>) {
    let error = match result {
        Ok(_) => panic!("PostgreSQL must reject invalid row"),
        Err(error) => error,
    };
    let is_constraint = matches!(
        error,
        sqlx::Error::Database(ref database)
            if database.code().is_some_and(|code| code.starts_with("23"))
    );
    assert!(is_constraint, "expected constraint violation: {error}");
}
