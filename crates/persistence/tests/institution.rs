#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "isolated PostgreSQL integration fixture"
)]

use core_types::{BlobHash, LogicalPath, TenantId, UserId, WorkspaceId};
use persistence::{
    Database, DatabaseConfig, ImportLimits, ImportMode, InstitutionBatchUpload,
    InstitutionOperation, InstitutionPageFilter, InstitutionRepository, MailOutboxConfig,
    MailSecretCipher, PaperTeamPageFilter, TeamTemplateResolutionInput, TemplateSeedFile,
    V2FilePolicy, V2Repository,
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
    let mail = MailOutboxConfig::new(
        MailSecretCipher::from_base64("BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=")
            .expect("test mail key"),
        Duration::from_secs(72 * 60 * 60),
    )
    .expect("test mail configuration");
    let repository = InstitutionRepository::new(database.clone()).with_mail(mail);
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
    automatic_account_provisioning_is_one_time_and_role_safe(&pool, &repository, actor).await;
    bulk_hundred_accounts_queue_exactly_once(&pool, &repository, actor).await;
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
    batch_dependency_edit_and_delete_contract(&pool, &repository, actor).await;
    legacy_history_cleanup_preserves_canonical_data(&pool, actor).await;

    pool.close().await;
    database.close().await;
}

async fn bulk_hundred_accounts_queue_exactly_once(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
) {
    let suffix = Uuid::new_v4().simple().to_string();
    let mut csv = String::from("reg_no,name,email,programme_code\n");
    for index in 0..100 {
        writeln!(
            csv,
            "MAIL-{suffix}-{index},Student {index},mail-{suffix}-{index}@example.edu,CSE"
        )
        .unwrap();
    }
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Add,
            &[InstitutionBatchUpload {
                filename: "students.csv".into(),
                target_table: None,
                bytes: csv.into_bytes(),
            }],
            ImportLimits::default(),
        )
        .await
        .unwrap();
    let batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    let applied = repository.apply_batch(batch_id, actor).await.unwrap();
    assert_eq!(applied["account_provisioning"]["created"], 100);
    assert_eq!(
        applied["account_provisioning"]["credential_emails_queued"],
        100
    );
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.email_outbox WHERE recipient_email LIKE $1",
    )
    .bind(format!("mail-{suffix}-%@example.edu"))
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(queued, 100);
    let repeated = repository.apply_batch(batch_id, actor).await.unwrap();
    assert_eq!(repeated["account_provisioning"]["created"], 0);
    let still_queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.email_outbox WHERE recipient_email LIKE $1",
    )
    .bind(format!("mail-{suffix}-%@example.edu"))
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(still_queued, 100);
}

async fn automatic_account_provisioning_is_one_time_and_role_safe(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
) {
    let suffix = Uuid::new_v4().simple().to_string();
    let new_student_email = format!("newstudent-{suffix}@example.edu");
    let mentor_email = format!("newmentor-{suffix}@example.edu");
    let unassigned_email = format!("unassigned-{suffix}@example.edu");
    let institutional_admin_email = format!("vcap-admin-{suffix}@example.edu");
    let reused_email = format!("existing-{suffix}@example.edu");
    let (existing_writer, _) = insert_user(pool, &reused_email, "writer").await;
    let existing_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM latex_core.user_credentials WHERE user_id=$1",
    )
    .bind(existing_writer.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    let team_key = format!("AUTO-{suffix}");
    let student_reg = format!("NEW-{suffix}");
    let reused_reg = format!("REUSED-{suffix}");
    let mentor_id = format!("MENTOR-{suffix}");
    let unassigned_id = format!("UNASSIGNED-{suffix}");
    let admin_id = format!("ADMIN-{suffix}");
    let uploads = vec![
        InstitutionBatchUpload {
            filename: "students.csv".into(),
            target_table: None,
            bytes: format!(
                "reg_no,name,email,programme_code\n{student_reg},New Student,{new_student_email},CSE\n{reused_reg},Existing Student,{reused_email},CSE\n"
            )
            .into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "faculty.csv".into(),
            target_table: None,
            bytes: format!(
                "faculty_id,name,email\n{mentor_id},New Mentor,{mentor_email}\n{unassigned_id},Unassigned Faculty,{unassigned_email}\n"
            )
            .into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "admins.csv".into(),
            target_table: None,
            bytes: format!("admin_id,email,name,pfp\n{admin_id},{institutional_admin_email},VCAP Admin,\n").into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "paper_teams.csv".into(),
            target_table: None,
            bytes: format!("external_team_key,team_name,academic_year,semester,status\n{team_key},Automatic Team,2026,1,ACTIVE\n").into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "paper_team_writers.csv".into(),
            target_table: None,
            bytes: format!("external_team_key,student_reg_no,writer_order,is_leader\n{team_key},{student_reg},1,true\n").into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "paper_team_mentors.csv".into(),
            target_table: None,
            bytes: format!("external_team_key,faculty_id\n{team_key},{mentor_id}\n").into_bytes(),
        },
    ];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Add,
            &uploads,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["status"], "VALIDATED");
    let batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    let applied = repository.apply_batch(batch_id, actor).await.unwrap();
    let accounts = &applied["account_provisioning"];
    assert_eq!(accounts["created"], 2);
    assert_eq!(accounts["credential_emails_queued"], 2);
    assert_eq!(accounts["reused"], 1);
    assert_eq!(accounts["needs_attention"], 0);
    let credentials = accounts["credentials"].as_array().unwrap();
    assert_eq!(credentials.len(), 2);
    for credential in credentials {
        let password = credential["temporary_password"].as_str().unwrap();
        assert_eq!(password.len(), 8);
        assert!(password.chars().any(|value| value.is_ascii_uppercase()));
        assert!(password.chars().any(|value| value.is_ascii_lowercase()));
        assert!(password.chars().any(|value| value.is_ascii_digit()));
        let stored: (String, String, bool) = sqlx::query_as(
            "SELECT role.role,credentials.password_hash,credentials.must_change_password \
             FROM latex_core.user_credentials credentials JOIN latex_core.global_user_roles role USING(user_id) \
             WHERE credentials.email=$1",
        )
        .bind(credential["email"].as_str().unwrap())
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(stored.1.starts_with("$argon2"));
        assert!(!stored.1.contains(password));
        assert!(stored.2);
        assert_eq!(
            stored.0,
            if credential["credential_role"] == "student" {
                "writer"
            } else {
                "mentor"
            }
        );
        let leaked_to_audit: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.audit_events WHERE metadata::text LIKE '%' || $1 || '%')",
        )
        .bind(password)
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(!leaked_to_audit);
        let outbox: (String, i32, bool, bool) = sqlx::query_as(
            "SELECT status,attempts,secret_ciphertext IS NOT NULL,encode(secret_ciphertext,'escape') LIKE '%' || $2 || '%' \
             FROM latex_core.email_outbox WHERE recipient_email=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(credential["email"].as_str().unwrap())
        .bind(password)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(outbox.0, "PENDING");
        assert_eq!(outbox.1, 0);
        assert!(outbox.2);
        assert!(!outbox.3);
    }
    let unchanged_hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM latex_core.user_credentials WHERE user_id=$1",
    )
    .bind(existing_writer.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(unchanged_hash, existing_hash);
    for email in [&unassigned_email, &institutional_admin_email] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.user_credentials WHERE lower(email)=lower($1))",
        )
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(!exists);
    }
    let primary_job = repository.batch_primary_job(batch_id).await.unwrap();
    let plans = repository.pending_team_plans(primary_job).await.unwrap();
    let plan = plans
        .iter()
        .find(|value| value.external_team_key == team_key)
        .unwrap();
    assert!(plan.unresolved.is_empty());
    assert_eq!(plan.writer_user_ids.len(), 1);
    assert_eq!(plan.mentor_user_ids.len(), 1);

    let repeated = repository.apply_batch(batch_id, actor).await.unwrap();
    assert_eq!(repeated["account_provisioning"]["created"], 0);
    assert_eq!(
        repeated["account_provisioning"]["credential_emails_queued"],
        0
    );
    assert!(
        repeated["account_provisioning"]["credentials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

async fn legacy_history_cleanup_preserves_canonical_data(pool: &PgPool, actor: UserId) {
    let job_id = Uuid::new_v4();
    let canonical_before: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM vcap.students),\
                (SELECT count(*) FROM latex_core.users),\
                (SELECT count(*) FROM latex_core.paper_teams)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO latex_core.institution_import_jobs \
         (id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id,status) \
         VALUES ($1,'departments','VALIDATE_ONLY','legacy-test.csv',$2,'CSV',$3,'VALIDATED')",
    )
    .bind(job_id)
    .bind("d".repeat(64))
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO latex_core.institution_import_rows \
         (job_id,source_table_or_sheet,row_number,natural_key,payload,action,status) \
         VALUES ($1,'departments',2,jsonb_build_object('department_id',$2),\
         jsonb_build_object('department_id',$2),'INSERT','VALID')",
    )
    .bind(job_id)
    .bind(Uuid::new_v4().to_string())
    .execute(pool)
    .await
    .unwrap();

    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM latex_core.institution_import_rows WHERE job_id=$1")
        .bind(job_id)
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query("DELETE FROM latex_core.institution_import_jobs WHERE id=$1 AND batch_id IS NULL")
        .bind(job_id)
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.commit().await.unwrap();

    let canonical_after: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM vcap.students),\
                (SELECT count(*) FROM latex_core.users),\
                (SELECT count(*) FROM latex_core.paper_teams)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(canonical_after, canonical_before);
    let history_remaining: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM latex_core.institution_import_jobs WHERE id=$1) +\
                (SELECT count(*) FROM latex_core.institution_import_rows WHERE job_id=$1)",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(history_remaining, 0);
}

async fn batch_dependency_edit_and_delete_contract(
    pool: &PgPool,
    repository: &InstitutionRepository,
    actor: UserId,
) {
    let department = Uuid::new_v4();
    let uploads = vec![
        InstitutionBatchUpload {
            filename: "VIT_students_2026.csv".into(),
            target_table: None,
            bytes: b"reg_no,name,email,programme_code\nBATCH-STUDENT,Random Order Student,batch-student@example.edu,BATCH-PROGRAMME\n".to_vec(),
        },
        InstitutionBatchUpload {
            filename: "programmes_2026.csv".into(),
            target_table: None,
            bytes: b"programme_code,hod_id\nBATCH-PROGRAMME,BATCH-FACULTY\n".to_vec(),
        },
        InstitutionBatchUpload {
            filename: "faculty.csv".into(),
            target_table: None,
            bytes: format!("faculty_id,name,email,dept_id,honorific,designation,status\nBATCH-FACULTY,Batch Faculty,batch-faculty@example.edu,{department},Dr,Guide,ACTIVE\n").into_bytes(),
        },
        InstitutionBatchUpload {
            filename: "departments.csv".into(),
            target_table: None,
            bytes: format!("department_id\n{department}\n").into_bytes(),
        },
    ];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Add,
            &uploads,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["status"], "VALIDATED");
    assert_eq!(detail["batch"]["added_rows"], 4);
    let batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    repository.apply_batch(batch_id, actor).await.unwrap();
    let student: (String, Option<String>) = sqlx::query_as(
        "SELECT name,programme_code FROM vcap.students WHERE reg_no='BATCH-STUDENT'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(student.1.as_deref(), Some("BATCH-PROGRAMME"));

    let edit = [InstitutionBatchUpload {
        filename: "students_edit.csv".into(),
        target_table: None,
        bytes: b"reg_no,name,email,programme_code\nBATCH-STUDENT,Edited Student,edited-batch@example.edu,BATCH-PROGRAMME\n".to_vec(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Edit,
            &edit,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["edited_rows"], 1);
    let batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    repository.apply_batch(batch_id, actor).await.unwrap();
    let edited: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT name,email FROM vcap.students WHERE reg_no='BATCH-STUDENT'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(edited.0.as_deref(), Some("Edited Student"));
    assert_eq!(edited.1.as_deref(), Some("edited-batch@example.edu"));

    let missing = [InstitutionBatchUpload {
        filename: "students.csv".into(),
        target_table: None,
        bytes: b"reg_no,name\nDOES-NOT-EXIST,Nobody\n".to_vec(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Edit,
            &missing,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["status"], "FAILED");
    assert_eq!(detail["issues"][0]["code"], "NOT_FOUND");

    let delete = [InstitutionBatchUpload {
        filename: "students.csv".into(),
        target_table: None,
        bytes: b"reg_no\nBATCH-STUDENT\n".to_vec(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Delete,
            &delete,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["status"], "VALIDATED");
    let batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    repository.apply_batch(batch_id, actor).await.unwrap();
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM vcap.students WHERE reg_no='BATCH-STUDENT')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!exists);

    let (preserved_user, _) = insert_user(pool, "delete-safe-writer@example.edu", "writer").await;
    let linked_add = [InstitutionBatchUpload {
        filename: "students.csv".into(),
        target_table: None,
        bytes: b"reg_no,name,email,programme_code\nDELETE-SAFE-STUDENT,Delete Safe,delete-safe-writer@example.edu,BATCH-PROGRAMME\n".to_vec(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Add,
            &linked_add,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    let add_batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    repository.apply_batch(add_batch_id, actor).await.unwrap();
    let linked_delete = [InstitutionBatchUpload {
        filename: "students.csv".into(),
        target_table: None,
        bytes: b"reg_no\nDELETE-SAFE-STUDENT\n".to_vec(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Delete,
            &linked_delete,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    let delete_batch_id = Uuid::parse_str(detail["batch"]["id"].as_str().unwrap()).unwrap();
    repository
        .apply_batch(delete_batch_id, actor)
        .await
        .unwrap();
    let user_preserved: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.users WHERE id=$1)")
            .bind(preserved_user.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(user_preserved);

    let materialized_student: String = sqlx::query_scalar(
        "SELECT assignment.student_reg_no FROM vcap.paper_assignment_students assignment JOIN latex_core.external_paper_team_links link USING(external_team_key) LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let blocked_delete = [InstitutionBatchUpload {
        filename: "students.csv".into(),
        target_table: None,
        bytes: format!("reg_no\n{materialized_student}\n").into_bytes(),
    }];
    let detail = repository
        .validate_batch(
            actor,
            InstitutionOperation::Delete,
            &blocked_delete,
            ImportLimits::default(),
        )
        .await
        .unwrap();
    assert_eq!(detail["batch"]["status"], "FAILED");
    assert_eq!(detail["issues"][0]["code"], "DELETE_BLOCKED_DEPENDENCY");
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

    let (api_writer, _) = insert_user(pool, "manual-api@example.edu", "writer").await;
    sqlx::query("INSERT INTO vcap.students (reg_no,email,programme_code) VALUES ('MANUAL-API','external@example.edu','CSE')")
        .execute(pool).await.unwrap();
    repository
        .manual_link_identity(actor, "STUDENT", "MANUAL-API", *api_writer.as_uuid())
        .await
        .unwrap();
    let state: (String, String) = sqlx::query_as(
        "SELECT status,match_method FROM vcap.student_user_links WHERE reg_no='MANUAL-API'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(state, ("LINKED".into(), "MANUAL".into()));
    sqlx::query("INSERT INTO vcap.faculty (faculty_id,email) VALUES ('MANUAL-BAD','external-faculty@example.edu')").execute(pool).await.unwrap();
    assert!(
        repository
            .manual_link_identity(actor, "FACULTY", "MANUAL-BAD", *api_writer.as_uuid())
            .await
            .is_err()
    );

    let (informational_admin, _) =
        insert_user(pool, "informational-admin@example.edu", "writer").await;
    sqlx::query("INSERT INTO vcap.admins (admin_id,email) VALUES ('MANUAL-ADMIN','institution-admin@example.edu')").execute(pool).await.unwrap();
    repository
        .manual_link_identity(
            actor,
            "ADMIN",
            "MANUAL-ADMIN",
            *informational_admin.as_uuid(),
        )
        .await
        .unwrap();
    let role: String =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(informational_admin.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(role, "writer");
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
    assert!(
        repository
            .unlink_identity(actor, "STUDENT", "TEAM-A")
            .await
            .is_err()
    );
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
    let (mentor_two, _) = insert_user(pool, "team-mentor-two@example.edu", "mentor").await;
    sqlx::query("INSERT INTO vcap.faculty (faculty_id,email) VALUES ('TEAM-MENTOR-TWO','team-mentor-two@example.edu')")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO vcap.faculty_user_links (faculty_id,user_id,match_method,status,linked_at) VALUES ('TEAM-MENTOR-TWO',$1,'TEST','LINKED',now())")
        .bind(mentor_two.as_uuid()).execute(pool).await.unwrap();
    let workspace_before: Uuid =
        sqlx::query_scalar("SELECT workspace_id FROM latex_core.paper_teams WHERE id=$1")
            .bind(team.id)
            .fetch_one(pool)
            .await
            .unwrap();
    repository
        .update_paper_team(
            actor,
            team.id,
            "Admin edited imported Team",
            &[*writer_c.as_uuid(), *writer_b.as_uuid()],
            *writer_b.as_uuid(),
            &[*mentor_two.as_uuid()],
        )
        .await
        .unwrap();
    let runtime_members: Vec<(Uuid, Option<i32>, bool)> = sqlx::query_as(
        "SELECT user_id,writer_order,is_leader FROM latex_core.paper_team_members \
         WHERE paper_team_id=$1 ORDER BY writer_order NULLS LAST",
    )
    .bind(team.id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(runtime_members[0], (*writer_c.as_uuid(), Some(1), false));
    assert_eq!(runtime_members[1], (*writer_b.as_uuid(), Some(2), true));
    assert_eq!(runtime_members[2], (*mentor_two.as_uuid(), None, false));
    let assignment_writers: Vec<(String, i32, bool)> = sqlx::query_as(
        "SELECT student_reg_no,writer_order,is_leader FROM vcap.paper_assignment_students \
         WHERE external_team_key='EXT-VALID' ORDER BY writer_order",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        assignment_writers,
        vec![("TEAM-C".into(), 1, false), ("TEAM-B".into(), 2, true)]
    );
    let assignment_mentors: Vec<String> = sqlx::query_scalar(
        "SELECT faculty_id FROM vcap.paper_assignment_mentors WHERE external_team_key='EXT-VALID'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(assignment_mentors, vec!["TEAM-MENTOR-TWO"]);
    let edited_team: (String, Uuid) =
        sqlx::query_as("SELECT name,workspace_id FROM latex_core.paper_teams WHERE id=$1")
            .bind(team.id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        edited_team,
        ("Admin edited imported Team".into(), workspace_before)
    );
    let pinned_after_edit: Uuid = sqlx::query_scalar(
        "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
    )
    .bind(team.id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(pinned_after_edit, template_id);
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
        writeln!(csv, "SCALE{index:05},Student {index},,SCALE").unwrap();
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
    assert_eq!(applied.job.status, "APPLIED");
    assert_eq!(applied.job.inserted_rows, 10_000);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vcap.students WHERE reg_no LIKE 'SCALE%'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 10_000);
    let student_page = repository
        .paginated_students(&InstitutionPageFilter {
            limit: 25,
            page: 1,
            programme_code: Some("SCALE".into()),
            ..InstitutionPageFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(student_page.total, 10_000);
    assert_eq!(student_page.items.len(), 25);
    assert!(student_page.has_more);

    sqlx::query(
        r"WITH created AS (
             INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id)
             SELECT gen_random_uuid(),(SELECT tenant_id FROM latex_core.users WHERE id=$1),$1
             FROM generate_series(1,1000) RETURNING id
           ), numbered AS (
             SELECT id,row_number() OVER (ORDER BY id) AS number FROM created
           ) INSERT INTO latex_core.paper_teams (id,workspace_id,name,created_by_user_id)
             SELECT gen_random_uuid(),id,'Scale Team ' || number,$1 FROM numbered",
    )
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .unwrap();
    let team_page = repository
        .paginated_paper_teams(&PaperTeamPageFilter {
            limit: 25,
            page: 1,
            search: Some("Scale Team".into()),
            ..PaperTeamPageFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(team_page.total, 1_000);
    assert_eq!(team_page.items.len(), 25);
    assert!(team_page.has_more);

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
    assert_eq!(applied.job.skipped_rows, 1);
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
    sqlx::query("INSERT INTO latex_core.template_files (template_id,path,blob_hash,size_bytes) VALUES ($1,'main.tex',$2,8)")
        .bind(id).bind("a".repeat(64)).execute(pool).await.unwrap();
    id
}
