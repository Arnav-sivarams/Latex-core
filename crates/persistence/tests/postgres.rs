#![cfg(feature = "database-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test fixtures"
)]

use core_types::{
    BlobHash, CompileKey, CostClass, IdempotencyKey, JobId, LatexmkProfileId, ShellPolicy,
    SnapshotId, TenantId, TexEngine, TexEnvironmentId, UserId, WorkerId, WorkspaceId,
};
use persistence::{
    AppError, AppRepository, Database, DatabaseConfig, EnqueueCompileJobV1, FilePolicy,
    InfrastructureOutcome, PostgresCompileQueue, ProjectRoles, PublishResult, QueueError,
    QueueLimits, TeamFileRecord,
};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::{env, time::Duration};
use uuid::Uuid;

const MANIFEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const COMPILE: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const BLOB: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
// Queue claims are intentionally global, so concurrent integration fixtures share one queue.
static QUEUE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn production_schema_enforces_relational_contract() {
    let _guard = QUEUE_TEST_LOCK.lock().await;
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

    sqlx::query("UPDATE latex_core.compile_jobs SET state='cancelled',finished_at=statement_timestamp() WHERE workspace_id=$1 AND state='queued'")
        .bind(workspace_a)
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn team_publish_is_canonical_and_preserves_stale_member_drafts() {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(&url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repo = AppRepository::new(database.clone());
    let suffix = Uuid::new_v4();
    let alice_email = format!("team-alice-{suffix}@example.test");
    let bob_email = format!("team-bob-{suffix}@example.test");
    let carol_email = format!("team-carol-{suffix}@example.test");
    let alice = repo.create_account(&alice_email, "hash").await.unwrap();
    let bob = repo.create_account(&bob_email, "hash").await.unwrap();
    let carol = repo.create_account(&carol_email, "hash").await.unwrap();
    let workspace = WorkspaceId::new();
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(alice.tenant_id.as_uuid())
        .bind(alice.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let team = repo
        .create_team(alice.user_id, "Thesis team")
        .await
        .unwrap();
    repo.set_group_member(bob.user_id, team.id, bob.user_id, false)
        .await
        .unwrap_err();
    repo.set_group_member(alice.user_id, team.id, bob.user_id, false)
        .await
        .unwrap();
    let canonical = BlobHash::digest(b"canonical chapter one");
    let project = repo
        .create_team_project(
            alice.user_id,
            team.id,
            workspace,
            "Shared thesis",
            &[
                TeamFileRecord {
                    path: "chapters/chapter1.tex".to_owned(),
                    blob_hash: canonical,
                    size_bytes: 21,
                    revision: 1,
                    policy: FilePolicy::Editable,
                },
                TeamFileRecord {
                    path: "chapters/chapter2.tex".to_owned(),
                    blob_hash: BlobHash::digest(b"canonical chapter two"),
                    size_bytes: 21,
                    revision: 1,
                    policy: FilePolicy::Editable,
                },
            ],
        )
        .await
        .unwrap();
    repo.set_project_member(
        alice.user_id,
        project.id,
        bob.user_id,
        ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: false,
        },
    )
    .await
    .unwrap();
    let alice_draft = BlobHash::digest(b"alice chapter one");
    let bob_draft = BlobHash::digest(b"bob chapter one");
    repo.save_draft(
        alice.user_id,
        project.id,
        "chapters/chapter1.tex",
        1,
        alice_draft,
        17,
    )
    .await
    .unwrap();
    repo.save_draft(
        bob.user_id,
        project.id,
        "chapters/chapter1.tex",
        1,
        bob_draft,
        15,
    )
    .await
    .unwrap();
    let bob_second_chapter = BlobHash::digest(b"bob chapter two");
    repo.save_draft(
        bob.user_id,
        project.id,
        "chapters/chapter2.tex",
        1,
        bob_second_chapter,
        15,
    )
    .await
    .unwrap();
    assert!(matches!(
        repo.publish_draft(bob.user_id, project.id, "chapters/chapter2.tex")
            .await
            .unwrap(),
        PublishResult::Published {
            canonical_generation: 2,
            file_revision: 2,
            ..
        }
    ));
    assert!(matches!(
        repo.publish_draft(alice.user_id, project.id, "chapters/chapter1.tex")
            .await
            .unwrap(),
        PublishResult::Published {
            canonical_generation: 3,
            file_revision: 2,
            ..
        }
    ));
    assert!(matches!(
        repo.publish_draft(bob.user_id, project.id, "chapters/chapter1.tex")
            .await
            .unwrap(),
        PublishResult::Conflict {
            current_file_revision: 2
        }
    ));
    assert_eq!(
        repo.draft_for_user(bob.user_id, project.id, "chapters/chapter1.tex")
            .await
            .unwrap()
            .unwrap()
            .blob_hash,
        bob_draft
    );
    assert_eq!(
        repo.team_file_for_user(alice.user_id, project.id, "chapters/chapter1.tex")
            .await
            .unwrap()
            .blob_hash,
        alice_draft
    );
    assert_eq!(
        repo.team_file_for_user(alice.user_id, project.id, "chapters/chapter2.tex")
            .await
            .unwrap()
            .blob_hash,
        bob_second_chapter
    );
    let new_file_draft = BlobHash::digest(b"private new chapter");
    repo.save_draft(
        alice.user_id,
        project.id,
        "chapters/new.tex",
        0,
        new_file_draft,
        19,
    )
    .await
    .unwrap();
    assert_eq!(
        repo.draft_only_paths_for_user(alice.user_id, project.id)
            .await
            .unwrap()
            .iter()
            .map(|draft| draft.path.as_str())
            .collect::<Vec<_>>(),
        vec!["chapters/new.tex"]
    );
    assert_eq!(
        repo.draft_only_for_user(alice.user_id, project.id, "chapters/new.tex")
            .await
            .unwrap()
            .unwrap()
            .blob_hash,
        new_file_draft
    );
    assert!(matches!(
        repo.publish_draft(alice.user_id, project.id, "chapters/new.tex")
            .await
            .unwrap(),
        PublishResult::Published {
            file_revision: 1,
            ..
        }
    ));
    assert!(
        repo.draft_only_for_user(alice.user_id, project.id, "chapters/new.tex")
            .await
            .unwrap()
            .is_none()
    );
    let event_hash: String = sqlx::query_scalar("SELECT payload->'operations'->0->>'blob_hash' FROM latex_core.workspace_events WHERE workspace_id=$1 ORDER BY sequence DESC LIMIT 1")
        .bind(workspace.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(event_hash, new_file_draft.to_hex());
    repo.set_file_policy(
        alice.user_id,
        project.id,
        "chapters/chapter1.tex",
        FilePolicy::ReadOnly,
    )
    .await
    .unwrap();
    assert!(matches!(
        repo.save_draft(
            bob.user_id,
            project.id,
            "chapters/chapter1.tex",
            2,
            bob_draft,
            15
        )
        .await,
        Err(AppError::Forbidden)
    ));
    repo.set_file_policy(
        alice.user_id,
        project.id,
        "chapters/chapter1.tex",
        FilePolicy::Managed,
    )
    .await
    .unwrap();
    assert!(
        repo.team_files_for_user(bob.user_id, project.id)
            .await
            .unwrap()
            .iter()
            .any(|file| file.path == "chapters/chapter1.tex")
    );
    assert!(matches!(
        repo.team_file_for_user(bob.user_id, project.id, "chapters/chapter1.tex")
            .await,
        Ok(_)
    ));
    repo.set_user_account_type(&carol_email, "admin")
        .await
        .unwrap();
    assert!(repo.teams_for_user(carol.user_id).await.unwrap().is_empty());
    assert!(matches!(
        repo.team_file_for_user(carol.user_id, project.id, "chapters/chapter1.tex")
            .await,
        Err(AppError::NotFound)
    ));
    repo.set_user_account_type(&bob_email, "professor")
        .await
        .unwrap();
    assert!(matches!(
        repo.project_access(bob.user_id, workspace).await.unwrap(),
        persistence::ProjectAccess::Team {
            can_write: true,
            can_mentor: false,
            ..
        }
    ));
    let template_name = format!("Faculty template {suffix}");
    repo.create_template(Uuid::new_v4(), &template_name, None, None, &[])
        .await
        .unwrap();
    repo.set_template_audiences(&template_name, &["professor"])
        .await
        .unwrap();
    assert!(
        repo.list_templates_for_user(alice.user_id)
            .await
            .unwrap()
            .iter()
            .all(|template| template.name != template_name)
    );
    assert!(
        repo.list_templates_for_user(bob.user_id)
            .await
            .unwrap()
            .iter()
            .any(|template| template.name == template_name)
    );
    repo.grant_template_to_user(&template_name, alice.user_id, carol.user_id)
        .await
        .unwrap();
    assert!(
        repo.list_templates_for_user(alice.user_id)
            .await
            .unwrap()
            .iter()
            .any(|template| template.name == template_name)
    );
    let second = repo.create_team(bob.user_id, "Second team").await.unwrap();
    assert_eq!(repo.teams_for_user(bob.user_id).await.unwrap().len(), 2);
    assert_ne!(team.id, second.id);
    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn project_manager_can_assign_composable_roles_and_cannot_remove_last_manager() {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(&url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repo = AppRepository::new(database.clone());
    let suffix = Uuid::new_v4();
    let alice = repo
        .create_account(
            &format!("project-manager-alice-{suffix}@example.test"),
            "hash",
        )
        .await
        .unwrap();
    let bob = repo
        .create_account(
            &format!("project-manager-bob-{suffix}@example.test"),
            "hash",
        )
        .await
        .unwrap();
    let carol = repo
        .create_account(
            &format!("project-manager-carol-{suffix}@example.test"),
            "hash",
        )
        .await
        .unwrap();
    let workspace = WorkspaceId::new();
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(alice.tenant_id.as_uuid())
        .bind(alice.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let team = repo
        .create_team(alice.user_id, "Role matrix")
        .await
        .unwrap();
    repo.set_group_member(alice.user_id, team.id, bob.user_id, false)
        .await
        .unwrap();
    repo.set_group_member(alice.user_id, team.id, carol.user_id, false)
        .await
        .unwrap();
    let project = repo
        .create_team_project(alice.user_id, team.id, workspace, "Roles", &[])
        .await
        .unwrap();
    let creator = repo
        .project_members(alice.user_id, project.id)
        .await
        .unwrap();
    assert_eq!(creator.len(), 1);
    assert!(creator[0].writer && creator[0].project_manager);

    for roles in [
        ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: false,
        },
        ProjectRoles {
            writer: false,
            mentor: true,
            project_manager: false,
        },
        ProjectRoles {
            writer: false,
            mentor: false,
            project_manager: true,
        },
        ProjectRoles {
            writer: true,
            mentor: true,
            project_manager: false,
        },
        ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: true,
        },
        ProjectRoles {
            writer: false,
            mentor: true,
            project_manager: true,
        },
    ] {
        repo.set_project_member(alice.user_id, project.id, bob.user_id, roles)
            .await
            .unwrap();
        let members = repo
            .project_members(alice.user_id, project.id)
            .await
            .unwrap();
        let bob_roles = members
            .iter()
            .find(|member| member.user_id == bob.user_id)
            .unwrap();
        assert_eq!(
            (
                bob_roles.writer,
                bob_roles.mentor,
                bob_roles.project_manager
            ),
            (roles.writer, roles.mentor, roles.project_manager)
        );
    }

    repo.set_project_member(
        alice.user_id,
        project.id,
        alice.user_id,
        ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: false,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        repo.set_project_member(
            bob.user_id,
            project.id,
            bob.user_id,
            ProjectRoles {
                writer: false,
                mentor: true,
                project_manager: false,
            },
        )
        .await,
        Err(AppError::Integrity { .. })
    ));
    assert!(matches!(
        repo.remove_project_member(bob.user_id, project.id, bob.user_id)
            .await,
        Err(AppError::Integrity { .. })
    ));
    repo.set_project_member(
        bob.user_id,
        project.id,
        carol.user_id,
        ProjectRoles {
            writer: false,
            mentor: true,
            project_manager: true,
        },
    )
    .await
    .unwrap();
    repo.remove_project_member(bob.user_id, project.id, carol.user_id)
        .await
        .unwrap();
    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn research_group_has_one_workspace_and_equal_members() {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(&url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repo = AppRepository::new(database.clone());
    let suffix = Uuid::new_v4();
    let owner = repo
        .create_account(&format!("group-owner-{suffix}@example.test"), "hash")
        .await
        .unwrap();
    let member = repo
        .create_account(&format!("group-member-{suffix}@example.test"), "hash")
        .await
        .unwrap();
    let outsider = repo
        .create_account(&format!("group-outsider-{suffix}@example.test"), "hash")
        .await
        .unwrap();
    let workspace = WorkspaceId::new();
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(owner.tenant_id.as_uuid())
        .bind(owner.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let group = repo
        .create_research_group(owner.user_id, workspace, "Vision Lab")
        .await
        .unwrap();
    assert_eq!(group.workspace_id, workspace);
    repo.add_research_group_member(owner.user_id, group.id, member.user_id)
        .await
        .unwrap();
    assert_eq!(
        repo.research_group_members(member.user_id, group.id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(matches!(
        repo.add_research_group_member(member.user_id, group.id, outsider.user_id)
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        repo.remove_research_group_member(owner.user_id, group.id, owner.user_id)
            .await,
        Err(AppError::Integrity { .. })
    ));
    assert!(matches!(
        repo.research_group_for_user(outsider.user_id, group.id)
            .await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        repo.create_research_group(owner.user_id, workspace, "Duplicate")
            .await,
        Err(AppError::Conflict)
    ));
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
        "audit_events",
        "file_policies",
        "member_change_operations",
        "member_drafts",
        "permission_overrides",
        "projects",
        "research_group_members",
        "research_groups",
        "sessions",
        "snapshots",
        "team_members",
        "team_project_audit",
        "team_project_files",
        "team_project_members",
        "team_projects",
        "teams",
        "template_account_types",
        "template_files",
        "template_policy_rules",
        "template_user_grants",
        "templates",
        "tenants",
        "user_credentials",
        "users",
        "workspace_events",
        "workspace_heads",
        "workspace_resume",
        "workspace_snapshots",
        "workspaces",
    ];
    expected.sort_unstable();
    assert_eq!(tables, expected);
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
    let snapshot = schema_snapshot(a);
    let manifest = digest(format!("schema-manifest-{a}").as_bytes());
    sqlx::query(
        "INSERT INTO latex_core.snapshots (snapshot_id, manifest_blob_hash) VALUES ($1,$2)",
    )
    .bind(&snapshot)
    .bind(&manifest)
    .execute(pool)
    .await
    .unwrap();
    for (workspace, version) in [(a, 1_i64), (b, 1_i64), (a, 2_i64)] {
        sqlx::query("INSERT INTO latex_core.workspace_snapshots (workspace_id, workspace_version, snapshot_id) VALUES ($1,$2,$3)").bind(workspace).bind(version).bind(&snapshot).execute(pool).await.unwrap();
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
        .bind(schema_snapshot(workspace))
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
    let valid = insert_job(pool, tenant, user, workspace, "valid:queued", "queued").await;
    for engine in ["latex", "pdflatex", "lualatex", "xelatex"] {
        sqlx::query("UPDATE latex_core.compile_jobs SET engine = $1 WHERE id = $2")
            .bind(engine)
            .bind(valid)
            .execute(pool)
            .await
            .unwrap();
    }
    let invalid_sqls = [
        "engine='unknown'",
        "engine='tectonic'",
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
        .bind(Uuid::new_v4()).bind(tenant).bind(user).bind(workspace).bind(schema_snapshot(workspace)).bind(COMPILE).bind(key).execute(pool).await
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
    let cache_key = digest(format!("schema-cache-key-{workspace}").as_bytes());
    let cache_manifest = digest(format!("schema-cache-manifest-{workspace}").as_bytes());
    sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ($1,$2,$3)").bind(&cache_key).bind(successful).bind(&cache_manifest).execute(pool).await.unwrap();
    assert_constraint(sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ('bad',$1,$2)").bind(successful).bind(MANIFEST).execute(pool).await);
    assert_constraint(sqlx::query("INSERT INTO latex_core.compile_cache (compile_key,source_job_id,artifact_manifest_blob_hash) VALUES ($1,$2,$3)").bind(digest(format!("schema-invalid-cache-{workspace}").as_bytes())).bind(Uuid::new_v4()).bind(&cache_manifest).execute(pool).await);
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

fn digest(seed: &[u8]) -> String {
    BlobHash::digest(seed).to_hex()
}

fn schema_snapshot(workspace: Uuid) -> String {
    digest(format!("schema-snapshot-{workspace}").as_bytes())
}

async fn queue_fixture() -> (
    PostgresCompileQueue,
    PgPool,
    TenantId,
    UserId,
    WorkspaceId,
    SnapshotId,
) {
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
    let snapshot: SnapshotId = digest(format!("queue snapshot {workspace}").as_bytes())
        .parse()
        .unwrap();
    sqlx::query("INSERT INTO latex_core.snapshots (snapshot_id,manifest_blob_hash) VALUES ($1,$2)")
        .bind(snapshot.to_hex())
        .bind(digest(format!("queue manifest {workspace}").as_bytes()))
        .execute(&pool)
        .await
        .unwrap();
    let limits = QueueLimits::new(2, 1, 3, Duration::from_secs(30), 2).unwrap();
    (
        PostgresCompileQueue::new(database, limits),
        pool,
        tenant,
        user,
        workspace,
        snapshot,
    )
}

fn request(
    tenant: TenantId,
    user: UserId,
    workspace: WorkspaceId,
    snapshot: SnapshotId,
    priority: i16,
    suffix: &str,
) -> EnqueueCompileJobV1 {
    EnqueueCompileJobV1 {
        job_id: JobId::new(),
        tenant_id: tenant,
        user_id: user,
        workspace_id: workspace,
        snapshot_id: snapshot,
        compile_key: digest(format!("key-{workspace}-{suffix}").as_bytes())
            .parse::<CompileKey>()
            .unwrap(),
        idempotency_key: IdempotencyKey::parse(&format!("request-{workspace}-{suffix}")).unwrap(),
        engine: TexEngine::PdfLatex,
        tex_environment_id: TexEnvironmentId::parse("texlive-2026+full@1").unwrap(),
        latexmk_profile: LatexmkProfileId::parse("safe-v1").unwrap(),
        shell_policy: ShellPolicy::Safe,
        synctex: true,
        cost_class: CostClass::Normal,
        priority,
    }
}

#[tokio::test]
async fn durable_queue_orders_claims_enforces_caps_and_rejects_stale_completion() {
    let _guard = QUEUE_TEST_LOCK.lock().await;
    let (queue, pool, tenant, user, workspace, snapshot) = queue_fixture().await;
    let low = request(tenant, user, workspace, snapshot, 1, "low");
    let high = request(tenant, user, workspace, snapshot, 9, "high");
    queue.enqueue(low.clone()).await.unwrap();
    queue.enqueue(high.clone()).await.unwrap();
    let worker = WorkerId::new();
    let winner = queue.claim(worker).await.unwrap().unwrap();
    assert_eq!(winner.id, high.job_id);
    assert!(
        queue.claim(WorkerId::new()).await.unwrap().is_none(),
        "per-user running cap applies during claim"
    );
    assert!(matches!(
        queue.renew_lease(winner.id, WorkerId::new()).await,
        Err(QueueError::LeaseLost)
    ));
    let overflow = request(tenant, user, workspace, snapshot, 0, "overflow");
    assert_eq!(
        queue.enqueue(overflow.clone()).await.unwrap(),
        overflow.job_id
    );
    assert!(matches!(
        queue
            .enqueue(request(
                tenant,
                user,
                workspace,
                snapshot,
                0,
                "overflow-two"
            ))
            .await,
        Err(QueueError::AdmissionRejected { .. })
    ));
    queue
        .complete_compile_failure(winner.id, worker, false, json!({"class":"test-cleanup"}))
        .await
        .unwrap();
    for job in [low.job_id, overflow.job_id] {
        queue
            .request_cancellation(job, Some("test cleanup"))
            .await
            .unwrap();
    }
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.compile_jobs WHERE workspace_id=$1 AND state='running'",
    )
    .bind(workspace.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(running, 0);
    pool.close().await;
}

#[tokio::test]
async fn cancellation_recovery_and_infrastructure_retry_are_durable() {
    let _guard = QUEUE_TEST_LOCK.lock().await;
    let (queue, pool, tenant, user, workspace, snapshot) = queue_fixture().await;
    let queued = request(tenant, user, workspace, snapshot, 0, "cancel");
    queue.enqueue(queued.clone()).await.unwrap();
    queue
        .request_cancellation(queued.job_id, Some("user request"))
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM latex_core.compile_jobs WHERE id=$1")
        .bind(queued.job_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "cancelled");
    let running = request(tenant, user, workspace, snapshot, 0, "retry");
    queue.enqueue(running.clone()).await.unwrap();
    let worker = WorkerId::new();
    let claimed = queue.claim(worker).await.unwrap().unwrap();
    assert_eq!(claimed.id, running.job_id);
    assert_eq!(
        queue
            .complete_infrastructure_failure(claimed.id, worker, json!({"class":"infrastructure"}))
            .await
            .unwrap(),
        InfrastructureOutcome::Requeued
    );
    let retry = queue.claim(worker).await.unwrap().unwrap();
    assert_eq!(retry.attempt_count, 2);
    assert_eq!(
        queue
            .complete_infrastructure_failure(retry.id, worker, json!({"class":"infrastructure"}))
            .await
            .unwrap(),
        InfrastructureOutcome::Failed
    );
    let state: String = sqlx::query_scalar("SELECT state FROM latex_core.compile_jobs WHERE id=$1")
        .bind(running.job_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "failed");
    pool.close().await;
}
