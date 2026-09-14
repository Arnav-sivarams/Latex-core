#![cfg(feature = "database-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test fixtures"
)]

use core_types::{
    BlobHash, CompileKey, LatexmkProfileId, LogicalPath, ShellPolicy, SnapshotId, TenantId,
    TexEngine, TexEnvironmentId, UserId, WorkspaceId,
};
use persistence::{
    CollaborationAccessMode, CollaborationUpdateInput, Database, DatabaseConfig, GlobalRole,
    IntegrationError, IntegrationRepository, PaperStatus, V2BuildRequest, V2Error, V2Repository,
};
use sqlx::PgPool;
use std::{env, time::Duration};
use uuid::Uuid;

static V2_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn global_roles_are_transitional_exclusive_and_database_constrained() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let writer = insert_user(&pool).await;
    let mentor = insert_user(&pool).await;
    let admin = insert_user(&pool).await;
    assert_eq!(
        repo.set_global_role(writer, GlobalRole::Writer)
            .await
            .unwrap()
            .role,
        GlobalRole::Writer
    );
    assert_eq!(
        repo.set_global_role(mentor, GlobalRole::Mentor)
            .await
            .unwrap()
            .role,
        GlobalRole::Mentor
    );
    assert_eq!(
        repo.set_global_role(admin, GlobalRole::Admin)
            .await
            .unwrap()
            .role,
        GlobalRole::Admin
    );
    assert_eq!(
        repo.get_global_role(writer).await.unwrap().unwrap().role,
        GlobalRole::Writer
    );

    let invalid_user = insert_user(&pool).await;
    assert_constraint(
        sqlx::query(
            "INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,'project_manager')",
        )
        .bind(invalid_user.as_uuid())
        .execute(&pool)
        .await,
    );
    assert!(repo.get_global_role(invalid_user).await.unwrap().is_none());

    let exclusive_user = insert_user(&pool).await;
    sqlx::query("INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,'writer')")
        .bind(exclusive_user.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert_constraint(
        sqlx::query("INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,'mentor')")
            .bind(exclusive_user.as_uuid())
            .execute(&pool)
            .await,
    );

    let unassigned = insert_user(&pool).await;
    assert!(repo.get_global_role(unassigned).await.unwrap().is_none());
    let removable = insert_user(&pool).await;
    repo.set_global_role(removable, GlobalRole::Mentor)
        .await
        .unwrap();
    repo.remove_global_role(removable).await.unwrap();
    assert!(repo.get_global_role(removable).await.unwrap().is_none());

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn successful_role_operations_revoke_sessions_and_rejections_are_atomic() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let (user, tenant) = insert_user_with_tenant(&pool).await;
    insert_session(&pool, user).await;
    repo.set_global_role(user, GlobalRole::Writer)
        .await
        .unwrap();
    assert_eq!(session_count(&pool, user).await, 0);

    insert_session(&pool, user).await;
    repo.set_global_role(user, GlobalRole::Mentor)
        .await
        .unwrap();
    assert_eq!(session_count(&pool, user).await, 0);

    insert_session(&pool, user).await;
    repo.remove_global_role(user).await.unwrap();
    assert_eq!(session_count(&pool, user).await, 0);

    repo.set_global_role(user, GlobalRole::Writer)
        .await
        .unwrap();
    let workspace = insert_workspace(&pool, tenant, user).await;
    repo.create_personal_paper(user, workspace, "Atomic role transition")
        .await
        .unwrap();
    insert_session(&pool, user).await;
    assert!(matches!(
        repo.set_global_role(user, GlobalRole::Mentor).await,
        Err(V2Error::PersonalPaperOwnershipConflict { .. })
    ));
    assert_eq!(session_count(&pool, user).await, 1);
    assert_eq!(
        repo.get_global_role(user).await.unwrap().unwrap().role,
        GlobalRole::Writer
    );

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn personal_papers_are_writer_owned_and_workspace_unique() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let (writer, writer_tenant) = insert_user_with_tenant(&pool).await;
    let (mentor, mentor_tenant) = insert_user_with_tenant(&pool).await;
    let (admin, admin_tenant) = insert_user_with_tenant(&pool).await;
    repo.set_global_role(writer, GlobalRole::Writer)
        .await
        .unwrap();
    repo.set_global_role(mentor, GlobalRole::Mentor)
        .await
        .unwrap();
    repo.set_global_role(admin, GlobalRole::Admin)
        .await
        .unwrap();

    let writer_workspace = insert_workspace(&pool, writer_tenant, writer).await;
    let paper = repo
        .create_personal_paper(writer, writer_workspace, "Writer paper")
        .await
        .unwrap();
    assert_eq!(paper.owner_user_id, writer);
    assert_eq!(paper.workspace_id, writer_workspace);
    assert_eq!(paper.status, PaperStatus::Active);
    assert_eq!(repo.personal_paper(paper.id).await.unwrap(), paper);
    assert_eq!(
        repo.list_personal_papers(writer).await.unwrap(),
        vec![paper.clone()]
    );

    let mentor_workspace = insert_workspace(&pool, mentor_tenant, mentor).await;
    assert!(matches!(
        repo.create_personal_paper(mentor, mentor_workspace, "Forbidden mentor paper")
            .await,
        Err(V2Error::RoleForbidden { .. })
    ));
    let admin_workspace = insert_workspace(&pool, admin_tenant, admin).await;
    assert!(matches!(
        repo.create_personal_paper(admin, admin_workspace, "Forbidden admin paper")
            .await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert!(matches!(
        repo.create_personal_paper(writer, writer_workspace, "Duplicate workspace")
            .await,
        Err(V2Error::WorkspaceConflict { .. })
    ));
    let (unassigned, unassigned_tenant) = insert_user_with_tenant(&pool).await;
    let unassigned_workspace = insert_workspace(&pool, unassigned_tenant, unassigned).await;
    assert!(matches!(
        repo.create_personal_paper(unassigned, unassigned_workspace, "Missing role")
            .await,
        Err(V2Error::RoleMissing { .. })
    ));

    assert!(matches!(
        repo.set_global_role(writer, GlobalRole::Mentor).await,
        Err(V2Error::PersonalPaperOwnershipConflict { .. })
    ));
    assert!(matches!(
        repo.set_global_role(writer, GlobalRole::Admin).await,
        Err(V2Error::PersonalPaperOwnershipConflict { .. })
    ));
    assert_eq!(
        repo.get_global_role(writer).await.unwrap().unwrap().role,
        GlobalRole::Writer
    );

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn paper_teams_and_membership_separate_access_from_global_role() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let (admin, admin_tenant) = insert_user_with_tenant(&pool).await;
    let (writer, writer_tenant) = insert_user_with_tenant(&pool).await;
    let second_writer = insert_user(&pool).await;
    let unassigned_writer = insert_user(&pool).await;
    let (mentor, mentor_tenant) = insert_user_with_tenant(&pool).await;
    repo.set_global_role(admin, GlobalRole::Admin)
        .await
        .unwrap();
    repo.set_global_role(writer, GlobalRole::Writer)
        .await
        .unwrap();
    repo.set_global_role(second_writer, GlobalRole::Writer)
        .await
        .unwrap();
    repo.set_global_role(mentor, GlobalRole::Mentor)
        .await
        .unwrap();

    let team_workspace = insert_workspace(&pool, admin_tenant, admin).await;
    let team = repo
        .create_paper_team(admin, team_workspace, "One paper team", writer)
        .await
        .unwrap();
    assert_eq!(team.workspace_id, team_workspace);
    assert_eq!(team.created_by_user_id, admin);
    assert_eq!(repo.paper_team(team.id).await.unwrap(), team);
    assert!(
        repo.writer_paper(writer, team.id)
            .await
            .unwrap()
            .is_team_leader
    );
    assert_team_statuses(&repo, team.id).await;

    let writer_workspace = insert_workspace(&pool, writer_tenant, writer).await;
    assert!(matches!(
        repo.create_paper_team(writer, writer_workspace, "Writer team", writer)
            .await,
        Err(V2Error::RoleForbidden { .. })
    ));
    let mentor_workspace = insert_workspace(&pool, mentor_tenant, mentor).await;
    assert!(matches!(
        repo.create_paper_team(mentor, mentor_workspace, "Mentor team", writer)
            .await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert!(matches!(
        repo.create_paper_team(admin, team_workspace, "Duplicate workspace", writer)
            .await,
        Err(V2Error::WorkspaceConflict { .. })
    ));

    repo.add_paper_team_member(team.id, second_writer, admin)
        .await
        .unwrap();
    repo.add_paper_team_member(team.id, mentor, admin)
        .await
        .unwrap();
    assert!(matches!(
        repo.add_paper_team_member(team.id, admin, admin).await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert!(matches!(
        repo.add_paper_team_member(team.id, writer, admin).await,
        Err(V2Error::Conflict { .. })
    ));
    let members = repo.list_paper_team_members(team.id).await.unwrap();
    assert_eq!(members.len(), 3);
    assert!(
        members
            .iter()
            .any(|member| member.user_id == writer && member.is_leader)
    );
    assert!(
        members
            .iter()
            .any(|member| member.user_id == second_writer && !member.is_leader)
    );
    assert!(members.iter().any(|member| member.user_id == mentor));

    assert_membership_has_only_team_leader_capability(&pool).await;

    assert!(matches!(
        repo.change_paper_team_leader(team.id, mentor, admin).await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert!(matches!(
        repo.change_paper_team_leader(team.id, admin, admin).await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert!(matches!(
        repo.change_paper_team_leader(team.id, unassigned_writer, admin)
            .await,
        Err(V2Error::RoleMissing { .. })
    ));
    let outside_writer = insert_user(&pool).await;
    repo.set_global_role(outside_writer, GlobalRole::Writer)
        .await
        .unwrap();
    assert!(matches!(
        repo.change_paper_team_leader(team.id, outside_writer, admin)
            .await,
        Err(V2Error::RoleForbidden { .. })
    ));
    assert_constraint(
        sqlx::query(
            "UPDATE latex_core.paper_team_members SET is_leader=TRUE \
             WHERE paper_team_id=$1 AND user_id=$2",
        )
        .bind(team.id)
        .bind(second_writer.as_uuid())
        .execute(&pool)
        .await,
    );
    let changed = repo
        .change_paper_team_leader(team.id, second_writer, admin)
        .await
        .unwrap();
    assert!(changed.is_leader);
    assert!(
        repo.writer_paper(second_writer, team.id)
            .await
            .unwrap()
            .is_team_leader
    );
    assert!(
        !repo
            .writer_paper(writer, team.id)
            .await
            .unwrap()
            .is_team_leader
    );
    assert!(matches!(
        repo.remove_paper_team_member(team.id, second_writer, admin)
            .await,
        Err(V2Error::Conflict { .. })
    ));
    assert!(matches!(
        repo.set_global_role(second_writer, GlobalRole::Mentor)
            .await,
        Err(V2Error::TeamMembershipConflict { .. })
    ));

    assert!(matches!(
        repo.set_global_role(writer, GlobalRole::Admin).await,
        Err(V2Error::TeamMembershipConflict { .. })
    ));
    assert!(
        repo.list_paper_team_members(team.id)
            .await
            .unwrap()
            .iter()
            .any(|member| member.user_id == writer)
    );
    repo.remove_paper_team_member(team.id, writer, admin)
        .await
        .unwrap();
    repo.set_global_role(writer, GlobalRole::Admin)
        .await
        .unwrap();

    let free_mentor = insert_user(&pool).await;
    repo.set_global_role(free_mentor, GlobalRole::Mentor)
        .await
        .unwrap();
    repo.set_global_role(free_mentor, GlobalRole::Writer)
        .await
        .unwrap();
    assert_eq!(
        repo.get_global_role(free_mentor)
            .await
            .unwrap()
            .unwrap()
            .role,
        GlobalRole::Writer
    );

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn paper_file_identity_survives_rename_and_supports_safe_path_reuse() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let (writer_a, tenant_a) = insert_user_with_tenant(&pool).await;
    let (writer_b, tenant_b) = insert_user_with_tenant(&pool).await;
    repo.set_global_role(writer_a, GlobalRole::Writer)
        .await
        .unwrap();
    repo.set_global_role(writer_b, GlobalRole::Writer)
        .await
        .unwrap();
    let workspace_a = insert_workspace(&pool, tenant_a, writer_a).await;
    let workspace_b = insert_workspace(&pool, tenant_b, writer_b).await;
    repo.create_personal_paper(writer_a, workspace_a, "Paper A")
        .await
        .unwrap();
    repo.create_personal_paper(writer_b, workspace_b, "Paper B")
        .await
        .unwrap();

    let original_path = LogicalPath::parse("chapters/introduction.tex").unwrap();
    let renamed_path = LogicalPath::parse("sections/introduction.tex").unwrap();
    let file = repo
        .register_paper_file(workspace_a, original_path.clone())
        .await
        .unwrap();
    assert_ne!(file.file_id, Uuid::nil());
    assert_eq!(repo.paper_file(file.file_id).await.unwrap(), file);
    let renamed = repo
        .rename_paper_file(file.file_id, renamed_path.clone())
        .await
        .unwrap();
    assert_eq!(renamed.file_id, file.file_id);
    assert_eq!(renamed.path, renamed_path);
    assert_eq!(renamed.revision, file.revision + 1);
    assert!(
        repo.resolve_live_paper_file(workspace_a, &original_path)
            .await
            .unwrap()
            .is_none()
    );

    assert!(matches!(
        repo.register_paper_file(workspace_a, renamed_path.clone())
            .await,
        Err(V2Error::DuplicateFilePath { .. })
    ));
    let other_workspace_file = repo
        .register_paper_file(workspace_b, renamed_path.clone())
        .await
        .unwrap();
    assert_ne!(other_workspace_file.file_id, file.file_id);

    let tombstoned = repo.tombstone_paper_file(file.file_id).await.unwrap();
    assert!(tombstoned.tombstoned);
    assert!(tombstoned.tombstoned_at.is_some());
    let replacement = repo
        .register_paper_file(workspace_a, renamed_path.clone())
        .await
        .unwrap();
    assert_ne!(replacement.file_id, file.file_id);
    assert_eq!(
        repo.resolve_live_paper_file(workspace_a, &renamed_path)
            .await
            .unwrap()
            .unwrap()
            .file_id,
        replacement.file_id
    );

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn statuses_are_closed_and_v1_schema_remains_independent() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;

    let (writer, tenant) = insert_user_with_tenant(&pool).await;
    repo.set_global_role(writer, GlobalRole::Writer)
        .await
        .unwrap();
    let workspace = insert_workspace(&pool, tenant, writer).await;
    let paper = repo
        .create_personal_paper(writer, workspace, "Status paper")
        .await
        .unwrap();
    for status in [
        PaperStatus::Active,
        PaperStatus::Frozen,
        PaperStatus::Submitted,
        PaperStatus::Archived,
    ] {
        assert_eq!(
            repo.set_personal_paper_status(paper.id, status)
                .await
                .unwrap()
                .status,
            status
        );
    }
    assert_constraint(
        sqlx::query("UPDATE latex_core.personal_papers SET status='invalid' WHERE id=$1")
            .bind(paper.id)
            .execute(&pool)
            .await,
    );

    for table in [
        "users",
        "projects",
        "teams",
        "team_projects",
        "research_groups",
        "member_drafts",
        "global_user_roles",
        "personal_papers",
        "paper_teams",
        "paper_team_members",
        "paper_files",
    ] {
        let qualified = format!("latex_core.{table}");
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(qualified)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(exists, "expected {table} to exist");
    }

    let (legacy_owner, legacy_tenant) = insert_user_with_tenant(&pool).await;
    let legacy_workspace = insert_workspace(&pool, legacy_tenant, legacy_owner).await;
    sqlx::query(
        "INSERT INTO latex_core.projects (workspace_id,owner_user_id,name) VALUES ($1,$2,'Legacy project')",
    )
    .bind(legacy_workspace.as_uuid())
    .bind(legacy_owner.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo.get_global_role(legacy_owner).await.unwrap().is_none());
    let v2_paper_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.personal_papers WHERE workspace_id=$1")
            .bind(legacy_workspace.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(v2_paper_count, 0);

    pool.close().await;
    database.close().await;
}

#[tokio::test]
async fn manual_build_review_lock_and_integration_reads_share_exact_state() {
    let _guard = V2_TEST_LOCK.lock().await;
    let (database, pool, repo) = connect().await;
    let (admin, tenant) = insert_user_with_tenant(&pool).await;
    let writer = insert_user(&pool).await;
    let writer_two = insert_user(&pool).await;
    let mentor = insert_user(&pool).await;
    for (user, role, label) in [
        (admin, GlobalRole::Admin, "admin"),
        (writer, GlobalRole::Writer, "writer-a"),
        (writer_two, GlobalRole::Writer, "writer-b"),
        (mentor, GlobalRole::Mentor, "mentor"),
    ] {
        repo.set_global_role(user, role).await.unwrap();
        sqlx::query(
            "INSERT INTO latex_core.user_credentials (user_id,email,password_hash) VALUES ($1,$2,'test-only-hash')",
        )
        .bind(user.as_uuid())
        .bind(format!("{label}-{}@example.test", user.as_uuid()))
        .execute(&pool)
        .await
        .unwrap();
    }
    let workspace = insert_workspace(&pool, tenant, admin).await;
    let team = repo
        .create_paper_team(admin, workspace, "Populated review-lock report", writer)
        .await
        .unwrap();
    repo.add_paper_team_member(team.id, writer_two, admin)
        .await
        .unwrap();
    repo.add_paper_team_member(team.id, mentor, admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.workspace_heads (workspace_id) VALUES ($1)")
        .bind(workspace.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let (file, _) = repo
        .create_file_with_event(
            workspace,
            writer,
            0,
            LogicalPath::parse("main.tex").unwrap(),
            "0".repeat(64).parse().unwrap(),
            32,
        )
        .await
        .unwrap();
    let starting_version: i64 = sqlx::query_scalar(
        "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1",
    )
    .bind(workspace.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let jobs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let source_bytes = b"\\documentclass{article}\n\\begin{document}Qualified\\end{document}\n";
    let source_hash = BlobHash::digest(source_bytes);
    let (_, source_version) = repo
        .save_file_with_event(
            file.file_id,
            writer,
            u64::try_from(starting_version).unwrap(),
            source_hash,
            u64::try_from(source_bytes.len()).unwrap(),
        )
        .await
        .unwrap();
    let jobs_after_save: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        jobs_after_save, jobs_before,
        "ordinary save must enqueue zero builds"
    );

    let snapshot_text = "2".repeat(64);
    sqlx::query("INSERT INTO latex_core.snapshots (snapshot_id,manifest_blob_hash) VALUES ($1,$1) ON CONFLICT DO NOTHING")
        .bind(&snapshot_text)
        .execute(&pool)
        .await
        .unwrap();
    let state_hash = snapshot_text.clone();
    let manifest = serde_json::json!({
        "schema_version":1,
        "workspace":{"files":{"main.tex":{"blob_hash":source_hash.to_hex(),"size_bytes":source_bytes.len()}}},
        "file_identities":[{"file_id":file.file_id,"path":"main.tex"}],
        "template_policy_provenance":{"file_policies":[{"file_id":file.file_id,"policy":"EDITABLE"}]},
        "front_matter":{"status":"not_configured"},
        "front_matter_resolved":{"status":"not_configured","fields":[]}
    });
    let request = V2BuildRequest {
        paper_id: team.id,
        workspace_id: workspace,
        document_epoch: 1,
        source_sequence: source_version,
        snapshot_id: snapshot_text.parse::<SnapshotId>().unwrap(),
        manifest,
        state_hash: state_hash.clone(),
        tenant_id: tenant,
        user_id: writer,
        trigger_type: "manual".into(),
        compile_key: "4".repeat(64).parse::<CompileKey>().unwrap(),
        engine: TexEngine::PdfLatex,
        tex_environment_id: TexEnvironmentId::parse("test-frozen-m7").unwrap(),
        latexmk_profile: LatexmkProfileId::parse("safe-v1").unwrap(),
        shell_policy: ShellPolicy::Safe,
        synctex: true,
    };
    let submission = repo.submit_v2_build(&request).await.unwrap();
    let build_id = submission.build_id.unwrap();
    let (job_id, version_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT compile_job_id,version_id FROM latex_core.v2_paper_builds WHERE id=$1",
    )
    .bind(build_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE latex_core.compile_jobs SET state='succeeded',finished_at=statement_timestamp() WHERE id=$1")
        .bind(job_id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE latex_core.v2_paper_builds SET status='succeeded' WHERE id=$1")
        .bind(build_id)
        .execute(&pool)
        .await
        .unwrap();
    let pdf_bytes = b"%PDF-1.4\n% qualification artifact\n%%EOF\n";
    for (kind, name, bytes, content_type) in [
        ("pdf", "main.pdf", pdf_bytes.as_slice(), "application/pdf"),
        ("log", "main.log", b"ok\n".as_slice(), "text/plain"),
        (
            "synctex",
            "main.synctex.gz",
            b"gz".as_slice(),
            "application/gzip",
        ),
    ] {
        let hash = BlobHash::digest(bytes).to_hex();
        sqlx::query("INSERT INTO latex_core.compilation_artifacts (artifact_id,job_id,compile_key,kind,logical_name,blob_hash,size_bytes,content_type) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(Uuid::new_v4()).bind(job_id).bind("4".repeat(64)).bind(kind).bind(name).bind(hash).bind(i64::try_from(bytes.len()).unwrap()).bind(content_type)
            .execute(&pool).await.unwrap();
    }
    sqlx::query("UPDATE latex_core.v2_paper_build_state SET active_build_id=NULL,current_build_id=$2 WHERE workspace_id=$1")
        .bind(workspace.as_uuid()).bind(build_id).execute(&pool).await.unwrap();

    let (round, created) = repo
        .open_review_round(writer, team.id, &state_hash)
        .await
        .unwrap();
    assert!(created);
    let round_id = Uuid::parse_str(round["id"].as_str().unwrap()).unwrap();
    for actor in [writer, writer_two] {
        assert_eq!(
            repo.collaboration_access(actor, team.id, file.file_id)
                .await
                .unwrap()
                .mode,
            CollaborationAccessMode::ReadOnly
        );
        assert!(matches!(
            repo.save_file_with_event(
                file.file_id,
                actor,
                source_version,
                "8".repeat(64).parse().unwrap(),
                3
            )
            .await,
            Err(V2Error::Conflict {
                entity: "paper under review"
            })
        ));
    }
    let yjs_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.collaboration_updates WHERE workspace_id=$1",
    )
    .bind(workspace.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(matches!(
        repo.persist_collaboration_batch(
            workspace,
            file.file_id,
            1,
            &[CollaborationUpdateInput {
                actor_user_id: writer,
                update_bytes: vec![1, 2, 3]
            }],
            "a".repeat(64).parse().unwrap(),
            3
        )
        .await,
        Err(V2Error::Conflict {
            entity: "paper under review"
        })
    ));
    let yjs_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.collaboration_updates WHERE workspace_id=$1",
    )
    .bind(workspace.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        yjs_after, yjs_before,
        "denied Yjs update must not reach durable shared state"
    );
    assert!(matches!(
        repo.submit_v2_build(&request).await,
        Err(V2Error::Conflict {
            entity: "paper under review"
        })
    ));
    let jobs_during_review: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.compile_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        jobs_during_review,
        jobs_before + 1,
        "denied review operations enqueue zero builds"
    );

    repo.close_review_round(writer, team.id, round_id)
        .await
        .unwrap();
    assert_eq!(
        repo.collaboration_access(writer_two, team.id, file.file_id)
            .await
            .unwrap()
            .mode,
        CollaborationAccessMode::ReadWrite
    );
    repo.save_file_with_event(
        file.file_id,
        writer_two,
        source_version,
        "9".repeat(64).parse().unwrap(),
        3,
    )
    .await
    .unwrap();

    let integration = IntegrationRepository::new(database.clone());
    let scopes = vec![
        "reports.read".to_owned(),
        "reports.files.read".to_owned(),
        "reports.pdf.read".to_owned(),
    ];
    let mut token_hash = team.id.as_bytes().to_vec();
    token_hash.extend_from_slice(team.id.as_bytes());
    let client = integration
        .create_client(
            admin,
            &format!("qualification archive {}", team.id),
            "lcint_qualification",
            &token_hash,
            &scopes,
            false,
            &[team.id],
            None,
        )
        .await
        .unwrap();
    let principal = integration.authenticate(&token_hash).await.unwrap();
    assert!(principal.allows_report(team.id));
    assert_eq!(
        integration
            .reports(&principal, None, 2, None, None, None, None)
            .await
            .unwrap()
            .len(),
        1
    );
    let projected = integration.report(team.id, false).await.unwrap();
    assert!(
        projected["relationships"]["members"]
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
    assert!(
        projected["relationships"]["members"][0]
            .get("email")
            .is_none()
    );
    let versions = integration.versions(team.id, None, 10).await.unwrap();
    assert!(
        versions
            .iter()
            .any(|version| version["id"] == version_id.to_string())
    );
    let builds = integration.builds(team.id, None, 10).await.unwrap();
    assert_eq!(builds.len(), 1);
    assert!(
        !builds[0]["is_current"].as_bool().unwrap(),
        "post-build edit makes PDF stale"
    );
    assert_eq!(
        integration.pdf(team.id, build_id).await.unwrap()["version_id"],
        version_id.to_string()
    );
    integration
        .admit_read(client.id, "reports.read")
        .await
        .unwrap();
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.integration_access_log WHERE client_id=$1",
    )
    .bind(client.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1);
    integration.revoke_client(admin, client.id).await.unwrap();
    assert!(matches!(
        integration.authenticate(&token_hash).await,
        Err(IntegrationError::Forbidden)
    ));

    pool.close().await;
    database.close().await;
}

async fn connect() -> (Database, PgPool, V2Repository) {
    let url = env::var("TEST_DATABASE_URL")
        .expect("database-tests requires TEST_DATABASE_URL; use ./scripts/test-db.sh");
    let database =
        Database::connect(DatabaseConfig::new(&url, 1, 4, Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
    database.migrate().await.unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let repo = V2Repository::new(database.clone());
    (database, pool, repo)
}

async fn insert_user(pool: &PgPool) -> UserId {
    insert_user_with_tenant(pool).await.0
}

async fn insert_user_with_tenant(pool: &PgPool) -> (UserId, TenantId) {
    let tenant_id = TenantId::new();
    let user_id = UserId::new();
    sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
        .bind(tenant_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    (user_id, tenant_id)
}

async fn insert_workspace(
    pool: &PgPool,
    tenant_id: TenantId,
    owner_user_id: UserId,
) -> WorkspaceId {
    let workspace_id = WorkspaceId::new();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .bind(owner_user_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    workspace_id
}

async fn insert_session(pool: &PgPool, user_id: UserId) {
    let token_digest = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO latex_core.sessions (token_digest,user_id,expires_at) \
         VALUES ($1,$2,statement_timestamp()+interval '1 hour')",
    )
    .bind(token_digest)
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .unwrap();
}

async fn session_count(pool: &PgPool, user_id: UserId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM latex_core.sessions WHERE user_id=$1")
        .bind(user_id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

fn assert_constraint<T>(result: Result<T, sqlx::Error>) {
    let Err(error) = result else {
        panic!("database constraint should reject the write");
    };
    assert!(error.as_database_error().is_some(), "{error:?}");
}

async fn assert_team_statuses(repo: &V2Repository, team_id: Uuid) {
    for status in [
        PaperStatus::Frozen,
        PaperStatus::Submitted,
        PaperStatus::Archived,
        PaperStatus::Active,
    ] {
        assert_eq!(
            repo.set_paper_team_status(team_id, status)
                .await
                .unwrap()
                .status,
            status
        );
    }
}

async fn assert_membership_has_only_team_leader_capability(pool: &PgPool) {
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema='latex_core' AND table_name='paper_team_members' ORDER BY ordinal_position",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        columns,
        vec![
            "paper_team_id",
            "user_id",
            "assigned_by_user_id",
            "created_at",
            "is_leader",
            "writer_order"
        ]
    );
}
