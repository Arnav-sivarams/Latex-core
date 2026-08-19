#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration test fixtures"
)]

use core_types::{BlobHash, WorkspaceId};
use persistence::{
    AppError, AppRepository, AppTemplateFileRecord, ChangeSetPublishResult, Database,
    DatabaseConfig, FilePolicy, PendingChangeSummary, TeamFileRecord,
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

struct Fixture {
    database: Database,
    repo: AppRepository,
    pool: PgPool,
    alice: core_types::UserId,
    bob: core_types::UserId,
    team: Uuid,
    tenant: Uuid,
    project: Uuid,
}

async fn fixture() -> Fixture {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(DatabaseConfig::development(&url).unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repo = AppRepository::new(database.clone());
    let nonce = Uuid::new_v4();
    let alice = repo
        .create_account(&format!("projection-alice-{nonce}@example.test"), "hash")
        .await
        .unwrap();
    let bob = repo
        .create_account(&format!("projection-bob-{nonce}@example.test"), "hash")
        .await
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let workspace = WorkspaceId::new();
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
        .create_team(alice.user_id, "Projection team")
        .await
        .unwrap();
    repo.set_group_member(alice.user_id, team.id, bob.user_id, false)
        .await
        .unwrap();
    let project = repo
        .create_team_project(
            alice.user_id,
            team.id,
            workspace,
            "Projection project",
            &[
                TeamFileRecord {
                    path: "a.tex".into(),
                    blob_hash: BlobHash::digest(b"a"),
                    size_bytes: 1,
                    revision: 4,
                    policy: FilePolicy::Editable,
                },
                TeamFileRecord {
                    path: "b.tex".into(),
                    blob_hash: BlobHash::digest(b"b"),
                    size_bytes: 1,
                    revision: 3,
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
        persistence::ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: false,
        },
    )
    .await
    .unwrap();
    Fixture {
        database,
        repo,
        pool,
        alice: alice.user_id,
        bob: bob.user_id,
        team: team.id,
        tenant: *alice.tenant_id.as_uuid(),
        project: project.id,
    }
}

async fn close(fixture: Fixture) {
    fixture.pool.close().await;
    fixture.database.close().await;
}

fn live_paths(tree: &persistence::PrivateWorkingTree) -> Vec<&str> {
    tree.files
        .iter()
        .filter(|file| !file.pending_delete)
        .map(|file| file.path.as_str())
        .collect()
}

async fn add_canonical_file(fixture: &Fixture, path: &str, revision: i64) {
    sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,1,$4)")
        .bind(fixture.project)
        .bind(path)
        .bind(BlobHash::digest(path.as_bytes()).to_hex())
        .bind(revision)
        .execute(&fixture.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn canonical_and_private_draft_projection_is_member_isolated() {
    let fixture = fixture().await;
    let new_hash = BlobHash::digest(b"alice draft");
    assert_eq!(
        fixture
            .repo
            .save_draft_with_revision(
                fixture.alice,
                fixture.project,
                "a.tex",
                4,
                new_hash,
                11,
                Some(0)
            )
            .await
            .unwrap(),
        1
    );
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "private.tex",
            0,
            BlobHash::digest(b"private"),
            7,
            Some(0),
        )
        .await
        .unwrap();
    let alice = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(
        alice
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.tex", "b.tex", "private.tex"]
    );
    let changed = alice
        .files
        .iter()
        .find(|file| file.path == "a.tex")
        .unwrap();
    assert_eq!(changed.canonical_path.as_deref(), Some("a.tex"));
    assert_eq!(changed.canonical_revision, Some(4));
    assert!(changed.modified);
    assert_eq!(changed.draft_revision, Some(1));
    assert_eq!(alice.summary.modified, 1);
    assert_eq!(alice.summary.added, 1);
    let bob = fixture
        .repo
        .private_working_tree(fixture.bob, fixture.project)
        .await
        .unwrap();
    assert_eq!(
        bob.files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.tex", "b.tex"]
    );
    assert_eq!(
        fixture
            .repo
            .team_files_for_user(fixture.bob, fixture.project)
            .await
            .unwrap()
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.tex", "b.tex"]
    );
    close(fixture).await;
}

#[tokio::test]
async fn project_manager_can_assign_each_supported_project_role() {
    let fixture = fixture().await;
    for roles in [
        persistence::ProjectRoles {
            writer: true,
            mentor: false,
            project_manager: false,
        },
        persistence::ProjectRoles {
            writer: false,
            mentor: true,
            project_manager: false,
        },
        persistence::ProjectRoles {
            writer: false,
            mentor: false,
            project_manager: true,
        },
    ] {
        fixture
            .repo
            .set_project_member(fixture.alice, fixture.project, fixture.bob, roles)
            .await
            .unwrap();
        let assigned = fixture
            .repo
            .project_members(fixture.alice, fixture.project)
            .await
            .unwrap()
            .into_iter()
            .find(|member| member.user_id == fixture.bob)
            .unwrap();
        assert_eq!(assigned.writer, roles.writer);
        assert_eq!(assigned.mentor, roles.mentor);
        assert_eq!(assigned.project_manager, roles.project_manager);
    }
    close(fixture).await;
}

#[tokio::test]
async fn projection_normalizes_rename_chains_and_projected_path_drafts() {
    let fixture = fixture().await;
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "first.tex")
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "first.tex", "final.tex")
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "final.tex",
            4,
            BlobHash::digest(b"renamed draft"),
            13,
            Some(0),
        )
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    let file = tree
        .files
        .iter()
        .find(|file| file.path == "final.tex")
        .unwrap();
    assert_eq!(file.canonical_path.as_deref(), Some("a.tex"));
    assert_eq!(file.canonical_revision, Some(4));
    assert!(file.renamed && file.modified);
    assert_eq!(
        tree.summary,
        PendingChangeSummary {
            modified: 0,
            added: 0,
            renamed: 1,
            deleted: 0,
            main_changed: false,
            total: 1,
        }
    );
    close(fixture).await;
}

#[tokio::test]
async fn projection_normalizes_private_add_rename_delete_and_main() {
    let fixture = fixture().await;
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "x.tex",
            0,
            BlobHash::digest(b"x"),
            1,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "x.tex", "y.tex")
        .await
        .unwrap();
    fixture
        .repo
        .set_team_main_file(fixture.alice, fixture.project, "y.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(tree.pending_main.as_deref(), Some("y.tex"));
    assert!(
        tree.files
            .iter()
            .any(|file| file.path == "y.tex" && file.added)
    );
    assert_eq!(tree.summary.added, 1);
    assert!(tree.summary.main_changed);
    assert_eq!(tree.summary.total, 2);
    assert!(matches!(
        fixture
            .repo
            .delete_team_file(fixture.alice, fixture.project, "y.tex")
            .await,
        Err(AppError::Conflict)
    ));
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "b.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(tree.summary.deleted, 1);
    close(fixture).await;
}

#[tokio::test]
async fn projection_rejects_collisions_and_preserves_draft_revision_contract() {
    let fixture = fixture().await;
    assert_eq!(
        fixture
            .repo
            .save_draft_with_revision(
                fixture.alice,
                fixture.project,
                "a.tex",
                4,
                BlobHash::digest(b"v1"),
                2,
                Some(0),
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .repo
            .save_draft_with_revision(
                fixture.alice,
                fixture.project,
                "a.tex",
                4,
                BlobHash::digest(b"v2"),
                2,
                Some(1),
            )
            .await
            .unwrap(),
        2
    );
    assert!(matches!(
        fixture
            .repo
            .save_draft_with_revision(
                fixture.alice,
                fixture.project,
                "a.tex",
                4,
                BlobHash::digest(b"stale"),
                5,
                Some(1),
            )
            .await,
        Err(AppError::DraftConflict)
    ));
    assert!(matches!(
        fixture
            .repo
            .rename_team_file(fixture.alice, fixture.project, "a.tex", "b.tex")
            .await,
        Err(AppError::Conflict)
    ));
    close(fixture).await;
}

#[tokio::test]
async fn draft_before_rename_and_rename_chain_keep_one_canonical_descendant() {
    let fixture = fixture().await;
    let hash = BlobHash::digest(b"before rename");
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "a.tex",
            4,
            hash,
            13,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "b-renamed.tex")
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(
            fixture.alice,
            fixture.project,
            "b-renamed.tex",
            "c-renamed.tex",
        )
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(live_paths(&tree), vec!["b.tex", "c-renamed.tex"]);
    let file = tree
        .files
        .iter()
        .find(|file| file.path == "c-renamed.tex")
        .unwrap();
    assert_eq!(file.canonical_path.as_deref(), Some("a.tex"));
    assert_eq!(file.canonical_revision, Some(4));
    assert!(file.renamed && file.modified);
    assert_eq!(file.draft_revision, Some(1));
    assert_eq!(tree.summary.renamed, 1);
    assert_eq!(tree.summary.modified, 0);
    close(fixture).await;
}

#[tokio::test]
async fn private_add_delete_and_add_rename_delete_collapse() {
    let fixture = fixture().await;
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "x.tex",
            0,
            BlobHash::digest(b"x"),
            1,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "x.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(live_paths(&tree), vec!["a.tex", "b.tex"]);
    assert_eq!(tree.summary.total, 0);
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "z.tex",
            0,
            BlobHash::digest(b"z"),
            1,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "z.tex", "y.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(live_paths(&tree), vec!["a.tex", "b.tex", "y.tex"]);
    assert!(
        tree.files
            .iter()
            .any(|file| file.path == "y.tex" && file.added)
    );
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "y.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(live_paths(&tree), vec!["a.tex", "b.tex"]);
    assert_eq!(tree.summary.total, 0);
    close(fixture).await;
}

#[tokio::test]
async fn canonical_deletes_are_private_and_edit_after_delete_is_rejected() {
    let fixture = fixture().await;
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "renamed.tex")
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "renamed.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    let deleted = tree.files.iter().find(|file| file.pending_delete).unwrap();
    assert_eq!(deleted.canonical_path.as_deref(), Some("a.tex"));
    assert_eq!(tree.summary.deleted, 1);
    assert_eq!(tree.summary.renamed, 0);
    assert!(matches!(
        fixture
            .repo
            .save_draft_with_revision(
                fixture.alice,
                fixture.project,
                "a.tex",
                4,
                BlobHash::digest(b"no"),
                2,
                Some(0)
            )
            .await,
        Err(AppError::Conflict)
    ));
    let canonical: Vec<String> = sqlx::query_scalar("SELECT logical_path FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(canonical, vec!["a.tex", "b.tex"]);
    close(fixture).await;
}

#[tokio::test]
async fn repeated_main_follows_rename_and_cannot_be_deleted() {
    let fixture = fixture().await;
    fixture
        .repo
        .set_team_main_file(fixture.alice, fixture.project, "a.tex")
        .await
        .unwrap();
    fixture
        .repo
        .set_team_main_file(fixture.alice, fixture.project, "b.tex")
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "b.tex", "main.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(tree.pending_main.as_deref(), Some("main.tex"));
    let mains: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2 AND operation_type='set_main'")
        .bind(fixture.project).bind(fixture.alice.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(mains, 1);
    assert!(matches!(
        fixture
            .repo
            .delete_team_file(fixture.alice, fixture.project, "main.tex")
            .await,
        Err(AppError::Conflict)
    ));
    close(fixture).await;
}

#[tokio::test]
async fn summary_is_exact_and_staging_never_changes_canonical_state() {
    let fixture = fixture().await;
    for path in ["c.tex", "d.tex", "e.tex"] {
        add_canonical_file(&fixture, path, 1).await;
    }
    let before_files: Vec<(String, i64)> = sqlx::query_as("SELECT logical_path,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    let before_events: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.workspace_events WHERE workspace_id=(SELECT workspace_id FROM latex_core.team_projects WHERE id=$1)").bind(fixture.project).fetch_one(&fixture.pool).await.unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "a.tex",
            4,
            BlobHash::digest(b"aa"),
            2,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "b.tex",
            3,
            BlobHash::digest(b"bb"),
            2,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "c.tex", "renamed-c.tex")
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "d.tex")
        .await
        .unwrap();
    fixture
        .repo
        .set_team_main_file(fixture.alice, fixture.project, "e.tex")
        .await
        .unwrap();
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(
        tree.summary,
        PendingChangeSummary {
            modified: 2,
            added: 0,
            renamed: 1,
            deleted: 1,
            main_changed: true,
            total: 5
        }
    );
    let after_files: Vec<(String, i64)> = sqlx::query_as("SELECT logical_path,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(after_files, before_files);
    let after_events: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.workspace_events WHERE workspace_id=(SELECT workspace_id FROM latex_core.team_projects WHERE id=$1)").bind(fixture.project).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(after_events, before_events);
    let paths = live_paths(&tree);
    let mut unique = paths.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(paths.len(), unique.len());
    close(fixture).await;
}

#[tokio::test]
async fn group_removal_aggregate_sums_normalized_private_work_across_projects() {
    let fixture = fixture().await;
    fixture
        .repo
        .save_draft_with_revision(
            fixture.bob,
            fixture.project,
            "a.tex",
            4,
            BlobHash::digest(b"bob a"),
            5,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.bob,
            fixture.project,
            "new1.tex",
            0,
            BlobHash::digest(b"bob new"),
            7,
            Some(0),
        )
        .await
        .unwrap();
    let workspace = WorkspaceId::new();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(fixture.tenant)
        .bind(fixture.alice.as_uuid())
        .execute(&fixture.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace.as_uuid())
        .execute(&fixture.pool)
        .await
        .unwrap();
    let project2 = fixture
        .repo
        .create_team_project(
            fixture.alice,
            fixture.team,
            workspace,
            "Second projection",
            &[
                TeamFileRecord {
                    path: "c.tex".into(),
                    blob_hash: BlobHash::digest(b"c"),
                    size_bytes: 1,
                    revision: 1,
                    policy: FilePolicy::Editable,
                },
                TeamFileRecord {
                    path: "d.tex".into(),
                    blob_hash: BlobHash::digest(b"d"),
                    size_bytes: 1,
                    revision: 1,
                    policy: FilePolicy::Editable,
                },
                TeamFileRecord {
                    path: "e.tex".into(),
                    blob_hash: BlobHash::digest(b"e"),
                    size_bytes: 1,
                    revision: 1,
                    policy: FilePolicy::Editable,
                },
            ],
        )
        .await
        .unwrap();
    fixture
        .repo
        .set_project_member(
            fixture.alice,
            project2.id,
            fixture.bob,
            persistence::ProjectRoles {
                writer: true,
                mentor: false,
                project_manager: false,
            },
        )
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.bob, project2.id, "c.tex", "renamed-c.tex")
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.bob, project2.id, "d.tex")
        .await
        .unwrap();
    fixture
        .repo
        .set_team_main_file(fixture.bob, project2.id, "e.tex")
        .await
        .unwrap();
    assert_eq!(
        fixture
            .repo
            .private_working_tree(fixture.bob, fixture.project)
            .await
            .unwrap()
            .summary
            .total,
        2
    );
    assert_eq!(
        fixture
            .repo
            .private_working_tree(fixture.bob, project2.id)
            .await
            .unwrap()
            .summary
            .total,
        3
    );
    let before_drafts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.member_drafts WHERE user_id=$1")
            .bind(fixture.bob.as_uuid())
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let before_ops: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.member_change_operations WHERE user_id=$1",
    )
    .bind(fixture.bob.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(
        fixture
            .repo
            .unpublished_change_count_for_member(fixture.alice, fixture.team, fixture.bob)
            .await
            .unwrap(),
        5
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.member_drafts WHERE user_id=$1"
        )
        .bind(fixture.bob.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        before_drafts
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM latex_core.member_change_operations WHERE user_id=$1"
        )
        .bind(fixture.bob.as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        before_ops
    );
    assert_eq!(
        fixture
            .repo
            .private_working_tree(fixture.bob, fixture.project)
            .await
            .unwrap()
            .summary
            .total,
        2
    );
    assert_eq!(
        fixture
            .repo
            .private_working_tree(fixture.bob, project2.id)
            .await
            .unwrap()
            .summary
            .total,
        3
    );
    close(fixture).await;
}

#[tokio::test]
async fn concurrent_structural_staging_allocates_distinct_sequences_and_stays_private() {
    let fixture = fixture().await;
    let (a, b) = tokio::join!(
        fixture
            .repo
            .rename_team_file(fixture.alice, fixture.project, "a.tex", "a-renamed.tex"),
        fixture
            .repo
            .rename_team_file(fixture.alice, fixture.project, "b.tex", "b-renamed.tex"),
    );
    assert!(a.is_ok(), "first stage failed: {a:?}");
    assert!(b.is_ok(), "second stage failed: {b:?}");
    let sequences: Vec<i64> = sqlx::query_scalar("SELECT operation_sequence FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2 AND operation_type='rename' ORDER BY operation_sequence")
        .bind(fixture.project).bind(fixture.alice.as_uuid()).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(sequences, vec![1, 2]);
    let tree = fixture
        .repo
        .private_working_tree(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert_eq!(live_paths(&tree), vec!["a-renamed.tex", "b-renamed.tex"]);
    let canonical: Vec<String> = sqlx::query_scalar("SELECT logical_path FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(canonical, vec!["a.tex", "b.tex"]);
    close(fixture).await;
}

#[tokio::test]
async fn publish_uses_one_final_delta_and_clears_private_state() {
    let fixture = fixture().await;
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "first.tex")
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "first.tex", "main.tex")
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "main.tex",
            4,
            BlobHash::digest(b"published"),
            9,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "new.tex",
            0,
            BlobHash::digest(b"new"),
            3,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "b.tex")
        .await
        .unwrap();
    fixture
        .repo
        .set_team_main_file(fixture.alice, fixture.project, "main.tex")
        .await
        .unwrap();
    let result = fixture
        .repo
        .publish_change_set(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert!(
        matches!(
            result,
            ChangeSetPublishResult::Published {
                canonical_generation: 2,
                workspace_version: 1,
                change_count: 3
            }
        ),
        "unexpected result: {result:?}"
    );
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT logical_path,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(rows, vec![("main.tex".into(), 5), ("new.tex".into(), 1)]);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.workspace_events WHERE workspace_id=(SELECT workspace_id FROM latex_core.team_projects WHERE id=$1)").bind(fixture.project).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(events, 1);
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2",
    )
    .bind(fixture.project)
    .bind(fixture.alice.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    let staged: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2").bind(fixture.project).bind(fixture.alice.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!((drafts, staged), (0, 0));
    close(fixture).await;
}

#[tokio::test]
async fn publish_conflict_is_atomic_and_keeps_private_state() {
    let fixture = fixture().await;
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "renamed.tex")
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "renamed.tex",
            4,
            BlobHash::digest(b"alice"),
            5,
            Some(0),
        )
        .await
        .unwrap();
    fixture
        .repo
        .delete_team_file(fixture.alice, fixture.project, "b.tex")
        .await
        .unwrap();
    sqlx::query("UPDATE latex_core.team_project_files SET file_revision=5 WHERE team_project_id=$1 AND logical_path='a.tex'").bind(fixture.project).execute(&fixture.pool).await.unwrap();
    let result = fixture
        .repo
        .publish_change_set(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert!(
        matches!(result, ChangeSetPublishResult::Conflict { .. }),
        "unexpected result: {result:?}"
    );
    let canonical: Vec<String> = sqlx::query_scalar("SELECT logical_path FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path").bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(canonical, vec!["a.tex", "b.tex"]);
    let generation: i64 =
        sqlx::query_scalar("SELECT canonical_generation FROM latex_core.team_projects WHERE id=$1")
            .bind(fixture.project)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(generation, 1);
    let drafts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2",
    )
    .bind(fixture.project)
    .bind(fixture.alice.as_uuid())
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    let staged: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2").bind(fixture.project).bind(fixture.alice.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!((drafts, staged), (1, 2));
    close(fixture).await;
}

#[tokio::test]
async fn rename_and_modify_requires_both_effective_permissions() {
    let fixture = fixture().await;
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "renamed.tex")
        .await
        .unwrap();
    fixture
        .repo
        .save_draft_with_revision(
            fixture.alice,
            fixture.project,
            "renamed.tex",
            4,
            BlobHash::digest(b"changed"),
            7,
            Some(0),
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.permission_overrides (id,user_id,context_kind,context_id,permission,effect,granted_by_user_id) VALUES ($1,$2,'project',$3,'file.write','deny',$2)")
        .bind(Uuid::new_v4()).bind(fixture.alice.as_uuid()).bind(fixture.project).execute(&fixture.pool).await.unwrap();
    let result = fixture
        .repo
        .publish_change_set(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert!(matches!(result, ChangeSetPublishResult::Conflict { .. }));
    let canonical: Vec<String> = sqlx::query_scalar("SELECT logical_path FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(fixture.project).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(canonical, vec!["a.tex", "b.tex"]);
    close(fixture).await;
}

#[tokio::test]
async fn canonical_main_follows_a_private_rename_without_explicit_set_main() {
    let fixture = fixture().await;
    let workspace: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM latex_core.team_projects WHERE id=$1")
            .bind(fixture.project)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,1,$2,0,'workspace.mutation',1,$3,$4)")
        .bind(workspace).bind(Uuid::new_v4()).bind(serde_json::json!({"schema_version":1,"operations":[{"op":"set_main_file","path":"a.tex"}]})).bind(fixture.alice.as_uuid()).execute(&fixture.pool).await.unwrap();
    sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=1 WHERE workspace_id=$1")
        .bind(workspace)
        .execute(&fixture.pool)
        .await
        .unwrap();
    fixture
        .repo
        .rename_team_file(fixture.alice, fixture.project, "a.tex", "main.tex")
        .await
        .unwrap();
    let result = fixture
        .repo
        .publish_change_set(fixture.alice, fixture.project)
        .await
        .unwrap();
    assert!(matches!(
        result,
        ChangeSetPublishResult::Published {
            workspace_version: 2,
            ..
        }
    ));
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM latex_core.workspace_events WHERE workspace_id=$1 AND sequence=2",
    )
    .bind(workspace)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert!(
        payload["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op == &serde_json::json!({"op":"set_main_file","path":"main.tex"}))
    );
    close(fixture).await;
}

#[tokio::test]
async fn protected_template_instantiation_copies_independent_team_release_state() {
    let fixture = fixture().await;
    let template_id = Uuid::new_v4();
    let template_files = [
        (
            "project.tex",
            b"\\documentclass{VITSCOPEProject}".as_slice(),
        ),
        (
            "VITSCOPEProject.cls",
            b"\\ProvidesClass{VITSCOPEProject}".as_slice(),
        ),
        ("chapters/chapter1.tex", b"Chapter one".as_slice()),
        ("appendix/a.tex", b"Appendix".as_slice()),
        ("images/logo.png", b"PNG".as_slice()),
        ("references.bib", b"@book{x,title={X}}".as_slice()),
        ("notes.tex", b"Notes".as_slice()),
    ];
    let records = template_files
        .iter()
        .map(|(path, bytes)| AppTemplateFileRecord {
            path: (*path).into(),
            blob_hash: BlobHash::digest(bytes),
            size_bytes: u64::try_from(bytes.len()).unwrap(),
        })
        .collect::<Vec<_>>();
    fixture
        .repo
        .create_template(
            template_id,
            &format!("VIT scope {template_id}"),
            None,
            Some("project.tex"),
            &records,
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE latex_core.templates SET main_file_locked=TRUE,strict_structure=TRUE WHERE id=$1",
    )
    .bind(template_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    for (path, policy) in [
        ("project.tex", "managed"),
        ("VITSCOPEProject.cls", "managed"),
        ("chapters/*", "editable"),
        ("appendix/*", "editable"),
        ("images/*", "editable"),
        ("references.bib", "editable"),
    ] {
        sqlx::query("INSERT INTO latex_core.template_policy_rules (template_id,path_pattern,access_policy) VALUES ($1,$2,$3)")
            .bind(template_id).bind(path).bind(policy).execute(&fixture.pool).await.unwrap();
    }
    let workspace = WorkspaceId::new();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace.as_uuid())
        .bind(fixture.tenant)
        .bind(fixture.alice.as_uuid())
        .execute(&fixture.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO latex_core.workspace_heads (workspace_id,durable_version) VALUES ($1,1)",
    )
    .bind(workspace.as_uuid())
    .execute(&fixture.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,1,$2,0,'workspace.mutation',1,$3,$4)")
        .bind(workspace.as_uuid()).bind(Uuid::new_v4()).bind(serde_json::json!({"schema_version":1,"operations":[{"op":"set_main_file","path":"project.tex"}]})).bind(fixture.alice.as_uuid()).execute(&fixture.pool).await.unwrap();
    let project = fixture
        .repo
        .instantiate_team_project_from_template(
            fixture.alice,
            fixture.team,
            workspace,
            "Protected VIT project",
            template_id,
        )
        .await
        .unwrap();
    fixture
        .repo
        .set_project_member(
            fixture.alice,
            project.id,
            fixture.bob,
            persistence::ProjectRoles {
                writer: true,
                mentor: false,
                project_manager: false,
            },
        )
        .await
        .unwrap();
    let metadata: (Uuid, bool, bool, String) = sqlx::query_as("SELECT template_id,main_file_locked,strict_structure,policy_default FROM latex_core.team_projects WHERE id=$1")
        .bind(project.id).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(metadata, (template_id, true, true, "editable".into()));
    let main: String = sqlx::query_scalar("SELECT payload->'operations'->0->>'path' FROM latex_core.workspace_events WHERE workspace_id=$1 AND sequence=1")
        .bind(workspace.as_uuid()).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(main, "project.tex");
    let policies: Vec<(String, String, String)> = sqlx::query_as("SELECT logical_path,access_policy,origin FROM latex_core.file_policies WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(project.id).fetch_all(&fixture.pool).await.unwrap();
    assert!(policies.contains(&(
        String::from("project.tex"),
        String::from("managed"),
        String::from("template")
    )));
    assert!(policies.contains(&(
        String::from("VITSCOPEProject.cls"),
        String::from("managed"),
        String::from("template")
    )));
    assert!(matches!(
        fixture
            .repo
            .save_draft(
                fixture.bob,
                project.id,
                "project.tex",
                1,
                BlobHash::digest(b"change"),
                6
            )
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        fixture
            .repo
            .delete_team_file(fixture.bob, project.id, "project.tex")
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        fixture
            .repo
            .rename_team_file(fixture.bob, project.id, "notes.tex", "project.tex")
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        fixture
            .repo
            .rename_team_file(fixture.bob, project.id, "project.tex", "other.tex")
            .await,
        Err(AppError::Forbidden)
    ));
    assert!(matches!(
        fixture
            .repo
            .set_team_main_file(fixture.bob, project.id, "notes.tex")
            .await,
        Err(AppError::Forbidden)
    ));
    fixture
        .repo
        .save_draft(
            fixture.bob,
            project.id,
            "chapters/chapter1.tex",
            1,
            BlobHash::digest(b"chapter edit"),
            12,
        )
        .await
        .unwrap();
    fixture
        .repo
        .save_draft(
            fixture.bob,
            project.id,
            "images/chart.png",
            0,
            BlobHash::digest(b"chart"),
            5,
        )
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .repo
            .set_file_policy(
                fixture.alice,
                project.id,
                "project.tex",
                FilePolicy::Editable
            )
            .await,
        Err(AppError::Forbidden)
    ));
    let before: (Uuid, bool, bool, Vec<(String, String, String)>) =
        (metadata.0, metadata.1, metadata.2, policies.clone());
    sqlx::query("DELETE FROM latex_core.template_account_types WHERE template_id=$1")
        .bind(template_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    let next_workspace = WorkspaceId::new();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(next_workspace.as_uuid())
        .bind(fixture.tenant)
        .bind(fixture.alice.as_uuid())
        .execute(&fixture.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(next_workspace.as_uuid())
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .repo
            .instantiate_team_project_from_template(
                fixture.alice,
                fixture.team,
                next_workspace,
                "Rejected copy",
                template_id
            )
            .await,
        Err(AppError::NotFound)
    ));
    let after_metadata: (Uuid, bool, bool) = sqlx::query_as("SELECT template_id,main_file_locked,strict_structure FROM latex_core.team_projects WHERE id=$1")
        .bind(project.id).fetch_one(&fixture.pool).await.unwrap();
    let after_policies: Vec<(String, String, String)> = sqlx::query_as("SELECT logical_path,access_policy,origin FROM latex_core.file_policies WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(project.id).fetch_all(&fixture.pool).await.unwrap();
    assert_eq!(after_metadata, (before.0, before.1, before.2));
    assert_eq!(after_policies, before.3);
    fixture
        .repo
        .save_draft(
            fixture.bob,
            project.id,
            "appendix/a.tex",
            1,
            BlobHash::digest(b"appendix edit"),
            13,
        )
        .await
        .unwrap();
    close(fixture).await;
}
