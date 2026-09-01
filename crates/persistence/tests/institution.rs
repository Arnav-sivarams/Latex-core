#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "isolated PostgreSQL integration fixture"
)]

use core_types::{BlobHash, LogicalPath, TenantId, UserId, WorkspaceId};
use persistence::{
    Database, DatabaseConfig, ImportLimits, ImportMode, InstitutionRepository, PaperTeamPageFilter,
    TeamTemplateResolutionInput, TemplateSeedFile, V2FilePolicy, V2Repository,
};
use sqlx::PgPool;
use std::{env, fmt::Write as _, time::Duration};
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn institutional_schema_import_identity_template_and_scale_contract() {
    let _guard = TEST_LOCK.lock().await;
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(
        DatabaseConfig::new(&url, 1, 8, Duration::from_secs(10)).expect("valid test config"),
    )
    .await
    .expect("database connects");
    database.migrate().await.expect("migration succeeds");
    let pool = PgPool::connect(&url).await.expect("test pool connects");
    let (actor, tenant) = insert_user(&pool, "v22-admin@example.edu", "admin").await;
    let repository = InstitutionRepository::new(database.clone());
    let v2 = V2Repository::new(database.clone());

    verify_vcap_schema_and_department_role_guard(&pool).await;

    sqlx::query("INSERT INTO vcap.programmes (programme_code) VALUES ('CSE'),('ECE'),('SCALE') ON CONFLICT DO NOTHING")
        .execute(&pool).await.unwrap();
    let fallback = insert_template(&pool, "V2.2 fallback").await;
    let cse_template = insert_template(&pool, "V2.2 CSE").await;
    repository
        .set_global_fallback(actor, fallback)
        .await
        .unwrap();
    repository
        .set_programme_template_default(actor, "CSE", cse_template)
        .await
        .unwrap();

    identity_linking_is_safe(&pool, &repository, actor).await;
    resolver_is_deterministic_and_pins_are_immutable(
        &pool,
        &repository,
        actor,
        tenant,
        fallback,
        cse_template,
    )
    .await;
    imported_team_materialization_is_ordered_atomic_and_isolated(
        &pool,
        &repository,
        &v2,
        actor,
        tenant,
        cse_template,
    )
    .await;
    import_modes_and_ten_thousand_rows(&pool, &repository, actor).await;

    pool.close().await;
    database.close().await;
}

async fn verify_vcap_schema_and_department_role_guard(pool: &PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema='vcap' ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for required in [
        "departments",
        "admins",
        "faculty",
        "programmes",
        "schools",
        "students",
        "student_course_registrations",
        "faculty_guide_capacity",
        "department_roles",
        "faculty_roles",
        "student_user_links",
        "faculty_user_links",
        "admin_user_links",
        "paper_assignment_groups",
        "paper_assignment_students",
        "paper_assignment_mentors",
    ] {
        assert!(
            tables.iter().any(|table| table == required),
            "missing {required}"
        );
    }
    let department = Uuid::new_v4();
    sqlx::query("INSERT INTO vcap.departments (department_id) VALUES ($1)")
        .bind(department)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("INSERT INTO vcap.department_roles (dept_id) VALUES ('not-a-uuid')")
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO vcap.department_roles (dept_id) VALUES ($1)")
            .bind(Uuid::new_v4().to_string())
            .execute(pool)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO vcap.department_roles (dept_id) VALUES ($1)")
        .bind(department.to_string())
        .execute(pool)
        .await
        .unwrap();
}

async fn identity_linking_is_safe(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
) {
    let (writer, _) = insert_user(pool, "linked-writer@example.edu", "writer").await;
    let students =
        b"reg_no,name,email,programme_code\nLINKED,Linked,linked-writer@example.edu,CSE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            students,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let linked: (String, Option<Uuid>) =
        sqlx::query_as("SELECT status,user_id FROM vcap.student_user_links WHERE reg_no='LINKED'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(linked, ("LINKED".into(), Some(*writer.as_uuid())));

    let faculty = b"faculty_id,email\nF-WRONG,linked-writer@example.edu\n";
    let job = repository
        .validate_upload(
            actor,
            "faculty.csv",
            None,
            ImportMode::Merge,
            faculty,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM vcap.faculty_user_links WHERE faculty_id='F-WRONG'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "ROLE_INCOMPATIBLE");

    let admins = b"admin_id,email\nA-WRONG,linked-writer@example.edu\n";
    let job = repository
        .validate_upload(
            actor,
            "admins.csv",
            None,
            ImportMode::Merge,
            admins,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM vcap.admin_user_links WHERE admin_id='A-WRONG'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "ROLE_INCOMPATIBLE");
    let role: String =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(writer.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(role, "writer");

    insert_user(pool, "duplicate@example.edu", "writer").await;
    insert_user(pool, "DUPLICATE@example.edu", "writer").await;
    let ambiguous = b"reg_no,name,email,programme_code\nAMB,Ambiguous,duplicate@example.edu,CSE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            ambiguous,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM vcap.student_user_links WHERE reg_no='AMB'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "AMBIGUOUS");

    let (manual_user, _) = insert_user(pool, "manual-link@example.edu", "writer").await;
    insert_user(pool, "automatic-candidate@example.edu", "writer").await;
    sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ('MANUAL-LINK','manual-link@example.edu','CSE')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.student_user_links (reg_no,user_id,match_method,status,linked_at) VALUES ('MANUAL-LINK',$1,'MANUAL','LINKED',now())")
        .bind(manual_user.as_uuid()).execute(pool).await.unwrap();
    let changed_email = b"reg_no,name,email,programme_code\nMANUAL-LINK,Manual,automatic-candidate@example.edu,CSE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            changed_email,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let manual_link: (Uuid, String) = sqlx::query_as(
        "SELECT user_id,match_method FROM vcap.student_user_links WHERE reg_no='MANUAL-LINK'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(manual_link, (*manual_user.as_uuid(), "MANUAL".into()));
}

async fn resolver_is_deterministic_and_pins_are_immutable(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
    tenant: Uuid,
    fallback: Uuid,
    cse_template: Uuid,
) {
    let (cse_a, _) = insert_user(pool, "cse-a@example.edu", "writer").await;
    let (cse_b, _) = insert_user(pool, "cse-b@example.edu", "writer").await;
    let (ece, _) = insert_user(pool, "ece@example.edu", "writer").await;
    for (reg_no, email, programme, user) in [
        ("CSE-A", "cse-a@example.edu", "CSE", cse_a),
        ("CSE-B", "cse-b@example.edu", "CSE", cse_b),
        ("ECE-A", "ece@example.edu", "ECE", ece),
    ] {
        sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ($1,$2,$3)")
            .bind(reg_no)
            .bind(email)
            .bind(programme)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO vcap.student_user_links (reg_no,user_id,match_method,status,linked_at) VALUES ($1,$2,'TEST','LINKED',now())")
            .bind(reg_no).bind(user.as_uuid()).execute(pool).await.unwrap();
    }
    let mode = repository
        .resolve_default_template_for_writers(&[cse_a, cse_b, ece])
        .await
        .unwrap();
    assert_eq!(mode.dominant_programme_code.as_deref(), Some("CSE"));
    assert_eq!(mode.resolution_method, "MODE");
    assert_eq!(mode.selected_template_id, cse_template);
    let tie = repository
        .resolve_default_template_for_writers(&[ece, cse_a])
        .await
        .unwrap();
    assert_eq!(tie.dominant_programme_code.as_deref(), Some("ECE"));
    assert_eq!(tie.resolution_method, "GLOBAL_FALLBACK");
    assert_eq!(tie.selected_template_id, fallback);
    assert!(tie.tie_break.is_some());
    let missing = repository
        .resolve_default_template_for_writers(&[UserId::new()])
        .await
        .unwrap();
    assert_eq!(missing.selected_template_id, fallback);
    assert!(!missing.warnings.is_empty());

    let workspace = Uuid::new_v4();
    let paper = Uuid::new_v4();
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace)
        .bind(tenant)
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.paper_teams (id,workspace_id,name,created_by_user_id) VALUES ($1,$2,'Pinned',$3)")
        .bind(paper).bind(workspace).bind(actor.as_uuid()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO latex_core.paper_template_resolutions (paper_team_id,selected_template_id,dominant_programme_code,resolution_method,manual_override) VALUES ($1,$2,'CSE','MODE',FALSE)")
        .bind(paper).bind(cse_template).execute(pool).await.unwrap();
    repository
        .set_programme_template_default(actor, "CSE", fallback)
        .await
        .unwrap();
    let selected: Uuid = sqlx::query_scalar("SELECT selected_template_id FROM latex_core.paper_template_resolutions WHERE paper_team_id=$1")
        .bind(paper).fetch_one(pool).await.unwrap();
    assert_eq!(selected, cse_template);
}

async fn imported_team_materialization_is_ordered_atomic_and_isolated(
    pool: &PgPool,
    repository: &InstitutionRepository,
    v2: &V2Repository,
    actor: UserId,
    tenant: Uuid,
    template_id: Uuid,
) {
    let (writer_a, _) = insert_user(pool, "team-a@example.edu", "writer").await;
    let (writer_b, _) = insert_user(pool, "team-b@example.edu", "writer").await;
    let (mentor, _) = insert_user(pool, "team-mentor@example.edu", "mentor").await;
    sqlx::query("INSERT INTO vcap.faculty (faculty_id,email) VALUES ('TEAM-MENTOR','team-mentor@example.edu')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.faculty_user_links (faculty_id,user_id,match_method,status,linked_at) VALUES ('TEAM-MENTOR',$1,'TEST','LINKED',now())")
        .bind(mentor.as_uuid()).execute(pool).await.unwrap();
    for (reg_no, email, user) in [
        ("TEAM-A", "team-a@example.edu", writer_a),
        ("TEAM-B", "team-b@example.edu", writer_b),
    ] {
        sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ($1,$2,'CSE')")
            .bind(reg_no)
            .bind(email)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO vcap.student_user_links (reg_no,user_id,match_method,status,linked_at) VALUES ($1,$2,'TEST','LINKED',now())")
            .bind(reg_no).bind(user.as_uuid()).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ('TEAM-UNRESOLVED','nobody@example.edu','CSE')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.paper_assignment_groups (external_team_key,team_name) VALUES ('EXT-VALID','Imported valid'),('EXT-BLOCKED','Imported blocked')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.paper_assignment_students (external_team_key,student_reg_no,writer_order,is_leader) VALUES ('EXT-VALID','TEAM-B',1,FALSE),('EXT-VALID','TEAM-A',2,TRUE),('EXT-BLOCKED','TEAM-UNRESOLVED',1,TRUE)")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.paper_assignment_mentors (external_team_key,faculty_id) VALUES ('EXT-VALID','TEAM-MENTOR')")
        .execute(pool).await.unwrap();
    let job = Uuid::new_v4();
    sqlx::query("INSERT INTO latex_core.institution_import_jobs (id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id,status) VALUES ($1,'workbook','MERGE','teams.xlsx',$2,'XLSX',$3,'APPLIED')")
        .bind(job).bind("a".repeat(64)).bind(actor.as_uuid()).execute(pool).await.unwrap();
    for (row_number, key) in [(2_i64, "EXT-VALID"), (3, "EXT-BLOCKED")] {
        sqlx::query("INSERT INTO latex_core.institution_import_rows (job_id,source_table_or_sheet,row_number,natural_key,payload,action,status) VALUES ($1,'paper_teams',$2,jsonb_build_object('external_team_key',$3),jsonb_build_object('external_team_key',$3),'MATERIALIZE','APPLIED')")
            .bind(job).bind(row_number).bind(key).execute(pool).await.unwrap();
    }
    let plans = repository.pending_team_plans(job).await.unwrap();
    let valid = plans
        .iter()
        .find(|plan| plan.external_team_key == "EXT-VALID")
        .unwrap();
    assert!(valid.unresolved.is_empty());
    assert_eq!(
        valid.writer_user_ids,
        vec![*writer_b.as_uuid(), *writer_a.as_uuid()]
    );
    assert_eq!(valid.leader_user_id, Some(*writer_a.as_uuid()));
    assert_eq!(valid.mentor_user_ids, vec![*mentor.as_uuid()]);
    let blocked = plans
        .iter()
        .find(|plan| plan.external_team_key == "EXT-BLOCKED")
        .unwrap();
    assert!(
        blocked
            .unresolved
            .iter()
            .any(|reason| reason.starts_with("UNRESOLVED_WRITER"))
    );

    let resolution = repository
        .resolve_default_template_for_writers(&[writer_b, writer_a])
        .await
        .unwrap();
    let metadata = TeamTemplateResolutionInput {
        dominant_programme_code: resolution.dominant_programme_code.clone(),
        resolution_method: resolution.resolution_method.clone(),
        external_team_key: Some("EXT-VALID".into()),
        source_import_job_id: Some(job),
    };
    let seed = TemplateSeedFile {
        path: LogicalPath::parse("main.tex").unwrap(),
        blob_hash: BlobHash::digest(b"template"),
        size_bytes: 8,
        policy: V2FilePolicy::Editable,
    };
    let (team, _) = v2
        .create_template_paper_team(
            actor,
            TenantId::from_uuid(tenant),
            WorkspaceId::new(),
            "Imported valid",
            writer_a,
            &[writer_b, writer_a],
            &[mentor],
            template_id,
            &"b".repeat(64),
            &LogicalPath::parse("main.tex").unwrap(),
            &[seed.clone()],
            Some(&metadata),
        )
        .await
        .unwrap();
    let members: Vec<(Uuid, Option<i32>, bool)> = sqlx::query_as("SELECT user_id,writer_order,is_leader FROM latex_core.paper_team_members WHERE paper_team_id=$1 ORDER BY writer_order NULLS LAST")
        .bind(team.id).fetch_all(pool).await.unwrap();
    assert_eq!(members[0], (*writer_b.as_uuid(), Some(1), false));
    assert_eq!(members[1], (*writer_a.as_uuid(), Some(2), true));
    assert_eq!(members[2], (*mentor.as_uuid(), None, false));
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_teams")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(
        v2.create_template_paper_team(
            actor,
            TenantId::from_uuid(tenant),
            WorkspaceId::new(),
            "Must roll back",
            writer_a,
            &[writer_b, writer_a],
            &[mentor],
            template_id,
            &"c".repeat(64),
            &LogicalPath::parse("main.tex").unwrap(),
            &[seed],
            Some(&metadata),
        )
        .await
        .is_err()
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_teams")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.external_paper_team_links WHERE external_team_key='EXT-VALID'").fetch_one(pool).await.unwrap();
    assert_eq!(links, 1);
    let (writer_c, _) = insert_user(pool, "team-c@example.edu", "writer").await;
    sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ('TEAM-C','team-c@example.edu','CSE')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.student_user_links (reg_no,user_id,match_method,status,linked_at) VALUES ('TEAM-C',$1,'TEST','LINKED',now())")
        .bind(writer_c.as_uuid()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.paper_assignment_students (external_team_key,student_reg_no,writer_order,is_leader) VALUES ('EXT-VALID','TEAM-C',3,FALSE)")
        .execute(pool).await.unwrap();
    sqlx::query("UPDATE vcap.paper_assignment_groups SET team_name='Imported merged' WHERE external_team_key='EXT-VALID'")
        .execute(pool).await.unwrap();
    let merge_plan = repository
        .pending_team_plans(job)
        .await
        .unwrap()
        .into_iter()
        .find(|plan| plan.external_team_key == "EXT-VALID")
        .unwrap();
    assert_eq!(merge_plan.existing_paper_team_id, Some(team.id));
    repository
        .merge_existing_team(actor, &merge_plan)
        .await
        .unwrap();
    let merged_members: Vec<(Uuid, Option<i32>, bool)> = sqlx::query_as("SELECT user_id,writer_order,is_leader FROM latex_core.paper_team_members WHERE paper_team_id=$1 ORDER BY writer_order NULLS LAST")
        .bind(team.id).fetch_all(pool).await.unwrap();
    assert_eq!(merged_members[0], (*writer_b.as_uuid(), Some(1), false));
    assert_eq!(merged_members[1], (*writer_a.as_uuid(), Some(2), true));
    assert_eq!(merged_members[2], (*writer_c.as_uuid(), Some(3), false));
    assert_eq!(merged_members[3], (*mentor.as_uuid(), None, false));
    let pinned_after_merge: Uuid = sqlx::query_scalar("SELECT selected_template_id FROM latex_core.paper_template_resolutions WHERE paper_team_id=$1").bind(team.id).fetch_one(pool).await.unwrap();
    assert_eq!(pinned_after_merge, template_id);
    repository
        .mark_team_unresolved(job, "EXT-BLOCKED", &["UNRESOLVED_WRITER".into()])
        .await
        .unwrap();
    let imported = repository
        .paginated_paper_teams(&PaperTeamPageFilter {
            source: Some("imported".into()),
            ..PaperTeamPageFilter::default()
        })
        .await
        .unwrap();
    assert!(
        imported
            .items
            .iter()
            .any(|item| item["external_team_key"] == "EXT-VALID")
    );
    let unresolved_page = repository
        .paginated_paper_teams(&PaperTeamPageFilter {
            unresolved: Some(true),
            ..PaperTeamPageFilter::default()
        })
        .await
        .unwrap();
    assert!(
        unresolved_page
            .items
            .iter()
            .any(|item| item["external_team_key"] == "EXT-BLOCKED" && item["unresolved"] == true)
    );
}

async fn import_modes_and_ten_thousand_rows(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
) {
    let validate = b"reg_no,name,email,programme_code\nVALIDATE-NO-MUTATION,Only Validate,validate@example.edu,SCALE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::ValidateOnly,
            validate,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(job.status, "VALIDATED");
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM vcap.students WHERE reg_no='VALIDATE-NO-MUTATION')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!exists);

    let bad_fk = b"reg_no,name,email,programme_code\nBAD-FK,Bad,bad@example.edu,DOES_NOT_EXIST\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            bad_fk,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(job.status, "FAILED");
    assert_eq!(job.error_rows, 1);

    let mut csv = String::from("reg_no,name,email,programme_code\n");
    for index in 0..10_000 {
        writeln!(
            csv,
            "SCALE{index:05},Student {index},scale{index}@example.edu,SCALE"
        )
        .unwrap();
    }
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            csv.as_bytes(),
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(job.total_rows, 10_000);
    let applied = repository.apply_import(job.id, actor).await.unwrap();
    assert_eq!(applied.status, "APPLIED");
    assert_eq!(applied.inserted_rows, 10_000);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vcap.students WHERE reg_no LIKE 'SCALE%'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 10_000);

    let merge = b"reg_no,name,email,programme_code\nSCALE00000,Updated,scale0@example.edu,SCALE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::Merge,
            merge,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    repository.apply_import(job.id, actor).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vcap.students WHERE reg_no LIKE 'SCALE%'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 10_000);
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM vcap.students WHERE reg_no='SCALE00000'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("Updated"));

    let add_only =
        b"reg_no,name,email,programme_code\nSCALE00000,Must Not Replace,scale0@example.edu,SCALE\n";
    let job = repository
        .validate_upload(
            actor,
            "students.csv",
            None,
            ImportMode::AddOnly,
            add_only,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    let applied = repository.apply_import(job.id, actor).await.unwrap();
    assert_eq!(applied.skipped_rows, 1);
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM vcap.students WHERE reg_no='SCALE00000'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("Updated"));
}

async fn insert_user(pool: &PgPool, email: &str, role: &str) -> (UserId, Uuid) {
    let tenant = Uuid::new_v4();
    let user = UserId::new();
    sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
        .bind(user.as_uuid())
        .bind(tenant)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO latex_core.user_credentials (user_id,email,password_hash) VALUES ($1,$2,'test-hash')").bind(user.as_uuid()).bind(email).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,$2)")
        .bind(user.as_uuid())
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    (user, tenant)
}

async fn insert_template(pool: &PgPool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO latex_core.templates (id,name,main_file) VALUES ($1,$2,'main.tex')")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    id
}
